use anyhow::{anyhow, Context, Result};
use igd_next::{aio::tokio::search_gateway, PortMappingProtocol, SearchOptions};
use std::net::{IpAddr, SocketAddr};

#[derive(Debug, Clone)]
pub struct PortMapping {
    pub local_address: SocketAddr,
    pub external_address: SocketAddr,
    pub method: &'static str,
    pub lease_seconds: u32,
}

/// Attempts standards-based router discovery and creates a temporary UDP mapping.
/// A finite lease makes a crashed runtime self-cleaning; long rooms renew by
/// calling this operation again before the lease expires.
pub async fn map_host_port(port: u16) -> Result<PortMapping> {
    let local_ip = local_ip_address::local_ip().context("discover LAN address")?;
    if !matches!(local_ip, IpAddr::V4(_)) {
        return Err(anyhow!("automatic router mapping currently requires IPv4"));
    }
    let gateway = search_gateway(SearchOptions::default())
        .await
        .context("no UPnP Internet Gateway Device answered discovery")?;
    let local_address = SocketAddr::new(local_ip, port);
    let lease_seconds = 7_200;
    gateway
        .add_port(
            PortMappingProtocol::UDP,
            port,
            local_address,
            lease_seconds,
            "Beatblock Together direct host",
        )
        .await
        .context("router rejected the UPnP UDP mapping")?;
    let external_ip = gateway
        .get_external_ip()
        .await
        .context("router did not report its public address")?;
    Ok(PortMapping {
        local_address,
        external_address: SocketAddr::new(external_ip, port),
        method: "UPnP IGD",
        lease_seconds,
    })
}
