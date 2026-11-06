//! The update check's one request (RFC-028 decision 2): the channel manifest
//! read from the address the operator configured, under the rules every
//! outbound request in this server follows (`outbound::client`).
//!
//! Nothing here decides whether to ask; `services::update_check` does, and
//! calls [`UpdateClient::fetch`] only once the check is on.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::config::UpdateCheckSettings;
use crate::remote::outbound::{self, BodyError, MAX_ANSWER_BYTES};

/// Which manifest a server reads: a pre-release follows `beta.json`, which
/// every release moves, so it hears of the stable version that follows it;
/// any other version follows `stable.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
}

impl Channel {
    /// The channel a version follows.
    pub fn of(version: &semver::Version) -> Self {
        if version.pre.is_empty() {
            Self::Stable
        } else {
            Self::Beta
        }
    }

    /// The manifest's file name without its extension.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
        }
    }
}

/// A channel manifest, as `scripts/release/channel-manifest.py` writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Manifest {
    /// The newest version on the channel, without its `v`.
    pub version: String,
    pub tag: String,
    /// When the release was published, RFC 3339, as the script wrote it.
    pub published_at: String,
    /// The release's page, where its notes and downloads are.
    pub page: String,
    /// The notes alone, as Markdown, when the release has some on R2.
    pub notes: Option<String>,
}

/// Why a check produced no manifest. One word each, and nothing from the
/// request or the answer, as every outbound verdict in this server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    /// The host could not be reached, or did not answer in time.
    #[error("unreachable")]
    Unreachable,
    /// The host answered with a status other than 200.
    #[error("status")]
    Status,
    /// The answer was over the ceiling, or broke off.
    #[error("unreadable")]
    Unreadable,
    /// The answer was not a manifest, or not a coherent one.
    #[error("invalid")]
    Invalid,
}

/// The running server's version, as `Cargo.toml` names it.
pub fn running_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("Cargo.toml's version is a semantic version")
}

/// Whether `latest` is a newer version than `running`, by Semantic Versioning
/// precedence: a pre-release before its release, build metadata ignored —
/// the rule `channel-manifest.py` applies to the same strings.
pub fn is_newer(latest: &semver::Version, running: &semver::Version) -> bool {
    let key = |version: &semver::Version| {
        (
            version.major,
            version.minor,
            version.patch,
            version.pre.clone(),
        )
    };
    key(latest) > key(running)
}

/// The manifest's version, once the manifest is known to be coherent: the
/// version parses, the tag is `v` and that version, a stable channel names no
/// pre-release, and the page and the notes are `http` or `https` addresses —
/// the web client renders them as links, and nothing else is one.
pub fn validate(manifest: &Manifest, channel: Channel) -> Result<semver::Version, Failure> {
    let version = semver::Version::parse(&manifest.version).map_err(|_| Failure::Invalid)?;
    if manifest.tag != format!("v{}", manifest.version) {
        return Err(Failure::Invalid);
    }
    if channel == Channel::Stable && !version.pre.is_empty() {
        return Err(Failure::Invalid);
    }
    let web_address = |raw: &str| {
        reqwest::Url::parse(raw).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
    };
    if !web_address(&manifest.page) || !manifest.notes.as_deref().is_none_or(web_address) {
        return Err(Failure::Invalid);
    }
    Ok(version)
}

/// The client the check reads its manifest with.
#[derive(Clone)]
pub struct UpdateClient {
    http: reqwest::Client,
    base_url: String,
    timeout: Duration,
}

impl UpdateClient {
    /// Built at boot; building sends nothing.
    pub fn new(settings: &UpdateCheckSettings) -> Result<Self, reqwest::Error> {
        Ok(Self {
            http: outbound::client(settings.timeout)?,
            base_url: settings.base_url.clone(),
            timeout: settings.timeout,
        })
    }

    /// Reads and checks one channel's manifest. A failure is a verdict, never
    /// an error carrying the address or the answer.
    ///
    /// **The whole request is bounded here**, the answer's body included: the
    /// outbound client bounds only the connection and leaves the rest to each
    /// caller (`outbound::client`). A host that accepts and then trickles, or
    /// never answers, would otherwise hold the background check for ever.
    pub async fn fetch(&self, channel: Channel) -> Result<(Manifest, semver::Version), Failure> {
        tokio::time::timeout(self.timeout, self.read(channel))
            .await
            .unwrap_or(Err(Failure::Unreachable))
    }

    async fn read(&self, channel: Channel) -> Result<(Manifest, semver::Version), Failure> {
        let response = self
            .http
            .get(format!("{}/{}.json", self.base_url, channel.as_str()))
            .send()
            .await
            .map_err(|_| Failure::Unreachable)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(Failure::Status);
        }
        let body = outbound::read_bounded_body(response, MAX_ANSWER_BYTES)
            .await
            .map_err(|error| match error {
                BodyError::Unread | BodyError::TooLarge => Failure::Unreadable,
            })?;
        let manifest: Manifest = serde_json::from_slice(&body).map_err(|_| Failure::Invalid)?;
        let version = validate(&manifest, channel)?;
        Ok((manifest, version))
    }
}

/// How long the boot waits before the first check, so the first request does
/// not race the server's own start.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests;
