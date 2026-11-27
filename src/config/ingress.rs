//! Home Assistant's ingress (RFC-030).

use std::net::SocketAddr;

use super::env::read_trimmed_env;

/// The Supervisor's address on Home Assistant's internal network, the only
/// client an ingress port has.
pub const DEFAULT_INGRESS_PEER: &str = "172.30.32.2";

/// The listener that serves the web client inside Home Assistant.
#[derive(Debug, Clone)]
pub struct IngressSettings {
    /// `WAVEFLOW_INGRESS_BIND`: where the ingress listener listens. Unset by
    /// default, and then there is no such listener and nothing in RFC-030
    /// happens. The Home Assistant app sets it to `0.0.0.0:8099`, the port its
    /// `config.yaml` names as `ingress_port`.
    pub bind: Option<SocketAddr>,
    /// `WAVEFLOW_INGRESS_PEERS`: the addresses or CIDR blocks that may connect
    /// to that listener, comma-separated, `172.30.32.2` by default. Every other
    /// connection is dropped as it arrives. Read only when the listener is on.
    pub peers: Vec<ipnet::IpNet>,
}

impl IngressSettings {
    /// No listener: what a server that is not a Home Assistant app runs with.
    pub fn off() -> Self {
        Self {
            bind: None,
            peers: Vec::new(),
        }
    }
}

pub(super) fn parse_ingress_settings() -> anyhow::Result<IngressSettings> {
    let Some(bind) = read_trimmed_env("WAVEFLOW_INGRESS_BIND") else {
        return Ok(IngressSettings::off());
    };
    let bind = bind
        .parse::<SocketAddr>()
        .map_err(|_| anyhow::anyhow!("invalid WAVEFLOW_INGRESS_BIND {bind:?}"))?;
    let list =
        read_trimmed_env("WAVEFLOW_INGRESS_PEERS").unwrap_or_else(|| DEFAULT_INGRESS_PEER.into());
    let peers = super::access::parse_addresses("WAVEFLOW_INGRESS_PEERS", &list)?;
    // An empty list would be a listener nobody may reach: a typo, never a
    // choice, so it refuses to boot rather than serving nothing.
    if peers.is_empty() {
        anyhow::bail!("WAVEFLOW_INGRESS_PEERS names no address");
    }
    Ok(IngressSettings {
        bind: Some(bind),
        peers,
    })
}
