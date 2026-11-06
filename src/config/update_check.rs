//! The update check (RFC-028).

use std::time::Duration;

use super::env::parse_env;

/// Who decides whether the server asks for the newest version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCheckPolicy {
    /// The default: nothing is sent until an administrator agrees, in the web
    /// client, and the answer is kept in the database.
    Ask,
    /// The environment says yes: the check runs and nobody is asked.
    On,
    /// The environment says no: the check never runs and nobody is asked.
    Off,
}

impl UpdateCheckPolicy {
    /// The value as `WAVEFLOW_UPDATE_CHECK` spells it, and as the route
    /// reports it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

/// Where the manifests are, how often they are read, and who decides.
#[derive(Debug, Clone)]
pub struct UpdateCheckSettings {
    /// `WAVEFLOW_UPDATE_CHECK`: `ask` (default), `on` or `off`.
    pub policy: UpdateCheckPolicy,
    /// `WAVEFLOW_UPDATE_URL`, default `https://download.waveflow.app/server`:
    /// the folder holding `stable.json` and `beta.json`, for a fork that
    /// publishes its own. Plain `http` only for a loopback address, which is
    /// how the test suite serves its manifests.
    pub base_url: String,
    /// One manifest request. Not set: a manifest is a few hundred bytes.
    pub timeout: Duration,
    /// Between two checks once the check is on. Not set: once a day.
    pub interval: Duration,
}

/// The default address of the manifests, compiled into every server.
pub const DEFAULT_UPDATE_URL: &str = "https://download.waveflow.app/server";

/// Reads the two variables, refusing anything but the three policies and an
/// address that is `https`, or `http` on loopback.
pub(super) fn parse_update_check_settings() -> anyhow::Result<UpdateCheckSettings> {
    let raw = parse_env::<String>("WAVEFLOW_UPDATE_CHECK", "ask")?;
    let policy = parse_policy(&raw)?;
    let base_url = parse_env::<String>("WAVEFLOW_UPDATE_URL", DEFAULT_UPDATE_URL)?;
    let base_url = validate_base_url(&base_url)?;
    Ok(UpdateCheckSettings {
        policy,
        base_url,
        timeout: Duration::from_secs(15),
        interval: Duration::from_secs(24 * 60 * 60),
    })
}

/// `ask`, `on` or `off`, in any case; anything else refuses to boot rather
/// than guessing which of the three was meant.
pub(super) fn parse_policy(raw: &str) -> anyhow::Result<UpdateCheckPolicy> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ask" => Ok(UpdateCheckPolicy::Ask),
        "on" | "true" | "1" | "yes" => Ok(UpdateCheckPolicy::On),
        "off" | "false" | "0" | "no" => Ok(UpdateCheckPolicy::Off),
        _ => anyhow::bail!("WAVEFLOW_UPDATE_CHECK must be ask, on or off (got {raw:?})"),
    }
}

/// The folder address without its trailing slash, `https`, or `http` on a
/// loopback host only: the manifest tells an administrator what to install, so
/// it is not read in clear from anywhere else.
pub(super) fn validate_base_url(raw: &str) -> anyhow::Result<String> {
    let url = reqwest::Url::parse(raw.trim())
        .map_err(|error| anyhow::anyhow!("WAVEFLOW_UPDATE_URL is not an address: {error}"))?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => anyhow::bail!("WAVEFLOW_UPDATE_URL must be https (http only on a loopback address)"),
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
