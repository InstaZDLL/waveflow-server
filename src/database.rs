//! SQLite v2 connection, migrations and tenant-safe foundation repositories.

use std::{path::Path, str::FromStr, sync::Arc};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha384};
use sqlx::{
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqliteConnection, SqlitePool,
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{config::Config, security::EncryptedSecret};

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations-v2");

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
    writer: Arc<Mutex<()>>,
    /// The rules this instance derives catalogue identifiers under.
    ///
    /// They live on the writer rather than on the scanner because identity is
    /// the writer's business: the test suite applies catalogue rows directly,
    /// and putting the specs anywhere else would have those tests exercise a
    /// different identity path than production.
    pid: crate::pid::PidSpecs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountRole {
    Admin,
    User,
}

impl AccountRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }
}

impl FromStr for AccountRole {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "admin" => Ok(Self::Admin),
            "user" => Ok(Self::User),
            _ => anyhow::bail!("unknown account role: {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LibraryVisibility {
    Private,
    Shared,
}

impl LibraryVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Shared => "shared",
        }
    }
}

impl FromStr for LibraryVisibility {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "private" => Ok(Self::Private),
            "shared" => Ok(Self::Shared),
            _ => anyhow::bail!("visibility must be private or shared"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LibraryRole {
    Owner,
    Manager,
    Listener,
}

impl LibraryRole {
    /// Whether the role may spend the owner's disk on a rescan.
    ///
    /// The same rule the `scan_job` insert enforces in SQL, named once here
    /// so the Subsonic fan-out over every reachable library agrees with it
    /// instead of discovering the refusal one insert at a time.
    pub fn may_scan(self) -> bool {
        matches!(self, Self::Owner | Self::Manager)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Manager => "manager",
            Self::Listener => "listener",
        }
    }
}

impl FromStr for LibraryRole {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "owner" => Ok(Self::Owner),
            "manager" => Ok(Self::Manager),
            "listener" => Ok(Self::Listener),
            _ => anyhow::bail!("library role must be owner, manager or listener"),
        }
    }
}

/// One issued API token, without its secret.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ApiTokenRecord {
    pub id: Uuid,
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AccountRecord {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub role: AccountRole,
    pub disabled: bool,
}

#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub username: String,
    pub role: AccountRole,
    pub refresh_expires_at: i64,
    /// The scopes this session was issued under. Empty for a password login
    /// and for a grant made by an unscoped credential; a narrowed token that
    /// authorized a device leaves its narrowing here.
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AccessRecord {
    pub user_id: Uuid,
    pub username: String,
    pub role: AccountRole,
    /// The scopes of the API token this request arrived on. Empty for a
    /// session or an OAuth grant, which carry the account's full authority,
    /// and for a token issued without any.
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NewSession<'a> {
    pub id: Uuid,
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub access_token_hash: &'a [u8],
    pub refresh_token_hash: &'a [u8],
    pub access_expires_at: i64,
    pub refresh_expires_at: i64,
    pub now_ms: i64,
    /// Carried from the credential that issued this session, so it can never
    /// be broader than what asked for it.
    pub scopes: &'a [String],
}

#[derive(Debug, Clone)]
pub struct NewAuthorization<'a> {
    pub code_hash: [u8; 32],
    pub user_id: Uuid,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub code_challenge: &'a str,
    pub device_name: &'a str,
    pub now_ms: i64,
    pub expires_at: i64,
    /// The scopes of the credential that authorized this grant.
    pub scopes: &'a [String],
}

#[derive(Debug, Clone)]
pub struct AuthorizationRecord {
    pub user_id: Uuid,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub device_name: String,
    /// Handed to the session this grant is redeemed for.
    pub scopes: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct SyncOperationReplayRecord {
    pub result_entity_id: Option<String>,
    pub event_cursor: i64,
    pub intent_hash: Option<Vec<u8>>,
    pub entity_type: String,
}

#[derive(Debug)]
pub(crate) enum SyncOperationReservation {
    InvalidOriginDevice,
    New,
    Incomplete,
    Replayed(SyncOperationReplayRecord),
}

impl Database {
    pub async fn setup_required(&self) -> Result<bool, sqlx::Error> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM account")
            .fetch_one(&self.pool)
            .await?;
        Ok(count == 0)
    }

