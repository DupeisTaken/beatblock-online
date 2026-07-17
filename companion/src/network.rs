use crate::model::{Envelope, ParticipantRole, RenderSample, PROTOCOL_VERSION};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hmac::{Hmac, Mac};
use quinn::{crypto::rustls::QuicClientConfig, Connection, Endpoint, RecvStream, SendStream};
use rcgen::generate_simple_self_signed;
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    DigitallySignedStruct, Error as TlsError, SignatureScheme,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, RwLock, Semaphore};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;
const AUTH_IDENTITY: &[u8] = b"beatblock-together-room-v2";
const MAX_CONTROL_FRAME: usize = 1_048_576;
const AUTH_FAILURE_WINDOW: Duration = Duration::from_secs(60);
const MAX_PASSWORD_FAILURE_IPS: usize = 4_096;
const MAX_PASSWORD_FAILURES_PER_IP: usize = 5;
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PENDING_AUTHENTICATIONS: usize = 64;

#[derive(Debug, Clone)]
pub enum NetworkEvent {
    Authenticated {
        session_id: String,
        display_name: String,
        role: ParticipantRole,
        remote_address: SocketAddr,
        hosting: bool,
    },
    Envelope {
        session_id: String,
        envelope: Envelope,
    },
    RenderSample {
        session_id: String,
        sample: RenderSample,
    },
    Disconnected {
        session_id: String,
        reason: String,
    },
    Diagnostic(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthHello {
    version: u8,
    display_name: String,
    role: ParticipantRole,
    spake_message: String,
    #[serde(default)]
    resume_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthChallenge {
    version: u8,
    spake_message: String,
    nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthProof {
    proof: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthWelcome {
    accepted: bool,
    session_id: String,
    message: String,
    #[serde(default)]
    resume_token: Option<String>,
}

#[derive(Clone)]
pub struct NetworkHub {
    events: mpsc::Sender<NetworkEvent>,
    peers: Arc<RwLock<HashMap<String, mpsc::Sender<Envelope>>>>,
    peer_connections: Arc<RwLock<HashMap<String, Connection>>>,
    server_writer: Arc<RwLock<Option<mpsc::Sender<Envelope>>>>,
    server_connection: Arc<RwLock<Option<Connection>>>,
    endpoint: Arc<Mutex<Option<Endpoint>>>,
    password_failures: Arc<RwLock<HashMap<IpAddr, VecDeque<Instant>>>>,
    resume_sessions: Arc<RwLock<HashMap<String, String>>>,
    client_resume_token: Arc<RwLock<Option<String>>>,
}

impl NetworkHub {
    pub fn new(events: mpsc::Sender<NetworkEvent>) -> Self {
        Self {
            events,
            peers: Arc::new(RwLock::new(HashMap::new())),
            peer_connections: Arc::new(RwLock::new(HashMap::new())),
            server_writer: Arc::new(RwLock::new(None)),
            server_connection: Arc::new(RwLock::new(None)),
            endpoint: Arc::new(Mutex::new(None)),
            password_failures: Arc::new(RwLock::new(HashMap::new())),
            resume_sessions: Arc::new(RwLock::new(HashMap::new())),
            client_resume_token: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start_host(&self, port: u16, password: String) -> Result<SocketAddr> {
        self.shutdown().await;
        self.resume_sessions.write().await.clear();
        *self.client_resume_token.write().await = None;
        if password.chars().count() < 4 || password.chars().count() > 128 {
            bail!("room password must contain 4-128 characters");
        }
        let certified = generate_simple_self_signed(vec!["beatblock-together.local".into()])?;
        let cert_der = certified.cert.der().clone();
        let key = PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der());
        let mut server_config = quinn::ServerConfig::with_single_cert(vec![cert_der], key.into())?;
        let transport = Arc::get_mut(&mut server_config.transport)
            .context("QUIC server transport configuration was unexpectedly shared")?;
        transport.max_concurrent_bidi_streams(64u32.into());
        transport.datagram_receive_buffer_size(Some(4 * 1024 * 1024));
        transport.datagram_send_buffer_size(4 * 1024 * 1024);
        let endpoint = Endpoint::server(
            server_config,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port),
        )?;
        let mut address = endpoint.local_addr()?;
        if address.ip().is_unspecified() {
            address.set_ip(local_ip_address::local_ip().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        }
        *self.endpoint.lock().expect("endpoint mutex poisoned") = Some(endpoint.clone());
        let hub = self.clone();
        tokio::spawn(async move {
            // A remote peer that never opens its authentication stream must not
            // retain a task and QUIC connection indefinitely.
            let authentication_slots = Arc::new(Semaphore::new(MAX_PENDING_AUTHENTICATIONS));
            while let Some(incoming) = endpoint.accept().await {
                let Ok(authentication_slot) = authentication_slots.clone().try_acquire_owned()
                else {
                    incoming.refuse();
                    continue;
                };
                let hub = hub.clone();
                let password = password.clone();
                tokio::spawn(async move {
                    let _authentication_slot = authentication_slot;
                    let connected = tokio::time::timeout(AUTH_TIMEOUT, incoming).await;
                    let error = match connected {
                        Ok(Ok(connection)) => {
                            match tokio::time::timeout(
                                AUTH_TIMEOUT,
                                hub.accept_peer(connection.clone(), &password),
                            )
                            .await
                            {
                                Ok(Ok(())) => None,
                                Ok(Err(error)) => Some(error.to_string()),
                                Err(_) => {
                                    connection.close(1u32.into(), b"authentication timed out");
                                    Some("room authentication timed out".into())
                                }
                            }
                        }
                        Ok(Err(error)) => Some(error.to_string()),
                        Err(_) => Some("QUIC connection handshake timed out".into()),
                    };
                    if let Some(error) = error {
                        let _ = hub.events.send(NetworkEvent::Diagnostic(error)).await;
                    }
                });
            }
        });
        Ok(address)
    }

    pub async fn join(
        &self,
        address: SocketAddr,
        password: &str,
        display_name: &str,
        role: ParticipantRole,
    ) -> Result<String> {
        tokio::time::timeout(
            JOIN_TIMEOUT,
            self.join_inner(address, password, display_name, role),
        )
        .await
        .context("timed out while connecting to the room")?
    }

    async fn join_inner(
        &self,
        address: SocketAddr,
        password: &str,
        display_name: &str,
        role: ParticipantRole,
    ) -> Result<String> {
        self.shutdown().await;
        let mut endpoint = Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))?;
        endpoint.set_default_client_config(insecure_client_config()?);
        let connection = endpoint
            .connect(address, "beatblock-together.local")?
            .await
            .context("connect to host QUIC endpoint")?;
        let (mut send, mut recv) = connection.open_bi().await?;
        let (spake, outbound) = Spake2::<Ed25519Group>::start_symmetric(
            &Password::new(password.as_bytes()),
            &Identity::new(AUTH_IDENTITY),
        );
        write_frame(
            &mut send,
            &AuthHello {
                version: PROTOCOL_VERSION,
                display_name: display_name.trim().into(),
                role,
                spake_message: BASE64.encode(outbound),
                resume_token: self.client_resume_token.read().await.clone(),
            },
        )
        .await?;
        let challenge: AuthChallenge = read_frame(&mut recv).await?;
        if challenge.version != PROTOCOL_VERSION {
            bail!("host uses incompatible protocol version");
        }
        let key = spake
            .finish(&BASE64.decode(challenge.spake_message)?)
            .map_err(|_| anyhow::anyhow!("password exchange failed"))?;
        let proof = auth_proof(&key, challenge.nonce.as_bytes(), b"client")?;
        write_frame(&mut send, &AuthProof { proof }).await?;
        let welcome: AuthWelcome = read_frame(&mut recv).await?;
        if !welcome.accepted {
            bail!(welcome.message);
        }
        *self.client_resume_token.write().await = welcome.resume_token.clone();
        let (outgoing_tx, outgoing_rx) = mpsc::channel(4096);
        *self.server_writer.write().await = Some(outgoing_tx);
        *self.server_connection.write().await = Some(connection.clone());
        *self.endpoint.lock().expect("endpoint mutex poisoned") = Some(endpoint);
        self.spawn_writer(send, outgoing_rx);
        self.spawn_reader(welcome.session_id.clone(), recv, connection.clone());
        let _ = self.events.try_send(NetworkEvent::Authenticated {
            session_id: welcome.session_id.clone(),
            display_name: display_name.trim().into(),
            role,
            remote_address: address,
            hosting: false,
        });
        Ok(welcome.session_id)
    }

    pub async fn broadcast(&self, envelope: Envelope) {
        let peer_writers = self
            .peers
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for writer in peer_writers {
            let _ = writer.try_send(envelope.clone());
        }
        if let Some(writer) = self.server_writer.read().await.as_ref() {
            let _ = writer.try_send(envelope);
        }
    }

    pub async fn send_to(&self, session_id: &str, envelope: Envelope) -> Result<()> {
        if let Some(writer) = self.peers.read().await.get(session_id) {
            writer.send(envelope).await?;
            return Ok(());
        }
        if let Some(writer) = self.server_writer.read().await.as_ref() {
            writer.send(envelope).await?;
            return Ok(());
        }
        bail!("network session is not connected")
    }

    pub async fn send_render_sample(&self, sample: &RenderSample) -> Result<()> {
        let bytes = sample.encode().to_vec().into();
        if let Some(connection) = self.server_connection.read().await.as_ref() {
            connection.send_datagram(bytes)?;
            return Ok(());
        }
        bail!("render datagrams are sent by participants, not the host")
    }

    pub async fn shutdown(&self) {
        self.shutdown_with_reason("Runtime stopped").await;
    }

    /// Close every transport with a user-facing reason that the remote room
    /// lifecycle can classify without presenting an intentional close as a
    /// runtime crash.
    pub async fn shutdown_with_reason(&self, reason: &str) {
        if let Some(endpoint) = self
            .endpoint
            .lock()
            .expect("endpoint mutex poisoned")
            .take()
        {
            endpoint.close(0u32.into(), reason.as_bytes());
        }
        self.peers.write().await.clear();
        self.peer_connections.write().await.clear();
        *self.server_writer.write().await = None;
        *self.server_connection.write().await = None;
    }

    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
    }

    pub async fn disconnect_peer(&self, session_id: &str, reason: &str, allow_resume: bool) {
        if !allow_resume {
            self.resume_sessions
                .write()
                .await
                .retain(|_, mapped_session| mapped_session != session_id);
        }
        self.peers.write().await.remove(session_id);
        if let Some(connection) = self.peer_connections.write().await.remove(session_id) {
            connection.close(1u32.into(), reason.as_bytes());
        }
    }

    pub async fn clear_client_resume(&self) {
        *self.client_resume_token.write().await = None;
    }

    async fn accept_peer(&self, connection: Connection, password: &str) -> Result<()> {
        let remote_address = connection.remote_address();
        {
            let mut failures = self.password_failures.write().await;
            if !password_attempt_allowed(&mut failures, remote_address.ip(), Instant::now()) {
                bail!("too many password attempts from {remote_address}; retry later");
            }
        }
        let (mut send, mut recv) = connection.accept_bi().await?;
        let hello: AuthHello = read_frame(&mut recv).await?;
        if hello.version != PROTOCOL_VERSION {
            write_frame(
                &mut send,
                &AuthWelcome {
                    accepted: false,
                    session_id: String::new(),
                    message: "Incompatible protocol version".into(),
                    resume_token: None,
                },
            )
            .await?;
            bail!("incompatible participant protocol");
        }
        let client_message = BASE64.decode(hello.spake_message)?;
        let (spake, outbound) = Spake2::<Ed25519Group>::start_symmetric(
            &Password::new(password.as_bytes()),
            &Identity::new(AUTH_IDENTITY),
        );
        let key = spake
            .finish(&client_message)
            .map_err(|_| anyhow::anyhow!("password exchange failed"))?;
        let nonce_bytes: [u8; 32] = rand::random();
        let nonce = BASE64.encode(nonce_bytes);
        write_frame(
            &mut send,
            &AuthChallenge {
                version: PROTOCOL_VERSION,
                spake_message: BASE64.encode(outbound),
                nonce: nonce.clone(),
            },
        )
        .await?;
        let proof: AuthProof = read_frame(&mut recv).await?;
        let expected = auth_proof(&key, nonce.as_bytes(), b"client")?;
        if !constant_time_eq(proof.proof.as_bytes(), expected.as_bytes()) {
            // Hold the write guard in a named binding so deref coercion reaches
            // the underlying map instead of passing the guard wrapper itself.
            let mut failures = self.password_failures.write().await;
            record_password_failure(&mut failures, remote_address.ip(), Instant::now());
            drop(failures);
            write_frame(
                &mut send,
                &AuthWelcome {
                    accepted: false,
                    session_id: String::new(),
                    message: "Incorrect room password".into(),
                    resume_token: None,
                },
            )
            .await?;
            bail!("room password authentication failed for {remote_address}");
        }
        self.password_failures
            .write()
            .await
            .remove(&remote_address.ip());
        let (session_id, resume_token) = if let Some(token) = hello.resume_token.as_ref() {
            let session = self.resume_sessions.read().await.get(token).cloned();
            if let Some(session) = session {
                if self.peer_connections.read().await.contains_key(&session) {
                    write_frame(
                        &mut send,
                        &AuthWelcome {
                            accepted: false,
                            session_id: String::new(),
                            message: "This room session is already connected".into(),
                            resume_token: None,
                        },
                    )
                    .await?;
                    bail!("duplicate resume attempt for {session}");
                }
                (session, token.clone())
            } else {
                (Uuid::new_v4().to_string(), random_resume_token())
            }
        } else {
            (Uuid::new_v4().to_string(), random_resume_token())
        };
        self.resume_sessions
            .write()
            .await
            .insert(resume_token.clone(), session_id.clone());
        write_frame(
            &mut send,
            &AuthWelcome {
                accepted: true,
                session_id: session_id.clone(),
                message: "Authenticated; awaiting room admission policy".into(),
                resume_token: Some(resume_token),
            },
        )
        .await?;
        let (outgoing_tx, outgoing_rx) = mpsc::channel(4096);
        self.peers
            .write()
            .await
            .insert(session_id.clone(), outgoing_tx);
        self.peer_connections
            .write()
            .await
            .insert(session_id.clone(), connection.clone());
        self.spawn_writer(send, outgoing_rx);
        self.spawn_reader(session_id.clone(), recv, connection);
        let _ = self
            .events
            .send(NetworkEvent::Authenticated {
                session_id,
                display_name: hello.display_name,
                role: hello.role,
                remote_address,
                hosting: true,
            })
            .await;
        Ok(())
    }

    fn spawn_writer(&self, mut send: SendStream, mut outgoing: mpsc::Receiver<Envelope>) {
        let events = self.events.clone();
        tokio::spawn(async move {
            while let Some(envelope) = outgoing.recv().await {
                if let Err(error) = write_frame(&mut send, &envelope).await {
                    let _ = events
                        .send(NetworkEvent::Diagnostic(error.to_string()))
                        .await;
                    break;
                }
            }
            let _ = send.finish();
        });
    }

    fn spawn_reader(&self, session_id: String, mut recv: RecvStream, connection: Connection) {
        let events = self.events.clone();
        let peers = self.peers.clone();
        let peer_connections = self.peer_connections.clone();
        tokio::spawn(async move {
            let reliable_events = events.clone();
            let reliable_session = session_id.clone();
            let reliable = tokio::spawn(async move {
                loop {
                    match read_frame::<Envelope>(&mut recv).await {
                        Ok(envelope) if envelope.version == PROTOCOL_VERSION => {
                            let _ = reliable_events
                                .send(NetworkEvent::Envelope {
                                    session_id: reliable_session.clone(),
                                    envelope,
                                })
                                .await;
                        }
                        Ok(_) => {
                            let _ = reliable_events
                                .send(NetworkEvent::Diagnostic(
                                    "Incompatible control message".into(),
                                ))
                                .await;
                            break;
                        }
                        Err(_) => break,
                    }
                }
            });
            while let Ok(bytes) = connection.read_datagram().await {
                match RenderSample::decode(&bytes) {
                    Ok(sample) => {
                        let _ = events
                            .send(NetworkEvent::RenderSample {
                                session_id: session_id.clone(),
                                sample,
                            })
                            .await;
                    }
                    Err(error) => {
                        let _ = events
                            .send(NetworkEvent::Diagnostic(error.to_string()))
                            .await;
                    }
                }
            }
            reliable.abort();
            peers.write().await.remove(&session_id);
            peer_connections.write().await.remove(&session_id);
            let reason = match connection.close_reason() {
                Some(quinn::ConnectionError::ApplicationClosed(close)) => {
                    String::from_utf8_lossy(&close.reason).into_owned()
                }
                Some(error) => error.to_string(),
                None => "QUIC connection closed".into(),
            };
            let _ = events
                .send(NetworkEvent::Disconnected { session_id, reason })
                .await;
        });
    }
}

async fn write_frame<T: Serialize>(send: &mut SendStream, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_CONTROL_FRAME {
        bail!("control message exceeds 1 MiB");
    }
    send.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    send.write_all(&bytes).await?;
    Ok(())
}

async fn read_frame<T: DeserializeOwned>(recv: &mut RecvStream) -> Result<T> {
    let mut length = [0u8; 4];
    recv.read_exact(&mut length).await?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_CONTROL_FRAME {
        bail!("control message exceeds 1 MiB");
    }
    let mut bytes = vec![0u8; length];
    recv.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn auth_proof(key: &[u8], nonce: &[u8], label: &[u8]) -> Result<String> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    mac.update(AUTH_IDENTITY);
    mac.update(nonce);
    mac.update(label);
    Ok(BASE64.encode(mac.finalize().into_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn random_resume_token() -> String {
    let token: [u8; 32] = rand::random();
    BASE64.encode(token)
}

/// Removes expired authentication state and enforces a hard upper bound on
/// distinct source addresses. An internet-facing host must not let spoofed or
/// rotating addresses grow this map for the lifetime of the process.
fn prune_password_failures(failures: &mut HashMap<IpAddr, VecDeque<Instant>>, now: Instant) {
    failures.retain(|_, attempts| {
        while attempts
            .front()
            .is_some_and(|attempt| now.saturating_duration_since(*attempt) > AUTH_FAILURE_WINDOW)
        {
            attempts.pop_front();
        }
        !attempts.is_empty()
    });
}

fn password_attempt_allowed(
    failures: &mut HashMap<IpAddr, VecDeque<Instant>>,
    address: IpAddr,
    now: Instant,
) -> bool {
    prune_password_failures(failures, now);
    failures
        .get(&address)
        .map_or(failures.len() < MAX_PASSWORD_FAILURE_IPS, |attempts| {
            attempts.len() < MAX_PASSWORD_FAILURES_PER_IP
        })
}

fn record_password_failure(
    failures: &mut HashMap<IpAddr, VecDeque<Instant>>,
    address: IpAddr,
    now: Instant,
) {
    prune_password_failures(failures, now);
    if let Some(attempts) = failures.get_mut(&address) {
        if attempts.len() < MAX_PASSWORD_FAILURES_PER_IP {
            attempts.push_back(now);
        }
    } else if failures.len() < MAX_PASSWORD_FAILURE_IPS {
        failures.insert(address, VecDeque::from([now]));
    }
}

fn insecure_client_config() -> Result<quinn::ClientConfig> {
    let rustls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();
    Ok(quinn::ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(rustls_config)?,
    )))
}

#[derive(Debug)]
struct SkipServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self(Arc::new(rustls::crypto::ring::default_provider())))
    }
}

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

pub fn certificate_fingerprint(certificate: &[u8]) -> String {
    let digest = Sha256::digest(certificate);
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_failure_tracking_expires_and_stays_bounded() {
        let now = Instant::now();
        let stale = now - AUTH_FAILURE_WINDOW - Duration::from_secs(1);
        let mut failures = HashMap::new();
        failures.insert(IpAddr::from([10, 0, 0, 1]), VecDeque::from([stale]));
        failures.insert(IpAddr::from([10, 0, 0, 2]), VecDeque::from([now]));
        prune_password_failures(&mut failures, now);
        assert!(!failures.contains_key(&IpAddr::from([10, 0, 0, 1])));
        assert!(failures.contains_key(&IpAddr::from([10, 0, 0, 2])));

        failures.clear();
        for index in 0..MAX_PASSWORD_FAILURE_IPS {
            let address = IpAddr::V6(std::net::Ipv6Addr::from(index as u128 + 1));
            record_password_failure(&mut failures, address, now);
        }
        assert_eq!(failures.len(), MAX_PASSWORD_FAILURE_IPS);
        let overflow = IpAddr::from([192, 0, 2, 1]);
        record_password_failure(&mut failures, overflow, now);
        assert_eq!(failures.len(), MAX_PASSWORD_FAILURE_IPS);
        assert!(!password_attempt_allowed(&mut failures, overflow, now));

        let tracked = IpAddr::V6(std::net::Ipv6Addr::from(1));
        for _ in 1..MAX_PASSWORD_FAILURES_PER_IP {
            record_password_failure(&mut failures, tracked, now);
        }
        assert!(!password_attempt_allowed(&mut failures, tracked, now));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stalled_authentication_is_closed_by_the_host() {
        let (host_events, _events) = mpsc::channel(8);
        let host = NetworkHub::new(host_events);
        let address = host.start_host(0, "correct horse".into()).await.unwrap();

        let mut endpoint =
            Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)).unwrap();
        endpoint.set_default_client_config(insecure_client_config().unwrap());
        let connection = endpoint
            .connect(address, "beatblock-together.local")
            .unwrap()
            .await
            .unwrap();

        // Deliberately never open the authentication stream. The host should
        // reclaim the connection instead of retaining this peer indefinitely.
        tokio::time::timeout(AUTH_TIMEOUT + Duration::from_secs(3), connection.closed())
            .await
            .expect("host did not close a stalled authentication");
        host.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn authenticates_password_and_exchanges_control_and_datagrams() {
        let (host_events_tx, mut host_events) = mpsc::channel(32);
        let host = NetworkHub::new(host_events_tx);
        let address = host.start_host(0, "correct horse".into()).await.unwrap();
        let (client_events_tx, mut client_events) = mpsc::channel(32);
        let client = NetworkHub::new(client_events_tx);
        let session = client
            .join(address, "correct horse", "Player", ParticipantRole::Player)
            .await
            .unwrap();
        assert!(!session.is_empty());
        let authenticated =
            tokio::time::timeout(std::time::Duration::from_secs(3), host_events.recv())
                .await
                .unwrap()
                .unwrap();
        assert!(matches!(
            authenticated,
            NetworkEvent::Authenticated { hosting: true, .. }
        ));
        client
            .broadcast(Envelope::new(
                "client.ready",
                1,
                serde_json::json!({"ready":true}),
            ))
            .await;
        let control = tokio::time::timeout(std::time::Duration::from_secs(3), host_events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(control, NetworkEvent::Envelope { .. }));
        let _ = client_events.recv().await;

        client.shutdown().await;
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(3), host_events.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(event, NetworkEvent::Disconnected { .. }) {
                break;
            }
        }
        let resumed = client
            .join(address, "correct horse", "Player", ParticipantRole::Player)
            .await
            .unwrap();
        assert_eq!(resumed, session, "reconnect must preserve room identity");
        loop {
            let event =
                tokio::time::timeout(std::time::Duration::from_secs(3), client_events.recv())
                    .await
                    .unwrap()
                    .unwrap();
            if matches!(event, NetworkEvent::Authenticated { hosting: false, .. }) {
                break;
            }
        }
        host.disconnect_peer(&session, "Removed from the room by the host", false)
            .await;
        let close_reason = loop {
            let event =
                tokio::time::timeout(std::time::Duration::from_secs(3), client_events.recv())
                    .await
                    .unwrap()
                    .unwrap();
            if let NetworkEvent::Disconnected { reason, .. } = event {
                break reason;
            }
        };
        assert_eq!(close_reason, "Removed from the room by the host");
        host.shutdown().await;
        client.shutdown().await;
    }
}
