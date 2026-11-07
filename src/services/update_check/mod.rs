//! Telling an administrator a new version is out (RFC-028).
//!
//! **Nothing is sent until the check is on**: `WAVEFLOW_UPDATE_CHECK=on`, or,
//! under the default `ask`, an administrator's yes stored in
//! `instance_update_check`. The request itself is `remote::update_check`'s;
//! this decides when it is made and keeps what it brought back, in memory —
//! a restart asks again rather than trust what an older run read.

use std::sync::{Arc, RwLock};

use tokio::sync::Notify;

use super::*;
use crate::config::{UpdateCheckPolicy, UpdateCheckSettings};
use crate::remote::update_check::{self, Channel, Failure, Manifest, UpdateClient};

/// What the last attempt left behind.
#[derive(Debug, Clone, Default)]
struct Known {
    /// The manifest of the last attempt that read one.
    latest: Option<(Manifest, semver::Version)>,
    /// When the last attempt ended, read or not.
    checked_at: Option<i64>,
    /// Why the last attempt read nothing; cleared by the next that reads one.
    failure: Option<Failure>,
}

/// The settings, the client, what is known, and the bell that starts a check
/// at once when an administrator turns the check on.
#[derive(Clone)]
pub(super) struct UpdateRuntime {
    settings: UpdateCheckSettings,
    client: Option<UpdateClient>,
    known: Arc<RwLock<Known>>,
    wake: Arc<Notify>,
}

impl UpdateRuntime {
    /// The runtime for these settings, with nothing known yet.
    pub(super) fn new(settings: UpdateCheckSettings) -> Self {
        // Building sends nothing. A client that cannot be built (no TLS
        // roots) leaves the check unable to run, and says so once.
        let client = match UpdateClient::new(&settings) {
            Ok(client) => Some(client),
            Err(error) => {
                tracing::warn!(error = %error, "the update check's client could not be built");
                None
            }
        };
        Self {
            settings,
            client,
            known: Arc::new(RwLock::new(Known::default())),
            wake: Arc::new(Notify::new()),
        }
    }
}

/// The update check as an administrator sees it (`GET /api/v2/server/update`).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UpdateStatus {
    /// `WAVEFLOW_UPDATE_CHECK`: `ask`, `on` or `off`.
    pub policy: &'static str,
    /// Under `ask`, the stored answer; `null` while nobody has answered, which
    /// is when the web client puts the question.
    pub decision: Option<bool>,
    /// Whether the server checks, all things considered.
    pub enabled: bool,
    /// The running version.
    pub current: &'static str,
    /// The manifest this version follows.
    pub channel: Channel,
    /// The manifest the last successful check read, while the check is on.
    pub latest: Option<Manifest>,
    /// Whether that manifest names a version newer than the running one.
    pub update_available: bool,
    /// When the last check ended, Unix epoch milliseconds.
    pub checked_at: Option<i64>,
    /// Why the last check read nothing: `unreachable`, `status`, `unreadable`
    /// or `invalid`; `null` when it read a manifest.
    pub failure: Option<&'static str>,
}

impl DomainServices {
    /// The stored answer under `ask`: `None` while nobody has answered.
    async fn update_decision(&self) -> Result<Option<bool>, ServiceError> {
        let enabled = sqlx::query_scalar::<_, i64>(
            "SELECT enabled FROM instance_update_check WHERE singleton = 1",
        )
        .fetch_optional(self.db.pool())
        .await?;
        Ok(enabled.map(|value| value == 1))
    }

    /// Whether the check runs now: the environment's word when it gave one,
    /// the stored answer otherwise.
    async fn update_check_enabled(&self) -> Result<bool, ServiceError> {
        Ok(match self.update.settings.policy {
            UpdateCheckPolicy::On => true,
            UpdateCheckPolicy::Off => false,
            UpdateCheckPolicy::Ask => self.update_decision().await? == Some(true),
        })
    }