    pub async fn bootstrap_admin(
        &self,
        username: &str,
        password_hash: &str,
        now_ms: i64,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        let id = Uuid::new_v4();
        let inserted = sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
             SELECT ?, ?, ?, 'admin', ?, ? WHERE NOT EXISTS (SELECT 1 FROM account)",
        )
        .bind(id.to_string())
        .bind(username.trim())
        .bind(password_hash)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if inserted {
            insert_audit(&mut tx, Some(id), "instance.bootstrapped", Some(id), now_ms).await?;
        }
        tx.commit().await?;
        Ok(inserted.then_some(id))
    }

    pub async fn open(config: &Config) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&config.data_dir).await?;
        let options = SqliteConnectOptions::new()
            .filename(&config.database_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(config.sqlite_busy_timeout);

        let pool = SqlitePoolOptions::new()
            .max_connections(config.db_max_connections)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect_with(options)
            .await
            .map_err(|error| anyhow::anyhow!("sqlite connect failed: {error}"))?;

        Ok(Self {
            pool,
            writer: Arc::new(Mutex::new(())),
            pid: config.pid.clone(),
        })
    }

    pub async fn migrate(&self) -> anyhow::Result<()> {
        let _writer = self.writer_guard().await;
        let normalized = self
            .normalize_legacy_crlf_migration_checksums()
            .await
            .map_err(|error| {
                anyhow::anyhow!("legacy migration checksum normalization failed: {error}")
            })?;
        if normalized > 0 {
            tracing::warn!(
                normalized,
                "normalized legacy Windows CRLF migration checksums"
            );
        }
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|error| anyhow::anyhow!("v2 migration failed: {error}"))
    }

    /// Early Windows builds embedded migrations after Git had converted their
    /// LF line endings to CRLF. SQLx hashes the exact bytes, so databases made
    /// by those builds are rejected after `.gitattributes` made the checkout
    /// byte-stable even though the SQL itself is unchanged.
    ///
    /// Only the checksum of the current migration with every LF converted to
    /// CRLF is accepted. Any other mismatch is left untouched for SQLx to
    /// reject normally.
    async fn normalize_legacy_crlf_migration_checksums(&self) -> Result<u64, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let migrations_table_exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master \
             WHERE type = 'table' AND name = '_sqlx_migrations')",
        )
        .fetch_one(&mut *tx)
        .await?;
        if migrations_table_exists == 0 {
            tx.commit().await?;
            return Ok(0);
        }

        let mut normalized = 0;
        for migration in MIGRATOR.iter() {
            let sql = migration.sql.as_str();
            if sql.contains('\r') || !sql.contains('\n') {
                continue;
            }
            let legacy_checksum = Sha384::digest(sql.replace('\n', "\r\n").as_bytes());
            let result = sqlx::query(
                "UPDATE _sqlx_migrations SET checksum = ? \
                 WHERE version = ? AND success = TRUE AND checksum = ?",
            )
            .bind(migration.checksum.as_ref())
            .bind(migration.version)
            .bind(&legacy_checksum[..])
            .execute(&mut *tx)
            .await?;
            normalized += result.rows_affected();
        }
        tx.commit().await?;
        Ok(normalized)
    }

    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map(|_| ())
    }

    pub async fn integrity_check(&self) -> Result<bool, sqlx::Error> {
        let result: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(&self.pool)
            .await?;
        Ok(result == "ok")
    }

    pub async fn bind_instance_key(
        &self,
        fingerprint: &[u8],
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO instance_metadata (singleton, key_fingerprint, created_at) \
             VALUES (1, ?, ?) ON CONFLICT (singleton) DO NOTHING",
        )
        .bind(fingerprint)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        let stored: Vec<u8> =
            sqlx::query_scalar("SELECT key_fingerprint FROM instance_metadata WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(stored == fingerprint)
    }

    pub async fn backup_to(&self, destination: &Path) -> anyhow::Result<()> {
        if destination.exists() {
            anyhow::bail!("backup database already exists: {}", destination.display());
        }
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let destination = destination
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("backup path is not valid UTF-8"))?;
        let _writer = self.writer_guard().await;
        sqlx::query("VACUUM INTO ?")
            .bind(destination)
            .execute(&self.pool)
            .await
            .map_err(|error| anyhow::anyhow!("SQLite backup failed: {error}"))?;
        Ok(())
    }

    pub async fn check_file(path: &Path) -> anyhow::Result<bool> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let result: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&pool)
            .await?;
        pool.close().await;
        Ok(result == "ok")
    }

    pub async fn check_file_instance_key(path: &Path, fingerprint: &[u8]) -> anyhow::Result<bool> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let stored = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT key_fingerprint FROM instance_metadata WHERE singleton=1",
        )
        .fetch_optional(&pool)
        .await?;
        pool.close().await;
        Ok(stored.is_some_and(|stored| stored == fingerprint))
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// The identity rules the catalogue is written under.
    pub fn pid(&self) -> &crate::pid::PidSpecs {
        &self.pid
    }

    pub(crate) async fn writer_guard(&self) -> OwnedMutexGuard<()> {
        Arc::clone(&self.writer).lock_owned().await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reserve_sync_operation(
        &self,
        _writer_guard: &OwnedMutexGuard<()>,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        operation_id: Uuid,
        origin_device_id: Option<Uuid>,
        intent_hash: &[u8],
        created_at: i64,
    ) -> Result<SyncOperationReservation, sqlx::Error> {
        if let Some(device_id) = origin_device_id {
            let owned = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM device \
                 WHERE id=? AND user_id=? AND revoked_at IS NULL)",
            )
            .bind(device_id.to_string())
            .bind(user_id.to_string())
            .fetch_one(&mut *connection)
            .await?;
            if !owned {
                return Ok(SyncOperationReservation::InvalidOriginDevice);
            }
        }

        let inserted = sqlx::query(
            "INSERT INTO sync_operation \
             (user_id, operation_id, origin_device_id, intent_hash, created_at) VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (user_id, operation_id) DO NOTHING",
        )
        .bind(user_id.to_string())
        .bind(operation_id.to_string())
        .bind(origin_device_id.map(|id| id.to_string()))
        .bind(intent_hash)
        .bind(created_at)
        .execute(&mut *connection)
        .await?
        .rows_affected();
        if inserted == 1 {
            return Ok(SyncOperationReservation::New);
        }

        let row = sqlx::query(
            "SELECT so.result_entity_id, so.event_cursor, so.intent_hash, se.entity_type \
             FROM sync_operation so JOIN sync_event se ON se.cursor=so.event_cursor AND se.user_id=so.user_id \
             WHERE so.user_id=? AND so.operation_id=? AND so.applied_at IS NOT NULL",
        )
        .bind(user_id.to_string())
        .bind(operation_id.to_string())
        .fetch_optional(&mut *connection)
        .await?;
        let Some(row) = row else {
            return Ok(SyncOperationReservation::Incomplete);
        };
        Ok(SyncOperationReservation::Replayed(
            SyncOperationReplayRecord {
                result_entity_id: row.try_get("result_entity_id")?,
                event_cursor: row.try_get("event_cursor")?,
                intent_hash: row.try_get("intent_hash")?,
                entity_type: row.try_get("entity_type")?,
            },
        ))
    }

    pub async fn create_account(
        &self,
        username: &str,
        password_hash: &str,
        role: AccountRole,
        now_ms: i64,
    ) -> Result<Uuid, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(username.trim())
        .bind(password_hash)
        .bind(role.as_str())
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        insert_audit(&mut tx, Some(id), "account.created", Some(id), now_ms).await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn account_by_username(
        &self,
        username: &str,
    ) -> Result<Option<AccountRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, disabled \
             FROM account WHERE username = ? COLLATE NOCASE",
        )
        .bind(username.trim())
        .fetch_optional(&self.pool)
        .await?;
        row.map(account_from_row).transpose()
    }

    pub async fn account_by_id(&self, id: Uuid) -> Result<Option<AccountRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, disabled FROM account WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(account_from_row).transpose()
    }

    pub async fn create_authorization(
        &self,
        grant: NewAuthorization<'_>,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO oauth_authorization \
               (code_hash, user_id, client_id, redirect_uri, code_challenge, device_name, \
                created_at, expires_at, scopes_json) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(grant.code_hash.as_slice())
        .bind(grant.user_id.to_string())
        .bind(grant.client_id)
        .bind(grant.redirect_uri)
        .bind(grant.code_challenge)
        .bind(grant.device_name)
        .bind(grant.now_ms)
        .bind(grant.expires_at)
        .bind(encode_scopes(grant.scopes)?)
        .execute(&mut *tx)
        .await?;
        // Delegating access to another application is exactly the kind of event
        // the audit trail exists for, alongside account and credential changes.
        insert_audit(
            &mut tx,
            Some(grant.user_id),
            "oauth.authorization.granted",
            Some(grant.user_id),
            grant.now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Claims a grant for redemption, atomically marking it used.
    ///
    /// The `redeemed_at IS NULL` predicate inside the UPDATE is what makes a
    /// code single-use: two concurrent redemptions cannot both match, so a
    /// stolen code cannot be replayed alongside the legitimate exchange.
    ///
    /// A code is spent by the first presentation whatever its outcome, so a
    /// failed PKCE check burns it rather than leaving it open to further
    /// attempts. That forecloses guessing a verifier, and matches OAuth 2.1's
    /// rule that a code presented more than once must be revoked. The cost is
    /// that a client which botches its own exchange must restart the flow.
    pub async fn redeem_authorization(
        &self,
        code_hash: &[u8],
        now_ms: i64,
    ) -> Result<Option<AuthorizationRecord>, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let row = sqlx::query(
            "UPDATE oauth_authorization SET redeemed_at = ? \
             WHERE code_hash = ? AND redeemed_at IS NULL AND expires_at > ? \
             RETURNING user_id, client_id, redirect_uri, code_challenge, device_name, \
                       scopes_json",
        )
        .bind(now_ms)
        .bind(code_hash)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(AuthorizationRecord {
                user_id: parse_uuid(row.try_get("user_id")?)?,
                client_id: row.try_get("client_id")?,
                redirect_uri: row.try_get("redirect_uri")?,
                code_challenge: row.try_get("code_challenge")?,
                device_name: row.try_get("device_name")?,
                scopes: decode_scopes(row.try_get("scopes_json")?)?,
            })
        })
        .transpose()
    }

    /// Sweeps expired grants hourly for the life of the process.
    ///
    /// Rows are kept after redemption so a replay is recognised rather than
    /// looking unknown, so nothing else ever deletes them; without this the
    /// table only grows.
    pub fn spawn_authorization_pruning(&self) {
        let db = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
            loop {
                ticker.tick().await;
                match db
                    .prune_authorizations(crate::authentication::now_ms())
                    .await
                {
                    Ok(0) => {}
                    Ok(removed) => tracing::debug!(removed, "pruned expired authorization codes"),
                    Err(error) => {
                        tracing::warn!(error = %error, "could not prune authorization codes")
                    }
                }
            }
        });
    }

    /// Drops grants that can no longer be redeemed.
    pub async fn prune_authorizations(&self, now_ms: i64) -> Result<u64, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query("DELETE FROM oauth_authorization WHERE expires_at <= ?")
            .bind(now_ms)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn create_device(
        &self,
        user_id: Uuid,
        name: &str,
        now_ms: i64,
    ) -> Result<Uuid, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO device (id, user_id, name, created_at, last_seen_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(name.trim())
        .bind(now_ms)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn create_session(&self, session: NewSession<'_>) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        sqlx::query(
            "INSERT INTO session \
             (id, user_id, device_id, access_token_hash, refresh_token_hash, access_expires_at, \
              refresh_expires_at, created_at, last_used_at, scopes_json) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session.id.to_string())
        .bind(session.user_id.to_string())
        .bind(session.device_id.to_string())
        .bind(session.access_token_hash)
        .bind(session.refresh_token_hash)
        .bind(session.access_expires_at)
        .bind(session.refresh_expires_at)
        .bind(session.now_ms)
        .bind(session.now_ms)
        .bind(encode_scopes(session.scopes)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn session_by_refresh_hash(
        &self,
        refresh_hash: &[u8],
        now_ms: i64,
    ) -> Result<Option<SessionRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT s.id, s.user_id, s.device_id, a.username, a.role, s.refresh_expires_at, \
                    s.scopes_json \
             FROM session s JOIN account a ON a.id = s.user_id \
             WHERE s.refresh_token_hash = ? AND s.revoked_at IS NULL \
               AND s.refresh_expires_at > ? AND a.disabled = 0",
        )
        .bind(refresh_hash)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await?;
        row.map(session_from_row).transpose()
    }

    pub async fn account_by_access_hash(
        &self,
        access_hash: &[u8],
        now_ms: i64,
    ) -> Result<Option<AccessRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT a.id, a.username, a.role, s.scopes_json FROM session s \
             JOIN account a ON a.id = s.user_id \
             JOIN device d ON d.id = s.device_id \
             WHERE s.access_token_hash = ? AND s.revoked_at IS NULL \
               AND s.access_expires_at > ? AND a.disabled = 0 AND d.revoked_at IS NULL",
        )
        .bind(access_hash)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(AccessRecord {
                user_id: parse_uuid(row.try_get("id")?)?,
                username: row.try_get("username")?,
                role: parse_role(row.try_get("role")?)?,
                scopes: decode_scopes(row.try_get("scopes_json")?)?,
            })
        })
        .transpose()
    }

    pub async fn account_by_api_token_hash(
        &self,
        token_hash: &[u8],
        now_ms: i64,
    ) -> Result<Option<AccessRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT a.id, a.username, a.role, t.scopes_json, t.last_used_at FROM api_token t \
             JOIN account a ON a.id = t.user_id \
             WHERE t.token_hash = ? AND t.revoked_at IS NULL \
               AND (t.expires_at IS NULL OR t.expires_at > ?) AND a.disabled = 0",
        )
        .bind(token_hash)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let last_used_at: Option<i64> = row.try_get("last_used_at")?;
        let account = AccessRecord {
            user_id: parse_uuid(row.try_get("id")?)?,
            username: row.try_get("username")?,
            role: parse_role(row.try_get("role")?)?,
            scopes: decode_scopes(row.try_get("scopes_json")?)?,
        };
        if last_used_at.is_none_or(|last_used| last_used <= now_ms.saturating_sub(60_000)) {
            let _writer = self.writer_guard().await;
            sqlx::query(
                "UPDATE api_token SET last_used_at = ? WHERE token_hash = ? \
                 AND revoked_at IS NULL AND (last_used_at IS NULL OR last_used_at <= ?)",
            )
            .bind(now_ms)
            .bind(token_hash)
            .bind(now_ms.saturating_sub(60_000))
            .execute(&self.pool)
            .await?;
        }
        Ok(Some(account))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn rotate_session(
        &self,
        session_id: Uuid,
        expected_refresh_hash: &[u8],
        access_hash: &[u8],
        refresh_hash: &[u8],
        access_expires_at: i64,
        refresh_expires_at: i64,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query(
            "UPDATE session SET access_token_hash = ?, refresh_token_hash = ?, \
                access_expires_at = ?, refresh_expires_at = ?, last_used_at = ? \
             WHERE id = ? AND refresh_token_hash = ? AND revoked_at IS NULL \
               AND refresh_expires_at > ?",
        )
        .bind(access_hash)
        .bind(refresh_hash)
        .bind(access_expires_at)
        .bind(refresh_expires_at)
        .bind(now_ms)
        .bind(session_id.to_string())
        .bind(expected_refresh_hash)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn revoke_session_by_access_hash(
        &self,
        access_hash: &[u8],
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query(
            "UPDATE session SET revoked_at = ? \
             WHERE access_token_hash = ? AND revoked_at IS NULL",
        )
        .bind(now_ms)
        .bind(access_hash)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn revoke_session_by_refresh_hash(
        &self,
        refresh_hash: &[u8],
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query(
            "UPDATE session SET revoked_at = ? \
             WHERE refresh_token_hash = ? AND revoked_at IS NULL",
        )
        .bind(now_ms)
        .bind(refresh_hash)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Issues a token and returns its record.
    ///
    /// `RETURNING` rather than an insert followed by a read: the caller wants
    /// the row it just wrote, and looking it up afterwards meant listing every
    /// token the account holds to find one whose id was already known.
    pub async fn create_api_token(
        &self,
        user_id: Uuid,
        name: &str,
        token_hash: &[u8],
        scopes: &[String],
        now_ms: i64,
    ) -> Result<ApiTokenRecord, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let id = Uuid::new_v4();
        let row = sqlx::query(
            "INSERT INTO api_token (id, user_id, name, token_hash, scopes_json, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             RETURNING id, name, scopes_json, expires_at, created_at, last_used_at, revoked_at",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(name.trim())
        .bind(token_hash)
        .bind(encode_scopes(scopes)?)
        .bind(now_ms)
        .fetch_one(&self.pool)
        .await?;
        api_token_from_row(row)
    }

    pub async fn revoke_api_token_by_hash(
        &self,
        token_hash: &[u8],
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query(
            "UPDATE api_token SET revoked_at = ? \
             WHERE token_hash = ? AND revoked_at IS NULL",
        )
        .bind(now_ms)
        .bind(token_hash)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// The tokens issued to one account, newest first, secrets excluded.
    ///
    /// `token_hash` is never projected. The secret exists once, in the creation
    /// response; a listing that could return it would make every read of this
    /// route as sensitive as issuing a new one.
    pub async fn api_tokens_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<ApiTokenRecord>, sqlx::Error> {
        sqlx::query(
            "SELECT id, name, scopes_json, expires_at, created_at, last_used_at, revoked_at \
             FROM api_token WHERE user_id = ? ORDER BY created_at DESC, id",
        )
        .bind(user_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(api_token_from_row)
        .collect()
    }

    /// Revokes one token of one account.
    ///
    /// The account is part of the condition rather than checked beforehand, so
    /// an administrator cannot revoke a token by naming the wrong owner, and a
    /// token already revoked answers the same as one that never existed: the
    /// caller asked for it to stop working, and it does not.
    pub async fn revoke_api_token(
        &self,
        actor_id: Uuid,
        user_id: Uuid,
        token_id: Uuid,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE api_token SET revoked_at = ? \
             WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
        )
        .bind(now_ms)
        .bind(token_id.to_string())
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
        let revoked = result.rows_affected() == 1;
        if revoked {
            insert_audit(
                &mut tx,
                Some(actor_id),
                "api_token.revoked",
                Some(token_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(revoked)
    }

    pub async fn create_library(
        &self,
        owner_id: Uuid,
        name: &str,
        root_path: &Path,
        visibility: LibraryVisibility,
        now_ms: i64,
    ) -> Result<Uuid, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        let id = Uuid::new_v4();
        let root_path = root_path.to_string_lossy();
        sqlx::query(
            "INSERT INTO library \
             (id, owner_user_id, name, root_path, visibility, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(owner_id.to_string())
        .bind(name.trim())
        .bind(root_path.as_ref())
        .bind(visibility.as_str())
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO library_member (library_id, user_id, role, created_at) VALUES (?, ?, 'owner', ?)",
        )
        .bind(id.to_string())
        .bind(owner_id.to_string())
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        insert_audit(&mut tx, Some(owner_id), "library.created", Some(id), now_ms).await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn add_library_member(
        &self,
        actor_id: Uuid,
        library_id: Uuid,
        user_id: Uuid,
        role: LibraryRole,
        now_ms: i64,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO library_member (library_id, user_id, role, created_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT (library_id, user_id) DO UPDATE SET role = excluded.role",
        )
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .bind(role.as_str())
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            Some(actor_id),
            "library.member_set",
            Some(library_id),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn remove_library_member(
        &self,
        actor_id: Uuid,
        library_id: Uuid,
        user_id: Uuid,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        let removed = sqlx::query(
            "DELETE FROM library_member \
             WHERE library_id=? AND user_id=? AND role!='owner'",
        )
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if removed {
            insert_audit(
                &mut tx,
                Some(actor_id),
                "library.member_removed",
                Some(library_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(removed)
    }

    pub async fn set_subsonic_credential(
        &self,
        actor_id: Uuid,
        user_id: Uuid,
        secret: &EncryptedSecret,
        api_key_hash: &[u8],
        now_ms: i64,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO subsonic_credential \
             (user_id, password_nonce, password_ciphertext, api_key_hash, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (user_id) DO UPDATE SET \
               password_nonce = excluded.password_nonce, \
               password_ciphertext = excluded.password_ciphertext, \
               api_key_hash = excluded.api_key_hash, updated_at = excluded.updated_at",
        )
        .bind(user_id.to_string())
        .bind(secret.nonce.as_slice())
        .bind(&secret.ciphertext)
        .bind(api_key_hash)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            Some(actor_id),
            "subsonic.credential_set",
            Some(user_id),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn revoke_subsonic_credential(
        &self,
        actor_id: Uuid,
        user_id: Uuid,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query("DELETE FROM subsonic_credential WHERE user_id = ?")
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?;
        insert_audit(
            &mut tx,
            Some(actor_id),
            "subsonic.credential_revoked",
            Some(user_id),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(result.rows_affected() == 1)
    }
}

fn account_from_row(row: sqlx::sqlite::SqliteRow) -> Result<AccountRecord, sqlx::Error> {
    Ok(AccountRecord {
        id: parse_uuid(row.try_get("id")?)?,
        username: row.try_get("username")?,
        password_hash: row.try_get("password_hash")?,
        role: parse_role(row.try_get("role")?)?,
        disabled: row.try_get::<i64, _>("disabled")? != 0,
    })
}

fn session_from_row(row: sqlx::sqlite::SqliteRow) -> Result<SessionRecord, sqlx::Error> {
    Ok(SessionRecord {
        id: parse_uuid(row.try_get("id")?)?,
        user_id: parse_uuid(row.try_get("user_id")?)?,
        device_id: parse_uuid(row.try_get("device_id")?)?,
        username: row.try_get("username")?,
        role: parse_role(row.try_get("role")?)?,
        refresh_expires_at: row.try_get("refresh_expires_at")?,
        scopes: decode_scopes(row.try_get("scopes_json")?)?,
    })
}

/// Scopes are stored as a JSON array so a credential's limit travels as one
/// column, on the three tables that can carry one: `api_token`, and now
/// `oauth_authorization` and `session`.
fn encode_scopes(scopes: &[String]) -> Result<String, sqlx::Error> {
    serde_json::to_string(scopes).map_err(|error| sqlx::Error::Encode(error.into()))
}

/// Counterpart of [`encode_scopes`]. An unreadable list is a decode error
/// rather than an empty one: silently reading a corrupt limit as "no limit"
/// would widen a credential exactly where it must not.
fn decode_scopes(value: &str) -> Result<Vec<String>, sqlx::Error> {
    serde_json::from_str(value).map_err(|error| sqlx::Error::Decode(error.into()))
}

fn parse_uuid(value: String) -> Result<Uuid, sqlx::Error> {
    Uuid::parse_str(&value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

fn parse_role(value: String) -> Result<AccountRole, sqlx::Error> {
    AccountRole::from_str(&value).map_err(|error| sqlx::Error::Decode(error.into_boxed_dyn_error()))
}

fn api_token_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ApiTokenRecord, sqlx::Error> {
    Ok(ApiTokenRecord {
        id: parse_uuid(row.try_get("id")?)?,
        name: row.try_get("name")?,
        scopes: decode_scopes(row.try_get("scopes_json")?)?,
        expires_at: row.try_get("expires_at")?,
        created_at: row.try_get("created_at")?,
        last_used_at: row.try_get("last_used_at")?,
        revoked_at: row.try_get("revoked_at")?,
    })
}

async fn insert_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor_id: Option<Uuid>,
    kind: &str,
    subject_id: Option<Uuid>,
    now_ms: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_event (actor_user_id, kind, subject_id, occurred_at) VALUES (?, ?, ?, ?)",
    )
    .bind(actor_id.map(|id| id.to_string()))
    .bind(kind)
    .bind(subject_id.map(|id| id.to_string()))
    .bind(now_ms)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn migrated_database() -> (tempfile::TempDir, Database) {
        let temp = tempfile::tempdir().expect("temporary directory");
        let database = Database::open(&Config::for_data_dir(temp.path().join("data")))
            .await
            .expect("open database");
        database.migrate().await.expect("initial migration");
        (temp, database)
    }

    fn crlf_checksum(version: i64) -> Vec<u8> {
        let migration = MIGRATOR
            .iter()
            .find(|migration| migration.version == version)
            .expect("known migration");
        Sha384::digest(migration.sql.as_str().replace('\n', "\r\n").as_bytes()).to_vec()
    }

    #[tokio::test]
    async fn migrate_accepts_checksums_from_legacy_windows_line_endings() {
        let (_temp, database) = migrated_database().await;
        let versions = [20260806000000_i64, 20260806120000_i64];

        let writer = database.writer_guard().await;
        for version in versions {
            let legacy_checksum = crlf_checksum(version);
            let current_checksum = MIGRATOR
                .iter()
                .find(|migration| migration.version == version)
                .expect("known migration")
                .checksum
                .as_ref();
            assert_ne!(legacy_checksum, current_checksum);
            sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
                .bind(legacy_checksum)
                .bind(version)
                .execute(&database.pool)
                .await
                .expect("install legacy checksum");
        }
        drop(writer);

        database.migrate().await.expect("normalize then migrate");

        for version in versions {
            let expected = MIGRATOR
                .iter()
                .find(|migration| migration.version == version)
                .expect("known migration")
                .checksum
                .as_ref();
            let stored: Vec<u8> =
                sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
                    .bind(version)
                    .fetch_one(&database.pool)
                    .await
                    .expect("read normalized checksum");
            assert_eq!(stored, expected);
        }
    }

    #[tokio::test]
    async fn migrate_still_rejects_an_unrelated_checksum_mismatch() {
        let (_temp, database) = migrated_database().await;
        let version = 20260806000000_i64;
        let writer = database.writer_guard().await;
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(vec![0_u8; 48])
            .bind(version)
            .execute(&database.pool)
            .await
            .expect("install invalid checksum");
        drop(writer);

        let error = database
            .migrate()
            .await
            .expect_err("unrelated checksum must remain invalid");
        assert!(
            error
                .to_string()
                .contains("previously applied but has been modified"),
            "unexpected error: {error:#}"
        );
    }
}
