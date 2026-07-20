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
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, RwLock, Semaphore};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;
const AUTH_IDENTITY: &[u8] = b"beatblock-online-room-v3";
const MAX_CONTROL_FRAME: usize = 64 * 1024;
const AUTH_FAILURE_WINDOW: Duration = Duration::from_secs(60);
const MAX_PASSWORD_FAILURE_IPS: usize = 4_096;
const MAX_PASSWORD_FAILURES_PER_IP: usize = 5;
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PENDING_AUTHENTICATIONS: usize = 64;
const OUTGOING_QUEUE_CAPACITY: usize = 512;
const TARGETED_SEND_TIMEOUT: Duration = Duration::from_millis(250);
const TRANSFER_HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TRANSFER_REQUEST_ID_CHARS: usize = 80;
const MAX_TRANSFER_NAME_CHARS: usize = 256;

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
    ChartTransferProgress {
        session_id: String,
        request_id: String,
        received: u64,
        total: u64,
    },
    ChartTransferReceived {
        session_id: String,
        header: ChartTransferHeader,
        path: PathBuf,
        executable_content_confirmed: bool,
    },
    Disconnected {
        session_id: String,
        reason: String,
    },
    Diagnostic(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartTransferHeader {
    pub request_id: String,
    pub name: String,
    pub size: u64,
    pub archive_sha256: String,
    pub chart_hash: String,
    pub contains_executable_content: bool,
}

impl ChartTransferHeader {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.request_id.is_empty()
            || self.request_id.chars().count() > MAX_TRANSFER_REQUEST_ID_CHARS
        {
            bail!("chart transfer request id exceeds the safety limit");
        }
        if self.name.is_empty() || self.name.chars().count() > MAX_TRANSFER_NAME_CHARS {
            bail!("chart transfer name exceeds the safety limit");
        }
        if self.size == 0 || self.size > crate::transfer::MAX_TRANSFER_BYTES {
            bail!("chart transfer size is outside the 1 byte to 1 GiB safety range");
        }
        if !is_sha256_hex(&self.archive_sha256) || !is_sha256_hex(&self.chart_hash) {
            bail!("chart transfer metadata contains an invalid SHA-256 digest");
        }
        Ok(())
    }
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
    certificate_sha256: String,
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
    #[serde(default)]
    server_proof: Option<String>,
}

#[derive(Debug, Clone)]
struct IncomingTransferAuthorization {
    header: ChartTransferHeader,
    executable_content_confirmed: bool,
}

/// Removes a partially received archive if its task errors, disconnects, or is
/// cancelled. Completed files are explicitly disarmed after ownership passes
/// to the application state.
struct TemporaryTransferFile {
    path: PathBuf,
    armed: bool,
}