    /// The update check as the route reports it. Administrators only.
    pub async fn update_status(&self, actor_id: Uuid) -> Result<UpdateStatus, ServiceError> {
        self.require_admin(actor_id).await?;
        let policy = self.update.settings.policy;
        let decision = match policy {
            UpdateCheckPolicy::Ask => self.update_decision().await?,
            UpdateCheckPolicy::On | UpdateCheckPolicy::Off => None,
        };
        let enabled = self.update_check_enabled().await?;
        let running = update_check::running_version();
        let known = self
            .update
            .known
            .read()
            .expect("update check state lock poisoned")
            .clone();
        // A manifest read before the check was turned off is not shown: off
        // means the server knows nothing it did not already announce.
        let latest = known.latest.filter(|_| enabled);
        Ok(UpdateStatus {
            policy: policy.as_str(),
            decision,
            enabled,
            current: env!("CARGO_PKG_VERSION"),
            channel: Channel::of(&running),
            update_available: latest
                .as_ref()
                .is_some_and(|(_, version)| update_check::is_newer(version, &running)),
            latest: latest.map(|(manifest, _)| manifest),
            checked_at: known.checked_at,
            failure: known.failure.map(|failure| match failure {
                Failure::Unreachable => "unreachable",
                Failure::Status => "status",
                Failure::Unreadable => "unreadable",
                Failure::Invalid => "invalid",
            }),
        })
    }

    /// Records an administrator's answer under `ask`; `Conflict` when the
    /// environment has decided. A yes starts a check at once.
    pub async fn set_update_check(
        &self,
        actor_id: Uuid,
        enabled: bool,
    ) -> Result<UpdateStatus, ServiceError> {
        self.require_admin(actor_id).await?;
        if self.update.settings.policy != UpdateCheckPolicy::Ask {
            return Err(ServiceError::Conflict);
        }
        {
            let _writer = self.db.writer_guard().await;
            sqlx::query(
                "INSERT INTO instance_update_check (singleton, enabled, decided_by, decided_at) \
                 VALUES (1, ?, ?, ?) \
                 ON CONFLICT (singleton) DO UPDATE SET enabled = excluded.enabled, \
                   decided_by = excluded.decided_by, decided_at = excluded.decided_at",
            )
            .bind(i64::from(enabled))
            .bind(actor_id.to_string())
            .bind(now_ms())
            .execute(self.db.pool())
            .await?;
        }
        if enabled {
            self.update.wake.notify_one();
        }
        self.update_status(actor_id).await
    }

    /// Reads the manifest once, if the check is on, and keeps the outcome.
    /// Returns whether a request was made.
    pub async fn check_for_update(&self) -> Result<bool, ServiceError> {
        if !self.update_check_enabled().await? {
            return Ok(false);
        }
        let Some(client) = self.update.client.as_ref() else {
            return Ok(false);
        };
        let channel = Channel::of(&update_check::running_version());
        let outcome = client.fetch(channel).await;
        let mut known = self
            .update
            .known
            .write()
            .expect("update check state lock poisoned");
        known.checked_at = Some(now_ms());
        match outcome {
            Ok(read) => {
                tracing::info!(channel = channel.as_str(), version = %read.1, "update check read its manifest");
                known.latest = Some(read);
                known.failure = None;
            }
            Err(failure) => {
                // The verdict only, as every outbound failure in this server.
                tracing::warn!(channel = channel.as_str(), verdict = %failure, "update check read nothing");
                known.failure = Some(failure);
            }
        }
        Ok(true)
    }

    /// The background check: once shortly after boot, then every interval, and
    /// at once when an administrator turns it on. A check that is off sends
    /// nothing and only waits, so turning it on later needs no restart.
    pub fn spawn_update_checker(&self) {
        if self.update.settings.policy == UpdateCheckPolicy::Off {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            // A yes given during the first delay is acted on at once, and its
            // wake is spent here: left stored, it would end the wait below
            // the moment the first check returned and send a second request.
            tokio::select! {
                () = tokio::time::sleep(update_check::FIRST_CHECK_DELAY) => {}
                () = this.update.wake.notified() => {}
            }
            loop {
                if let Err(error) = this.check_for_update().await {
                    tracing::warn!(error = %error, "update check could not read its setting");
                }
                tokio::select! {
                    () = tokio::time::sleep(this.update.settings.interval) => {}
                    () = this.update.wake.notified() => {}
                }
            }
        });
    }
}