impl Drop for TemporaryTransferFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
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
    incoming_transfer_authorizations:
        Arc<RwLock<HashMap<(String, String), IncomingTransferAuthorization>>>,
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
            incoming_transfer_authorizations: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn start_host(&self, port: u16, password: String) -> Result<SocketAddr> {
        self.shutdown().await;
        self.resume_sessions.write().await.clear();
        *self.client_resume_token.write().await = None;
        if password.chars().count() < 4 || password.chars().count() > 128 {
            bail!("room password must contain 4-128 characters");
        }
        let certified = generate_simple_self_signed(vec!["beatblock-online.local".into()])?;
        let cert_der = certified.cert.der().clone();
        let certificate_sha256: [u8; 32] = Sha256::digest(cert_der.as_ref()).into();
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
                let certificate_sha256 = certificate_sha256;
                tokio::spawn(async move {
                    let _authentication_slot = authentication_slot;
                    let connected = tokio::time::timeout(AUTH_TIMEOUT, incoming).await;
                    let error = match connected {
                        Ok(Ok(connection)) => {
                            match tokio::time::timeout(
                                AUTH_TIMEOUT,
                                hub.accept_peer(connection.clone(), &password, &certificate_sha256),
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
        if password.chars().count() < 4 || password.chars().count() > 128 {
            bail!("room password must contain 4-128 characters");
        }
        let display_name = display_name.trim();
        if display_name.is_empty() || display_name.chars().count() > 48 {
            bail!("display name must contain 1-48 characters");
        }
        if role == ParticipantRole::Host {
            bail!("a joining peer cannot request the host role");
        }
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
            .connect(address, "beatblock-online.local")?
            .await
            .context("connect to host QUIC endpoint")?;
        let observed_certificate_sha256 = peer_certificate_sha256(&connection)?;
        let (mut send, mut recv) = connection.open_bi().await?;
        let (spake, client_message) = Spake2::<Ed25519Group>::start_symmetric(
            &Password::new(password.as_bytes()),
            &Identity::new(AUTH_IDENTITY),
        );
        write_frame(
            &mut send,
            &AuthHello {
                version: PROTOCOL_VERSION,
                display_name: display_name.trim().into(),
                role,
                spake_message: BASE64.encode(&client_message),
                resume_token: self.client_resume_token.read().await.clone(),
            },
        )
        .await?;
        let challenge: AuthChallenge = read_frame(&mut recv).await?;
        if challenge.version != PROTOCOL_VERSION {
            bail!("host uses incompatible protocol version");
        }
        let challenged_certificate_sha256 = hex::decode(&challenge.certificate_sha256)
            .context("host certificate digest is invalid")?;
        if !constant_time_eq(&challenged_certificate_sha256, &observed_certificate_sha256) {
            bail!("host authentication was bound to a different TLS certificate");
        }
        let server_message = BASE64.decode(&challenge.spake_message)?;
        let nonce = BASE64.decode(&challenge.nonce)?;
        let key = spake
            .finish(&server_message)
            .map_err(|_| anyhow::anyhow!("password exchange failed"))?;
        let proof = auth_proof(
            &key,
            &client_message,
            &server_message,
            &nonce,
            &observed_certificate_sha256,
            b"client",
        )?;
        write_frame(&mut send, &AuthProof { proof }).await?;
        let welcome: AuthWelcome = read_frame(&mut recv).await?;
        if !welcome.accepted {
            bail!(welcome.message);
        }
        let expected_server_proof = auth_proof(
            &key,
            &client_message,
            &server_message,
            &nonce,
            &observed_certificate_sha256,
            b"server",
        )?;
        let server_proof = welcome
            .server_proof
            .as_deref()
            .context("host did not prove possession of the room password")?;
        if !constant_time_eq(server_proof.as_bytes(), expected_server_proof.as_bytes()) {
            bail!("host password proof is invalid");
        }
        *self.client_resume_token.write().await = welcome.resume_token.clone();
        let (outgoing_tx, outgoing_rx) = mpsc::channel(OUTGOING_QUEUE_CAPACITY);
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
            .iter()
            .map(|(session_id, writer)| (session_id.clone(), writer.clone()))
            .collect::<Vec<_>>();
        let mut overloaded_peers = Vec::new();
        for (session_id, writer) in peer_writers {
            if writer.try_send(envelope.clone()).is_err() {
                overloaded_peers.push(session_id);
            }
        }
        for session_id in overloaded_peers {
            self.disconnect_peer(
                &session_id,
                "Disconnected because the control-message queue was full",
                true,
            )
            .await;
        }
        let server_writer = self.server_writer.read().await.clone();
        if let Some(writer) = server_writer {
            if writer.try_send(envelope).is_err() {
                if let Some(connection) = self.server_connection.read().await.clone() {
                    connection.close(1u32.into(), b"control-message queue was full");
                }
                *self.server_writer.write().await = None;
                *self.server_connection.write().await = None;
            }
        }
    }

    pub async fn send_to(&self, session_id: &str, envelope: Envelope) -> Result<()> {
        if let Some(writer) = self.peers.read().await.get(session_id).cloned() {
            match tokio::time::timeout(TARGETED_SEND_TIMEOUT, writer.send(envelope)).await {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(error)) => {
                    self.disconnect_peer(session_id, "Control stream closed", true)
                        .await;
                    return Err(error.into());
                }
                Err(_) => {
                    self.disconnect_peer(
                        session_id,
                        "Disconnected because the control-message queue was full",
                        true,
                    )
                    .await;
                    bail!("timed out enqueueing a control message for {session_id}");
                }
            }
        }
        if let Some(writer) = self.server_writer.read().await.clone() {
            match tokio::time::timeout(TARGETED_SEND_TIMEOUT, writer.send(envelope)).await {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(error)) => {
                    if let Some(connection) = self.server_connection.read().await.clone() {
                        connection.close(1u32.into(), b"control stream closed");
                    }
                    *self.server_writer.write().await = None;
                    *self.server_connection.write().await = None;
                    return Err(error.into());
                }
                Err(_) => {
                    if let Some(connection) = self.server_connection.read().await.clone() {
                        connection.close(1u32.into(), b"control-message queue was full");
                    }
                    *self.server_writer.write().await = None;
                    *self.server_connection.write().await = None;
                    bail!("timed out enqueueing a control message for the room host");
                }
            }
        }
        bail!("network session is not connected")
    }

    /// Grants exactly one incoming archive stream permission for the supplied
    /// session and metadata. Authorization is consumed even when a sender
    /// reuses the request ID with different metadata, preventing retries from
    /// turning a prior consent decision into a wildcard.
    pub async fn authorize_incoming_chart_transfer(
        &self,
        session_id: &str,
        header: ChartTransferHeader,
        executable_content_confirmed: bool,
    ) -> Result<()> {
        header.validate()?;
        if header.contains_executable_content && !executable_content_confirmed {
            bail!("executable chart content requires explicit confirmation");
        }
        let key = (session_id.to_owned(), header.request_id.clone());
        let mut authorizations = self.incoming_transfer_authorizations.write().await;
        if authorizations.contains_key(&key) {
            bail!("chart transfer request is already authorized");
        }
        authorizations.insert(
            key,
            IncomingTransferAuthorization {
                header,
                executable_content_confirmed,
            },
        );
        Ok(())
    }

    pub async fn clear_incoming_chart_transfers(&self) {
        self.incoming_transfer_authorizations.write().await.clear();
    }

    pub async fn send_render_sample(&self, sample: &RenderSample) -> Result<()> {
        let bytes = sample.encode().to_vec().into();
        if let Some(connection) = self.server_connection.read().await.as_ref() {
            connection.send_datagram(bytes)?;
            return Ok(());
        }
        bail!("render datagrams are sent by participants, not the host")
    }

    /// Relays an already validated render sample only to authorized
    /// Commentators. This avoids broadcasting high-rate telemetry to ordinary
    /// spectators and keeps local mirror enablement an explicit subscription.
    pub async fn send_render_sample_to(
        &self,
        session_ids: &[String],
        sample: &RenderSample,
    ) -> Result<()> {
        let bytes = sample.encode().to_vec();
        let connections = self.peer_connections.read().await;
        for session_id in session_ids {
            if let Some(connection) = connections.get(session_id) {
                connection.send_datagram(bytes.clone().into())?;
            }
        }
        Ok(())
    }

    /// Sends one bounded chart archive on a QUIC uni stream. Control messages
    /// carry consent and metadata; package bytes never compete with the room's
    /// ordered control stream.
    pub async fn send_chart_transfer(
        &self,
        session_id: &str,
        header: &ChartTransferHeader,
        path: &std::path::Path,
    ) -> Result<()> {
        if header.size > crate::transfer::MAX_TRANSFER_BYTES {
            bail!("chart transfer exceeds 1 GiB");
        }
        let connection = self
            .peer_connections
            .read()
            .await
            .get(session_id)
            .cloned()
            .context("chart transfer peer is not connected")?;
        let mut stream = connection.open_uni().await?;
        write_frame(&mut stream, header).await?;
        let mut file = tokio::fs::File::open(path).await?;
        let mut buffer = vec![0u8; 64 * 1024];
        let mut sent = 0u64;
        loop {
            let count = file.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            sent = sent.saturating_add(count as u64);
            if sent > header.size || sent > crate::transfer::MAX_TRANSFER_BYTES {
                stream.reset(2u32.into())?;
                bail!("chart package changed or exceeded its offered size");
            }
            stream.write_all(&buffer[..count]).await?;
        }
        if sent != header.size {
            stream.reset(2u32.into())?;
            bail!("chart package size changed during transfer");
        }
        stream.finish()?;
        Ok(())
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
        self.incoming_transfer_authorizations.write().await.clear();
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
        self.incoming_transfer_authorizations
            .write()
            .await
            .retain(|(authorized_session, _), _| authorized_session != session_id);
        if let Some(connection) = self.peer_connections.write().await.remove(session_id) {
            connection.close(1u32.into(), reason.as_bytes());
        }
    }

    pub async fn clear_client_resume(&self) {
        *self.client_resume_token.write().await = None;
    }

    async fn accept_peer(
        &self,
        connection: Connection,
        password: &str,
        certificate_sha256: &[u8; 32],
    ) -> Result<()> {
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
                    message: "This room uses Beatblock Online protocol v3; update every client"
                        .into(),
                    resume_token: None,
                    server_proof: None,
                },
            )
            .await?;
            bail!("incompatible participant protocol");
        }
        let client_message = BASE64.decode(hello.spake_message)?;
        let (spake, server_message) = Spake2::<Ed25519Group>::start_symmetric(
            &Password::new(password.as_bytes()),
            &Identity::new(AUTH_IDENTITY),
        );
        let key = spake
            .finish(&client_message)
            .map_err(|_| anyhow::anyhow!("password exchange failed"))?;
        let nonce_bytes: [u8; 32] = rand::random();
        write_frame(
            &mut send,
            &AuthChallenge {
                version: PROTOCOL_VERSION,
                spake_message: BASE64.encode(&server_message),
                nonce: BASE64.encode(nonce_bytes),
                certificate_sha256: hex::encode(certificate_sha256),
            },
        )
        .await?;
        let proof: AuthProof = read_frame(&mut recv).await?;
        let expected = auth_proof(
            &key,
            &client_message,
            &server_message,
            &nonce_bytes,
            certificate_sha256,
            b"client",
        )?;
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
                    server_proof: None,
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
                            server_proof: None,
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
                server_proof: Some(auth_proof(
                    &key,
                    &client_message,
                    &server_message,
                    &nonce_bytes,
                    certificate_sha256,
                    b"server",
                )?),
            },
        )
        .await?;
        let (outgoing_tx, outgoing_rx) = mpsc::channel(OUTGOING_QUEUE_CAPACITY);
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
        let incoming_transfer_authorizations = self.incoming_transfer_authorizations.clone();
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
            let transfer_events = events.clone();
            let transfer_session = session_id.clone();
            let transfer_connection = connection.clone();
            let transfer_authorizations = incoming_transfer_authorizations.clone();
            let transfers = tokio::spawn(async move {
                while let Ok(mut stream) = transfer_connection.accept_uni().await {
                    let result = async {
                        let header: ChartTransferHeader =
                            tokio::time::timeout(TRANSFER_HEADER_TIMEOUT, read_frame(&mut stream))
                                .await
                                .context("chart transfer header stalled for 10 seconds")??;
                        header.validate()?;
                        let authorization = consume_incoming_transfer_authorization(
                            &transfer_authorizations,
                            &transfer_session,
                            &header,
                        )
                        .await?;
                        let directory = std::env::temp_dir().join("beatblock-online-transfers");
                        tokio::fs::create_dir_all(&directory).await?;
                        // Never derive local paths from peer-controlled request
                        // IDs. A UUID plus create_new also prevents clobbering a
                        // concurrently received or pre-existing file.
                        let path = directory.join(format!("{}.partial", Uuid::new_v4()));
                        let mut temporary_file = TemporaryTransferFile {
                            path: path.clone(),
                            armed: true,
                        };
                        let mut file = tokio::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)
                            .await?;
                        let mut received = 0u64;
                        loop {
                            let chunk = tokio::time::timeout(
                                Duration::from_secs(30),
                                stream.read_chunk(64 * 1024, true),
                            )
                            .await
                            .context("chart transfer stalled for 30 seconds")??;
                            let Some(chunk) = chunk else { break };
                            received = received.saturating_add(chunk.bytes.len() as u64);
                            if received > header.size
                                || received > crate::transfer::MAX_TRANSFER_BYTES
                            {
                                bail!("incoming chart transfer exceeded its offered size");
                            }
                            file.write_all(&chunk.bytes).await?;
                            let _ = transfer_events.try_send(NetworkEvent::ChartTransferProgress {
                                session_id: transfer_session.clone(),
                                request_id: header.request_id.clone(),
                                received,
                                total: header.size,
                            });
                        }
                        file.flush().await?;
                        if received != header.size {
                            bail!("incoming chart transfer ended before completion");
                        }
                        tokio::time::timeout(
                            Duration::from_secs(5),
                            transfer_events.send(NetworkEvent::ChartTransferReceived {
                                session_id: transfer_session.clone(),
                                header,
                                path: path.clone(),
                                executable_content_confirmed: authorization
                                    .executable_content_confirmed,
                            }),
                        )
                        .await
                        .context("application event queue stalled for 5 seconds")?
                        .context("application stopped before chart transfer completed")?;
                        temporary_file.armed = false;
                        Ok::<(), anyhow::Error>(())
                    }
                    .await;
                    if let Err(error) = result {
                        let _ = transfer_events
                            .send(NetworkEvent::Diagnostic(format!(
                                "chart transfer failed: {error}"
                            )))
                            .await;
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
            transfers.abort();
            peers.write().await.remove(&session_id);
            peer_connections.write().await.remove(&session_id);
            incoming_transfer_authorizations
                .write()
                .await
                .retain(|(authorized_session, _), _| authorized_session != &session_id);
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
        bail!("control message exceeds 64 KiB");
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
        bail!("control message exceeds 64 KiB");
    }
    let mut bytes = vec![0u8; length];
    recv.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Authenticates the complete password-exchange transcript and the certificate
/// actually observed by the client. This turns the otherwise unauthenticated
/// self-signed QUIC certificate into a password-authenticated channel binding.
fn auth_proof(
    key: &[u8],
    client_message: &[u8],
    server_message: &[u8],
    nonce: &[u8],
    certificate_sha256: &[u8],
    label: &[u8],
) -> Result<String> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    for field in [
        AUTH_IDENTITY,
        &[PROTOCOL_VERSION],
        client_message,
        server_message,
        nonce,
        certificate_sha256,
        label,
    ] {
        mac.update(&(field.len() as u64).to_le_bytes());
        mac.update(field);
    }
    Ok(BASE64.encode(mac.finalize().into_bytes()))
}

fn peer_certificate_sha256(connection: &Connection) -> Result<[u8; 32]> {
    let identity = connection
        .peer_identity()
        .context("QUIC peer did not provide a TLS identity")?;
    let certificates = identity
        .downcast::<Vec<CertificateDer<'static>>>()
        .map_err(|_| anyhow::anyhow!("QUIC peer identity was not a certificate chain"))?;
    let certificate = certificates
        .first()
        .context("QUIC peer certificate chain was empty")?;
    Ok(Sha256::digest(certificate.as_ref()).into())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

async fn consume_incoming_transfer_authorization(
    authorizations: &RwLock<HashMap<(String, String), IncomingTransferAuthorization>>,
    session_id: &str,
    header: &ChartTransferHeader,
) -> Result<IncomingTransferAuthorization> {
    header.validate()?;
    let authorization = authorizations
        .write()
        .await
        .remove(&(session_id.to_owned(), header.request_id.clone()))
        .context("unsolicited chart transfer was rejected")?;
    if authorization.header != *header {
        bail!("chart transfer metadata does not match the accepted offer");
    }
    Ok(authorization)
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

    #[tokio::test]
    async fn join_rejects_invalid_credentials_before_network_allocation() {
        let (events, _receiver) = mpsc::channel(1);
        let network = NetworkHub::new(events);
        let address: SocketAddr = "127.0.0.1:9".parse().unwrap();

        assert!(network
            .join(address, "abc", "Player", ParticipantRole::Player)
            .await
            .is_err());
        assert!(network
            .join(address, "password", "", ParticipantRole::Player)
            .await
            .is_err());
        assert!(network
            .join(address, "password", "Player", ParticipantRole::Host)
            .await
            .is_err());
    }

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
            .connect(address, "beatblock-online.local")
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn streams_one_bounded_chart_archive_without_blocking_control() {
        let (host_events_tx, mut host_events) = mpsc::channel(32);
        let host = NetworkHub::new(host_events_tx);
        let address = host.start_host(0, "correct horse".into()).await.unwrap();
        let (client_events_tx, mut client_events) = mpsc::channel(32);
        let client = NetworkHub::new(client_events_tx);
        let client_session = client
            .join(address, "correct horse", "Player", ParticipantRole::Player)
            .await
            .unwrap();
        let peer = loop {
            if let NetworkEvent::Authenticated {
                session_id,
                hosting: true,
                ..
            } = host_events.recv().await.unwrap()
            {
                break session_id;
            }
        };
        let root = std::env::temp_dir().join(format!("bbt-quic-file-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("chart.zip");
        std::fs::write(&source, b"bounded chart bytes").unwrap();
        let header = ChartTransferHeader {
            request_id: "transfer-1".into(),
            name: "chart.zip".into(),
            size: std::fs::metadata(&source).unwrap().len(),
            archive_sha256: "a".repeat(64),
            chart_hash: "b".repeat(64),
            contains_executable_content: false,
        };
        client
            .authorize_incoming_chart_transfer(&client_session, header.clone(), false)
            .await
            .unwrap();
        host.send_chart_transfer(&peer, &header, &source)
            .await
            .unwrap();
        let received = loop {
            if let NetworkEvent::ChartTransferReceived { path, .. } =
                tokio::time::timeout(Duration::from_secs(3), client_events.recv())
                    .await
                    .unwrap()
                    .unwrap()
            {
                break path;
            }
        };
        assert_eq!(std::fs::read(&received).unwrap(), b"bounded chart bytes");
        let _ = std::fs::remove_file(received);

        // The authorization was consumed by the first stream. Replaying the
        // byte-identical stream must be rejected before another file is made.
        let _ = host.send_chart_transfer(&peer, &header, &source).await;
        let diagnostic = loop {
            match tokio::time::timeout(Duration::from_secs(3), client_events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                NetworkEvent::Diagnostic(message) => break message,
                NetworkEvent::ChartTransferReceived { path, .. } => {
                    panic!("replayed transfer reached disk at {}", path.display())
                }
                _ => {}
            }
        };
        assert!(diagnostic.contains("unsolicited chart transfer"));
        let _ = std::fs::remove_dir_all(root);
        host.shutdown().await;
        client.shutdown().await;
    }

    #[tokio::test]
    async fn incoming_transfer_consent_is_exact_and_one_shot() {
        let (events, _receiver) = mpsc::channel(8);
        let hub = NetworkHub::new(events);
        let offered = ChartTransferHeader {
            request_id: "transfer-1".into(),
            name: "chart.zip".into(),
            size: 32,
            archive_sha256: "a".repeat(64),
            chart_hash: "b".repeat(64),
            contains_executable_content: true,
        };
        assert!(hub
            .authorize_incoming_chart_transfer("host", offered.clone(), false)
            .await
            .is_err());
        hub.authorize_incoming_chart_transfer("host", offered.clone(), true)
            .await
            .unwrap();

        let mut mismatched = offered.clone();
        mismatched.size += 1;
        assert!(consume_incoming_transfer_authorization(
            &hub.incoming_transfer_authorizations,
            "host",
            &mismatched,
        )
        .await
        .is_err());
        assert!(consume_incoming_transfer_authorization(
            &hub.incoming_transfer_authorizations,
            "host",
            &offered,
        )
        .await
        .is_err());
        assert!(hub.incoming_transfer_authorizations.read().await.is_empty());
    }

    #[test]
    fn authentication_proof_binds_both_spake_messages_and_certificate() {
        let key = [7u8; 32];
        let client_message = [1u8; 32];
        let server_message = [2u8; 32];
        let nonce = [3u8; 32];
        let certificate = [4u8; 32];
        let proof = auth_proof(
            &key,
            &client_message,
            &server_message,
            &nonce,
            &certificate,
            b"server",
        )
        .unwrap();

        let mut forged_certificate = certificate;
        forged_certificate[0] ^= 1;
        let forged = auth_proof(
            &key,
            &client_message,
            &server_message,
            &nonce,
            &forged_certificate,
            b"server",
        )
        .unwrap();
        assert_ne!(proof, forged);
    }

    #[test]
    fn chart_transfer_header_limits_match_the_protocol_schema() {
        let valid = ChartTransferHeader {
            request_id: "r".repeat(MAX_TRANSFER_REQUEST_ID_CHARS),
            name: "n".repeat(MAX_TRANSFER_NAME_CHARS),
            size: 1,
            archive_sha256: "a".repeat(64),
            chart_hash: "b".repeat(64),
            contains_executable_content: false,
        };
        valid.validate().unwrap();

        let mut invalid = valid.clone();
        invalid.size = 0;
        assert!(invalid.validate().is_err());
        invalid = valid.clone();
        invalid.archive_sha256 = "A".repeat(64);
        assert!(invalid.validate().is_err());
        invalid = valid;
        invalid.request_id.push('x');
        assert!(invalid.validate().is_err());
    }
}
