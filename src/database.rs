//! SQLite v2 connection, migrations and tenant-safe foundation repositories.

use std::{path::Path, str::FromStr, sync::Arc};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha384};
use sqlx::{
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Connection, Row, SqliteConnection, SqlitePool,
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{config::Config, security::EncryptedSecret};

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations-v2");

/// What this binary's migrations find in a database.
///
/// Asked before anything is applied, because the key commands promise a
/// reading that changes nothing and applying a migration is a change. It is
/// also what lets [`Database::key_material`] count without guarding each
/// table: `Current` means every migration this binary carries has run, so
/// every table it creates is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaState {
    /// No migration has run. The file is not a WaveFlow database yet.
    Empty,
    /// Every migration this binary carries has run.
    Current,
    /// This binary carries migrations the database has not run.
    Behind(usize),
}
/// What an instance key protects, counted from the database.
///
/// Four numbers rather than a sentence, because the sentence is what the
/// documentation already carries and it is not what an operator needs at the
/// moment of deciding. The third is the one nobody expects: discarding the
/// scrobbling links discards the listens queued behind them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyMaterial {
    pub subsonic_credentials: i64,
    pub scrobble_links: i64,
    pub queued_listens: i64,
    pub shares: i64,
}

/// What to do with the values the previous key protected.
///
/// Two variants, and two commands above them rather than one command and a
/// flag: one of these preserves exactly what the other destroys, and that is
/// not a detail of a single operation.
pub enum Rekey<'a> {
    /// The old key is in hand. Every ciphertext is read and written back.
    Reencrypt(&'a crate::security::SecretBox),
    /// The old key is gone. Nothing can be read, so the rows go.
    Discard,
}

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

    /// Whether the role may correct a track's tags.
    ///
    /// The same two roles as [`Self::may_scan`], and not by coincidence: a
    /// correction is written once and read by every member, so it asks the same
    /// trust as spending the owner's disk. Named apart because the two rules
    /// answer different questions and one of them may move.
    pub fn may_write_metadata(self) -> bool {
        matches!(self, Self::Owner | Self::Manager)
    }

    /// Whether the role may hand the library a new file.
    ///
    /// The same two roles again, and deliberately so: a rescan spends the
    /// owner's disk, and so does receiving a file — permanently, this time.
    /// It is never sufficient on its own. `library.accepts_uploads` decides
    /// where, and a role that may upload into an opted-in library may upload
    /// into no other.
    pub fn may_upload(self) -> bool {
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

/// Whether an account holds a dedicated Subsonic credential, and since when.
///
/// **The password is not in this shape and cannot be put into it**, because
/// the query behind it never selects the columns that hold one. A shape that
/// does not carry a secret cannot leak one by a later careless edit of the
/// handler.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct SubsonicCredentialStatus {
    /// When this account's credential was first set.
    pub created_at: i64,
    /// When it last changed — the same instant a rotation reports back.
    pub updated_at: i64,
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
/// One API token to write, gathered rather than passed as seven arguments.
///
/// `actor_id` is who registered it, which the audit entry records and which
/// is not always the owner: an administrator provisioning a service account is
/// the actor, and the account is the subject.
pub struct NewApiToken<'a> {
    pub actor_id: Uuid,
    pub user_id: Uuid,
    pub name: &'a str,
    pub token_hash: &'a [u8],
    pub scopes: &'a [String],
    pub expires_at: Option<i64>,
}

pub struct AccessRecord {
    pub user_id: Uuid,
    pub username: String,
    pub role: AccountRole,
    /// The scopes of the API token this request arrived on. Empty for a
    /// session or an OAuth grant, which carry the account's full authority,
    /// and for a token issued without any.
    pub scopes: Vec<String>,
    /// When the API token this request arrived on stops working, if it is one
    /// and if it has a deadline.
    ///
    /// `None` for a session, which is a different kind of credential: it is
    /// short-lived and refreshes, and authorising something longer-lived than
    /// itself is part of what it is for. `None` for a token issued without an
    /// expiry too — there is simply nothing to bound by.
    pub credential_expires_at: Option<i64>,
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
            .acquire_timeout(config.db_acquire_timeout)
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
        // The connection is detached rather than returned to the pool: the
        // migration turns foreign-key enforcement off on whatever connection
        // it runs on, and a connection left unenforced is one every later
        // query would inherit.
        let mut connection = self.pool.acquire().await?.detach();
        let outcome = Self::migrate_on(&mut connection).await;
        connection.close().await.ok();
        outcome
    }

    /// Every pending migration, on a connection nobody else is using.
    ///
    /// Two callers, and the exclusivity is what they have in common rather
    /// than how they get it: [`Self::migrate`] detaches a connection behind
    /// the writer gate, while the offline key commands hold the database file
    /// itself (see [`Self::open_alone`]). An upgrade that two writers shared
    /// would be a different thing entirely, which is why neither path runs
    /// this on a pool.
    ///
    /// The caller owns the connection afterwards, foreign keys and all — see
    /// below.
    pub(crate) async fn migrate_on(connection: &mut SqliteConnection) -> anyhow::Result<()> {
        let normalized = Self::normalize_legacy_crlf_migration_checksums(connection)
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

        // Migrations run with enforcement off, which is the procedure SQLite
        // documents for a table rebuild and the only one open to us here.
        //
        // A rebuild drops the old parent, and SQLite runs an implicit
        // `DELETE FROM` before dropping a table. Two children of `album` and
        // `artist` carry `ON DELETE RESTRICT`, so that delete is a constraint
        // violation on any database holding a single track. Deferring does not
        // save it: `defer_foreign_keys` moves the check to the commit, but the
        // counter the drop raised is never cleared, so the commit fails while
        // `PRAGMA foreign_key_check` reports nothing wrong — and an empty
        // database migrates cleanly, which is why a suite that only ever
        // migrates fresh databases cannot see this at all.
        //
        // A migration file cannot turn enforcement off for itself: `PRAGMA
        // foreign_keys` is a no-op inside a transaction and every migration
        // runs in one. So the runner does it, and answers for it below — and
        // leaves it off, so a caller that keeps the connection has to put it
        // back before writing anything a foreign key should govern.
        Self::run_migrations_unenforced(connection).await
    }

    /// Runs every pending migration on a connection with foreign keys off,
    /// then answers for what enforcement would have caught.
    ///
    /// The check is `PRAGMA foreign_key_check` over the whole database rather
    /// than per migration: a rebuild that dropped a parent row instead of
    /// carrying it shows up here as an orphan, and refusing to start is the
    /// only safe answer — the alternative is serving a catalogue whose
    /// relations silently lost rows.
    async fn run_migrations_unenforced(connection: &mut SqliteConnection) -> anyhow::Result<()> {
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *connection)
            .await
            .map_err(|error| anyhow::anyhow!("could not suspend foreign keys: {error}"))?;
        MIGRATOR
            .run(&mut *connection)
            .await
            .map_err(|error| anyhow::anyhow!("v2 migration failed: {error}"))?;
        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *connection)
            .await
            .map_err(|error| anyhow::anyhow!("foreign key check failed: {error}"))?;
        if !violations.is_empty() {
            anyhow::bail!(
                "v2 migration left {} broken foreign key reference(s); \
                 the database has not been changed further and needs restoring from backup",
                violations.len()
            );
        }
        Ok(())
    }

    /// Early Windows builds embedded migrations after Git had converted their
    /// LF line endings to CRLF. SQLx hashes the exact bytes, so databases made
    /// by those builds are rejected after `.gitattributes` made the checkout
    /// byte-stable even though the SQL itself is unchanged.
    ///
    /// Only the checksum of the current migration with every LF converted to
    /// CRLF is accepted. Any other mismatch is left untouched for SQLx to
    /// reject normally.
    async fn normalize_legacy_crlf_migration_checksums(
        connection: &mut SqliteConnection,
    ) -> Result<u64, sqlx::Error> {
        let mut tx = connection.begin().await?;
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

    /// Opens the database for an offline operation, refusing to share it.
    ///
    /// `locking_mode=EXCLUSIVE` followed at once by `BEGIN EXCLUSIVE` is the
    /// only question SQLite answers honestly here: a running server keeps
    /// connections open against this file, and the lock cannot be taken while
    /// one of them exists. It then stays taken until this connection closes,
    /// so the refusal is not a check with a gap after it — it is the same lock
    /// the work is then done under.
    ///
    /// That is strictly more than [`Self::writer_guard`] offers. The writer
    /// gate keeps two tasks of *one* process apart; an offline command has one
    /// task and a different hazard, which is the other process.
    ///
    /// The file must already exist. `create_if_missing` would turn "the
    /// database is not where you said" into an empty database bound to a fresh
    /// key, which is the one outcome worse than the error.
    pub async fn open_alone(
        path: &Path,
        busy_timeout: std::time::Duration,
    ) -> anyhow::Result<SqliteConnection> {
        if !path.exists() {
            anyhow::bail!("no SQLite database at {}", path.display());
        }
        let mut connection = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false)
                .foreign_keys(true)
                .journal_mode(SqliteJournalMode::Wal)
                .busy_timeout(busy_timeout),
        )
        .await
        .map_err(|error| anyhow::anyhow!("sqlite connect failed: {error}"))?;
        sqlx::query("PRAGMA locking_mode = EXCLUSIVE")
            .execute(&mut connection)
            .await
            .map_err(|error| anyhow::anyhow!("could not ask for an exclusive lock: {error}"))?;
        sqlx::query("BEGIN EXCLUSIVE")
            .execute(&mut connection)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "{} is open in another process; stop the server before running this",
                    path.display()
                )
            })?;
        sqlx::query("COMMIT")
            .execute(&mut connection)
            .await
            .map_err(|error| anyhow::anyhow!("could not take the database: {error}"))?;
        Ok(connection)
    }

    /// Where this database stands against the migrations this binary carries.
    pub async fn schema_state(connection: &mut SqliteConnection) -> anyhow::Result<SchemaState> {
        let has_table = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master \
             WHERE type = 'table' AND name = '_sqlx_migrations')",
        )
        .fetch_one(&mut *connection)
        .await?;
        if has_table == 0 {
            return Ok(SchemaState::Empty);
        }
        let applied = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM _sqlx_migrations WHERE success = TRUE",
        )
        .fetch_all(&mut *connection)
        .await?;
        if applied.is_empty() {
            return Ok(SchemaState::Empty);
        }
        let pending = MIGRATOR
            .iter()
            .filter(|migration| !applied.contains(&migration.version))
            .count();
        Ok(if pending == 0 {
            SchemaState::Current
        } else {
            SchemaState::Behind(pending)
        })
    }

    /// The fingerprint this database was bound to, if it has been bound.
    ///
    /// Absent on a database no server has ever opened. Present and different
    /// is what aborts a boot, and what tells the two key commands apart.
    pub async fn stored_key_fingerprint(
        connection: &mut SqliteConnection,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT key_fingerprint FROM instance_metadata WHERE singleton=1",
        )
        .fetch_optional(&mut *connection)
        .await?)
    }

    /// What the instance key protects, counted rather than described.
    ///
    /// The operator is about to be told what a key change costs, and a count
    /// read from their own database is the only version of that worth
    /// printing.
    pub async fn key_material(connection: &mut SqliteConnection) -> anyhow::Result<KeyMaterial> {
        let count = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql);
        Ok(KeyMaterial {
            subsonic_credentials: count("SELECT COUNT(*) FROM subsonic_credential")
                .fetch_one(&mut *connection)
                .await?,
            scrobble_links: count("SELECT COUNT(*) FROM scrobble_link")
                .fetch_one(&mut *connection)
                .await?,
            // Every row of the outbox hangs off a link by `ON DELETE CASCADE`,
            // so discarding the links discards these with them. Counted
            // because an operator told "two scrobbling links" would not guess
            // that it also means a few hundred listens nobody has sent yet.
            queued_listens: count("SELECT COUNT(*) FROM scrobble_outbox")
                .fetch_one(&mut *connection)
                .await?,
            shares: count("SELECT COUNT(*) FROM share")
                .fetch_one(&mut *connection)
                .await?,
        })
    }

    /// Moves every value the instance key governs onto a new key.
    ///
    /// One transaction, so a database half under one key and half under
    /// another is not a state this can leave behind. The caller writes the new
    /// key file only after this returns, for the same reason: a key that
    /// arrived before the commit would name a database that never changed.
    ///
    /// Shares are rewritten whichever plan runs, and they are the reason this
    /// is one function rather than two. A share token is *derived* from the
    /// key rather than stored ([`crate::security::SecretBox::derive_share_token`]),
    /// so after a key change the stored hash is one the new key can never
    /// produce: the share is not merely re-addressed, it is unreachable, and
    /// its owner is shown a URL that answers 404. Recomputing the hash is what
    /// makes "shares survive, with a new URL" true rather than hoped for.
    pub async fn rekey(
        connection: &mut SqliteConnection,
        plan: Rekey<'_>,
        new_key: &crate::security::SecretBox,
        new_fingerprint: &[u8],
        now_ms: i64,
    ) -> anyhow::Result<KeyMaterial> {
        // Asked for here rather than taken on trust from the caller. It is a
        // connection flag, `migrate_on` turns it off, and the delete below is
        // the one statement whose correctness depends on it — a cascade that
        // did not fire leaves no error, only rows. A no-op inside a
        // transaction, so it has to come before the `begin`.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *connection)
            .await
            .map_err(|error| anyhow::anyhow!("could not enforce foreign keys: {error}"))?;
        let enforced = sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
            .fetch_one(&mut *connection)
            .await?;
        if enforced != 1 {
            anyhow::bail!("foreign keys are not enforced; nothing has been changed");
        }
        let mut tx = connection.begin().await?;
        let mut moved = KeyMaterial::default();
        match plan {
            Rekey::Reencrypt(old_key) => {
                let credentials = sqlx::query(
                    "SELECT user_id, password_nonce, password_ciphertext FROM subsonic_credential",
                )
                .fetch_all(&mut *tx)
                .await?;
                for row in credentials {
                    let user_id: String = row.try_get("user_id")?;
                    let plaintext = old_key
                        .decrypt(
                            row.try_get::<Vec<u8>, _>("password_nonce")?.as_slice(),
                            row.try_get::<Vec<u8>, _>("password_ciphertext")?.as_slice(),
                        )
                        .map_err(|_| {
                            anyhow::anyhow!(
                                "a stored Subsonic password does not open with this key; \
                                 nothing has been changed"
                            )
                        })?;
                    let sealed = new_key.encrypt(&plaintext)?;
                    sqlx::query(
                        "UPDATE subsonic_credential \
                         SET password_nonce=?, password_ciphertext=?, updated_at=? \
                         WHERE user_id=?",
                    )
                    .bind(sealed.nonce.as_slice())
                    .bind(&sealed.ciphertext)
                    .bind(now_ms)
                    .bind(&user_id)
                    .execute(&mut *tx)
                    .await?;
                    moved.subsonic_credentials += 1;
                }

                // The `unlinked` generations included: they keep their
                // ciphertext, and a past authorisation left unreadable would
                // be a row this server can no longer account for.
                let links = sqlx::query(
                    "SELECT id, credential_nonce, credential_ciphertext FROM scrobble_link",
                )
                .fetch_all(&mut *tx)
                .await?;
                for row in links {
                    let id: String = row.try_get("id")?;
                    let plaintext = old_key
                        .decrypt(
                            row.try_get::<Vec<u8>, _>("credential_nonce")?.as_slice(),
                            row.try_get::<Vec<u8>, _>("credential_ciphertext")?
                                .as_slice(),
                        )
                        .map_err(|_| {
                            anyhow::anyhow!(
                                "a stored scrobbling secret does not open with this key; \
                                 nothing has been changed"
                            )
                        })?;
                    let sealed = new_key.encrypt(&plaintext)?;
                    sqlx::query(
                        "UPDATE scrobble_link \
                         SET credential_nonce=?, credential_ciphertext=?, updated_at=? \
                         WHERE id=?",
                    )
                    .bind(sealed.nonce.as_slice())
                    .bind(&sealed.ciphertext)
                    .bind(now_ms)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                    moved.scrobble_links += 1;
                }
            }
            Rekey::Discard => {
                // The outbox goes with the links, by `ON DELETE CASCADE`
                // rather than by a statement here — which only holds with
                // enforcement on, and `migrate_on` leaves it off. Counted
                // before the delete and asserted against afterwards, because
                // a cascade that silently did not fire would leave the number
                // printed to the operator as the only trace of it.
                moved.queued_listens = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM scrobble_outbox \
                     WHERE link_id IN (SELECT id FROM scrobble_link)",
                )
                .fetch_one(&mut *tx)
                .await?;
                moved.subsonic_credentials = sqlx::query("DELETE FROM subsonic_credential")
                    .execute(&mut *tx)
                    .await?
                    .rows_affected() as i64;
                moved.scrobble_links = sqlx::query("DELETE FROM scrobble_link")
                    .execute(&mut *tx)
                    .await?
                    .rows_affected() as i64;
                let orphans = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM scrobble_outbox")
                    .fetch_one(&mut *tx)
                    .await?;
                if orphans > 0 {
                    anyhow::bail!(
                        "{orphans} queued listen(s) outlived the links they belong to; \
                         foreign keys were not enforced and nothing has been changed"
                    );
                }
            }
        }

        for id in sqlx::query_scalar::<_, String>("SELECT id FROM share")
            .fetch_all(&mut *tx)
            .await?
        {
            let share = Uuid::parse_str(&id)
                .map_err(|_| anyhow::anyhow!("a share carries an identifier that is not a UUID"))?;
            let token = new_key.derive_share_token(share);
            // `updated_at` is left alone on purpose. The share did not change;
            // the key that names it did, and bumping the column would hand
            // every synchronising client a modification with no journal entry
            // behind it.
            sqlx::query("UPDATE share SET token_hash=? WHERE id=?")
                .bind(crate::security::token_hash(&token).as_slice())
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            moved.shares += 1;
        }

        sqlx::query(
            "INSERT INTO instance_metadata (singleton, key_fingerprint, created_at) \
             VALUES (1, ?, ?) \
             ON CONFLICT (singleton) DO UPDATE SET key_fingerprint = excluded.key_fingerprint",
        )
        .bind(new_fingerprint)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(moved)
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
                // A session, not an API token: see the field.
                credential_expires_at: None,
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
            "SELECT a.id, a.username, a.role, t.scopes_json, t.last_used_at, \
                    t.expires_at FROM api_token t \
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
            credential_expires_at: row.try_get("expires_at")?,
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
        token: NewApiToken<'_>,
        now_ms: i64,
    ) -> Result<ApiTokenRecord, sqlx::Error> {
        let NewApiToken {
            actor_id,
            user_id,
            name,
            token_hash,
            scopes,
            expires_at,
        } = token;
        let _writer = self.writer_guard().await;
        let id = Uuid::new_v4();
        // `expires_at` has been a column, and honoured by the lookup, since
        // the table existed — and never written. Nothing minted a bounded
        // token, so every one of them outlived whatever asked for it.
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "INSERT INTO api_token \
               (id, user_id, name, token_hash, scopes_json, expires_at, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             RETURNING id, name, scopes_json, expires_at, created_at, last_used_at, revoked_at",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(name.trim())
        .bind(token_hash)
        .bind(encode_scopes(scopes)?)
        .bind(expires_at)
        .bind(now_ms)
        .fetch_one(&mut *tx)
        .await?;
        // In the same transaction as the row it describes, the way the
        // revocation beside it already is. Revoking a credential left an audit
        // entry and registering one left none, which is the wrong half to
        // record: it became an act an account can perform for itself without
        // an administrator anywhere, so it is exactly the event an operator
        // would want to find afterwards.
        insert_audit(
            &mut tx,
            Some(actor_id),
            "api_token.created",
            Some(id),
            now_ms,
        )
        .await?;
        tx.commit().await?;
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

    /// Opens or closes a library to receiving files.
    ///
    /// Audited, like every other change to what a library permits. The role a
    /// member holds says who may upload; this says whether the library takes
    /// anything at all, and it is the operator's call rather than a member's —
    /// which is why it arrives through the CLI and not through the API.
    pub async fn set_library_accepts_uploads(
        &self,
        actor_id: Uuid,
        library_id: Uuid,
        accepts: bool,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE library SET accepts_uploads=? WHERE id=?")
            .bind(i64::from(accepts))
            .bind(library_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
        if changed {
            insert_audit(
                &mut tx,
                Some(actor_id),
                if accepts {
                    "library.uploads_opened"
                } else {
                    "library.uploads_closed"
                },
                Some(library_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    /// Opens or closes a library to receiving canvases.
    ///
    /// A second door rather than the same one, and the reasoning is in
    /// `20260830010000_library_accepts_canvas.sql`: a read-only server that
    /// refuses to grow in audio may still want a few hundred kilobytes of video
    /// loop, and that is the most common installation there is.
    ///
    /// Audited and CLI-only, exactly like the upload flag. The role a member
    /// holds says who may place one; this says whether the library takes any at
    /// all, and that is the operator's call rather than a member's.
    pub async fn set_library_accepts_canvas(
        &self,
        actor_id: Uuid,
        library_id: Uuid,
        accepts: bool,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE library SET accepts_canvas=? WHERE id=?")
            .bind(i64::from(accepts))
            .bind(library_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
        if changed {
            insert_audit(
                &mut tx,
                Some(actor_id),
                if accepts {
                    "library.canvas_opened"
                } else {
                    "library.canvas_closed"
                },
                Some(library_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
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
        let revoked = result.rows_affected() == 1;
        // Only when something was removed. The entry was unconditional, and
        // the service answers 404 after writing one, so the trail recorded
        // revocations that had not happened. That was cosmetic while an
        // administrator was the only caller; the self-scoped route makes it
        // reachable by any account, in a loop, which turns a wrong line in a
        // history into a way to fill one. `rotate_subsonic_credential` guards
        // the same way, for the same reason.
        if revoked {
            insert_audit(
                &mut tx,
                Some(actor_id),
                "subsonic.credential_revoked",
                Some(user_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(revoked)
    }

    /// Whether this account holds a Subsonic credential, and since when.
    ///
    /// The projection is the two timestamps and nothing else. The password
    /// columns are not read, so no edit of a caller can forward what this
    /// never fetched.
    pub async fn subsonic_credential_status(
        &self,
        user_id: Uuid,
    ) -> Result<Option<SubsonicCredentialStatus>, sqlx::Error> {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT created_at, updated_at FROM subsonic_credential WHERE user_id = ?",
        )
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map(|row| {
            row.map(|(created_at, updated_at)| SubsonicCredentialStatus {
                created_at,
                updated_at,
            })
        })
    }

    /// Replaces the password and the API key of a credential that already
    /// exists, and answers whether one did.
    ///
    /// **An `UPDATE`, where `set_subsonic_credential` upserts, and that is the
    /// whole difference between the two.** This one cannot bring a credential
    /// into being: an account holding none cannot use the Subsonic facade at
    /// all, so creating one does not narrow a door already open — it opens
    /// one, which is the operator's decision and not this path's. Checking
    /// first and then upserting would be the same decision taken racily;
    /// `WHERE user_id = ?` takes it in the statement, where two callers cannot
    /// interleave.
    ///
    /// The API key moves with the password. Leaving it would rotate half a
    /// credential: a leaked `wfsk_` key authenticates on its own, so a
    /// rotation that kept it would answer "my key has leaked" with a new
    /// password and the same working key.
    pub async fn rotate_subsonic_credential(
        &self,
        actor_id: Uuid,
        user_id: Uuid,
        secret: &EncryptedSecret,
        api_key_hash: &[u8],
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE subsonic_credential SET \
               password_nonce = ?, password_ciphertext = ?, api_key_hash = ?, updated_at = ? \
             WHERE user_id = ?",
        )
        .bind(secret.nonce.as_slice())
        .bind(&secret.ciphertext)
        .bind(api_key_hash)
        .bind(now_ms)
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
        let rotated = result.rows_affected() == 1;
        // Only when something changed. An audit trail that records attempts
        // the database refused would read as a history of rotations that never
        // happened.
        if rotated {
            insert_audit(
                &mut tx,
                Some(actor_id),
                "subsonic.credential_rotated",
                Some(user_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(rotated)
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

/// RFC-011's cache, which [`crate::enrichment`] fills and reads.
///
/// No user id and no tenancy anywhere in it: what a provider says about a name
/// is the same whoever asks, and nothing here is account data (decision 4).
/// Tenancy applies where a cached name meets the catalogue, which is the
/// reader's query, not these.
impl Database {
    /// Every provider's row for one canonical name, in no particular order.
    ///
    /// A row from a provider this build does not know — written by a later
    /// version the instance was downgraded from — is passed over rather than
    /// refused: it is a fact about a provider nobody configured here.
    pub async fn metadata_artist_rows(
        &self,
        name_canonical: &str,
    ) -> Result<Vec<crate::enrichment::CachedArtist>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT provider, outcome, bio_summary, bio_full, image_hash, fetched_at, expires_at \
             FROM metadata_artist WHERE name_canonical = ?",
        )
        .bind(name_canonical)
        .fetch_all(&self.pool)
        .await?;
        let mut cached = Vec::with_capacity(rows.len());
        for row in rows {
            let provider: String = row.try_get("provider")?;
            let Some(provider) = crate::enrichment::Provider::parse(&provider) else {
                continue;
            };
            let outcome: String = row.try_get("outcome")?;
            let outcome = crate::enrichment::Outcome::parse(&outcome).ok_or_else(|| {
                sqlx::Error::Decode(format!("unknown enrichment outcome {outcome:?}").into())
            })?;
            cached.push(crate::enrichment::CachedArtist {
                provider,
                outcome,
                bio_summary: row.try_get("bio_summary")?,
                bio_full: row.try_get("bio_full")?,
                image_hash: row.try_get("image_hash")?,
                fetched_at: row.try_get("fetched_at")?,
                expires_at: row.try_get("expires_at")?,
            });
        }
        Ok(cached)
    }

    /// One provider's ranked neighbours for one canonical name, best first.
    pub async fn metadata_artist_similar(
        &self,
        provider: crate::enrichment::Provider,
        name_canonical: &str,
    ) -> Result<Vec<crate::enrichment::CachedSimilar>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT similar_name, similar_canonical, score FROM metadata_artist_similar \
             WHERE provider = ? AND name_canonical = ? ORDER BY rank",
        )
        .bind(provider.as_str())
        .bind(name_canonical)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(crate::enrichment::CachedSimilar {
                    name: row.try_get("similar_name")?,
                    name_canonical: row.try_get("similar_canonical")?,
                    score: row.try_get("score")?,
                })
            })
            .collect()
    }

    /// Links the portrait `hash` to every artist called `name_canonical` that
    /// does not show it yet, emitting one artist event per row it moved.
    ///
    /// Read first without the writer gate, since this runs on the read path
    /// and almost always finds nothing to do. The artwork row is checked in
    /// the same statement as the link: the sweeper may have taken it since
    /// the cache row was read.
    pub(crate) async fn link_artist_portrait(
        &self,
        name_canonical: &str,
        hash: &str,
    ) -> Result<(), sqlx::Error> {
        let unlinked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM artist \
               WHERE canonical_name = ? AND artwork_hash IS NOT ?)",
        )
        .bind(name_canonical)
        .bind(hash)
        .fetch_one(&self.pool)
        .await?;
        if !unlinked {
            return Ok(());
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        link_portrait_in(
            &mut tx,
            name_canonical,
            hash,
            crate::authentication::now_ms(),
        )
        .await?;
        tx.commit().await
    }

    /// Records what a provider answered: its row, its whole neighbour list,
    /// and its portrait — the artwork row and every artist it belongs to —
    /// in one transaction.
    ///
    /// One transaction is what keeps the sweeper from ever seeing the artwork
    /// row unreferenced: it deletes under this same gate, so it runs wholly
    /// before or wholly after. The writer gate is taken here and nowhere
    /// earlier — the requests that produced `answer` are over, and the
    /// portrait's file is already in the store, so no network or file I/O
    /// ever runs under it.
    pub(crate) async fn write_metadata_artist(
        &self,
        provider: crate::enrichment::Provider,
        name_canonical: &str,
        answer: &crate::enrichment::ArtistAnswer,
        fetched_at: i64,
        expires_at: i64,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool.begin().await?;
        if let Some(image) = &answer.image {
            sqlx::query(
                "INSERT INTO artwork (hash, format, source, byte_size, created_at) \
                 VALUES (?, ?, 'deezer', ?, ?) ON CONFLICT (hash) DO NOTHING",
            )
            .bind(&image.hash)
            .bind(image.format)
            .bind(image.byte_size)
            .bind(fetched_at)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO metadata_artist \
                (provider, name_canonical, outcome, bio_summary, bio_full, image_hash, \
                 fetched_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (provider, name_canonical) DO UPDATE SET \
                outcome = excluded.outcome, bio_summary = excluded.bio_summary, \
                bio_full = excluded.bio_full, image_hash = excluded.image_hash, \
                fetched_at = excluded.fetched_at, expires_at = excluded.expires_at",
        )
        .bind(provider.as_str())
        .bind(name_canonical)
        .bind(answer.outcome().as_str())
        .bind(answer.bio_summary.as_deref())
        .bind(answer.bio_full.as_deref())
        .bind(answer.image.as_ref().map(|image| image.hash.as_str()))
        .bind(fetched_at)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        if let Some(image) = &answer.image {
            link_portrait_in(&mut tx, name_canonical, &image.hash, fetched_at).await?;
        }
        sqlx::query(
            "DELETE FROM metadata_artist_similar WHERE provider = ? AND name_canonical = ?",
        )
        .bind(provider.as_str())
        .bind(name_canonical)
        .execute(&mut *tx)
        .await?;
        for (rank, similar) in answer.similar.iter().enumerate() {
            sqlx::query(
                "INSERT INTO metadata_artist_similar \
                    (provider, name_canonical, rank, similar_name, similar_canonical, score) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(provider.as_str())
            .bind(name_canonical)
            .bind(rank as i64)
            .bind(&similar.name)
            .bind(&similar.name_canonical)
            .bind(similar.score)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }
}

/// Points every artist called `name_canonical` at the portrait `hash`, and
/// records one artist event for each row that actually moved — RFC-007
/// decision 6's rule for albums, which an artist now follows: a refresh that
/// fetched the same picture changes nothing and says nothing.
///
/// Artist rows are per library, and so are events, so each moved row carries
/// its own library. A row whose artwork row is gone is not linked: the cache
/// may name a hash the sweeper has since taken.
async fn link_portrait_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    name_canonical: &str,
    hash: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    let moved = sqlx::query(
        "UPDATE artist SET artwork_hash = ?, updated_at = ? \
         WHERE canonical_name = ? AND artwork_hash IS NOT ? \
           AND EXISTS (SELECT 1 FROM artwork WHERE artwork.hash = ?) \
         RETURNING id, library_id",
    )
    .bind(hash)
    .bind(now)
    .bind(name_canonical)
    .bind(hash)
    .bind(hash)
    .fetch_all(&mut **tx)
    .await?;
    for row in moved {
        let id: String = row.try_get("id")?;
        let library_id: String = row.try_get("library_id")?;
        let parse =
            |value: &str| Uuid::parse_str(value).map_err(|error| sqlx::Error::Decode(error.into()));
        crate::catalog::record_library_event(
            tx,
            crate::catalog::LibraryChange {
                library_id: parse(&library_id)?,
                entity_type: "artist",
                entity_id: parse(&id)?,
                action: "upsert",
                payload: serde_json::json!({}),
                changed_at: now,
                origin_device_id: None,
            },
        )
        .await?;
    }
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

    /// Every other migration test starts from an empty database, and an empty
    /// database is the one case a table rebuild cannot fail on: dropping the
    /// old parent cascades into nothing and restricts nothing. The rebuild in
    /// `20260823010000_participants` therefore passed the whole suite and then
    /// refused to open the first database that held a single track.
    ///
    /// So this one fills the catalogue in first, at the schema version just
    /// before the rebuild, and migrates from there.
    ///
    /// It is pinned to that one boundary on purpose — it was written for that
    /// defect. The general question, whether a database at a *released*
    /// schema still upgrades with its rows and still answers through the
    /// services, is the `migrations` integration target (#223); add a seed
    /// row there rather than here when a new table has to survive an upgrade.
    #[tokio::test]
    async fn a_populated_database_survives_a_table_rebuild() {
        use sqlx::migrate::Migrate;

        /// The last migration before `artist` and `album` are rebuilt.
        const BEFORE_THE_REBUILD: i64 = 20260823000000;

        let temp = tempfile::tempdir().expect("temporary directory");
        let database = Database::open(&Config::for_data_dir(temp.path().join("data")))
            .await
            .expect("open database");

        let mut connection = database.pool.acquire().await.expect("connection");
        connection
            .ensure_migrations_table(&MIGRATOR.table_name)
            .await
            .expect("migrations table");
        for migration in MIGRATOR
            .iter()
            .filter(|migration| migration.version <= BEFORE_THE_REBUILD)
        {
            connection
                .apply(&MIGRATOR.table_name, migration)
                .await
                .expect("earlier migration");
        }

        // A track, its album and its artist, plus the credit relating them:
        // one row in each table the rebuild touches or cascades from. The
        // identifiers are UUIDs because every real row's is: a fixture shaped
        // unlike production is one a production-shaped bug can pass through.
        const ACCOUNT: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000001";
        const LIBRARY: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000002";
        const ARTIST: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000003";
        const ALBUM: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000004";
        const TRACK: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000005";
        let digest = "0".repeat(64);
        for (statement, binds) in [
            (
                "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
                   VALUES (?, 'owner', 'x', 'admin', 0, 0)",
                vec![ACCOUNT],
            ),
            (
                "INSERT INTO library (id, owner_user_id, name, root_path, visibility, \
                                      created_at, updated_at) \
                   VALUES (?, ?, 'Library', '/music', 'private', 0, 0)",
                vec![LIBRARY, ACCOUNT],
            ),
            (
                "INSERT INTO artist (id, library_id, name, canonical_name, created_at, \
                                     updated_at) \
                   VALUES (?, ?, 'Nova Kern', 'nova kern', 0, 0)",
                vec![ARTIST, LIBRARY],
            ),
            (
                "INSERT INTO album (id, library_id, title, canonical_title, identity_key, \
                                    album_artist_id, album_artist_name, created_at, updated_at) \
                   VALUES (?, ?, 'Convergence', 'convergence', 'key', ?, 'Nova Kern', 0, 0)",
                vec![ALBUM, LIBRARY, ARTIST],
            ),
            (
                "INSERT INTO track (id, library_id, album_id, relative_path, file_size, \
                                    file_modified_at, quick_hash, full_hash, title, \
                                    duration_ms, created_at, updated_at) \
                   VALUES (?, ?, ?, 'a.flac', 1, 0, ?, ?, 'Zenith', 1000, 0, 0)",
                vec![TRACK, LIBRARY, ALBUM, digest.as_str(), digest.as_str()],
            ),
            (
                "INSERT INTO track_artist (track_id, artist_id, library_id, position) \
                   VALUES (?, ?, ?, 0)",
                vec![TRACK, ARTIST, LIBRARY],
            ),
        ] {
            let mut query = sqlx::query(statement);
            for bind in binds {
                query = query.bind(bind.to_owned());
            }
            query.execute(&mut *connection).await.expect("seed row");
        }
        drop(connection);

        database
            .migrate()
            .await
            .expect("a database with a catalogue migrates");

        // The rebuild has to carry its rows, not merely survive: an implicit
        // `DELETE FROM` before a drop empties children silently, and a
        // migration that lost the credits would still report success.
        let mut connection = database.pool.acquire().await.expect("connection");
        for (table, query) in [
            ("artist", "SELECT COUNT(*) FROM artist"),
            ("album", "SELECT COUNT(*) FROM album"),
            ("track", "SELECT COUNT(*) FROM track"),
            (
                "track_participant",
                "SELECT COUNT(*) FROM track_participant",
            ),
        ] {
            let count: i64 = sqlx::query_scalar(query)
                .fetch_one(&mut *connection)
                .await
                .expect("count");
            assert_eq!(count, 1, "{table} lost its rows to the rebuild");
        }
        let role: String =
            sqlx::query_scalar("SELECT role FROM track_participant WHERE track_id = '6f1d1b6e-0f1a-4a3d-9c21-000000000005'")
                .fetch_one(&mut *connection)
                .await
                .expect("carried credit");
        assert_eq!(role, "artist", "the carried credit kept its meaning");

        // And enforcement, suspended for the rebuild, is answered for.
        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *connection)
            .await
            .expect("foreign key check");
        assert!(
            violations.is_empty(),
            "the rebuilt schema left {} broken reference(s)",
            violations.len()
        );
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

    /// A retry queued before `retried_at` existed still counts as the joker
    /// spent, once the column arrives.
    ///
    /// Retried entries exist on every database linked since #191, and until
    /// `20260914000000` the only trace of one was a second row naming it
    /// through `retry_of`. Adding the column empty would leave every one of
    /// those originals unanswered again — the migration would reintroduce, on
    /// the only servers that have anything to reintroduce it to, exactly the
    /// defect it exists to correct. Reading it on a database populated at the
    /// schema version before the column is what says otherwise; a rereading of
    /// the SQL is not.
    #[tokio::test]
    async fn a_retry_queued_before_the_column_counts_as_the_joker_spent() {
        use sqlx::migrate::Migrate;

        /// The last migration before `scrobble_outbox.retried_at`.
        const BEFORE_THE_COLUMN: i64 = 20260913000000;

        /// When the ambiguous entry was last written, and when its retry was
        /// queued. Distinct on purpose: the backfill takes the instant the
        /// joker was spent, which is the retry's, and a test where the two
        /// agreed could not tell one from the other.
        const SETTLED_AT: i64 = 1_700_000_111_000;
        const RETRIED_AT: i64 = 1_700_000_555_000;

        let temp = tempfile::tempdir().expect("temporary directory");
        let database = Database::open(&Config::for_data_dir(temp.path().join("data")))
            .await
            .expect("open database");

        let mut connection = database.pool.acquire().await.expect("connection");
        connection
            .ensure_migrations_table(&MIGRATOR.table_name)
            .await
            .expect("migrations table");
        for migration in MIGRATOR
            .iter()
            .filter(|migration| migration.version <= BEFORE_THE_COLUMN)
        {
            connection
                .apply(&MIGRATOR.table_name, migration)
                .await
                .expect("earlier migration");
        }

        // An account and one authorisation, inserted as rows rather than
        // through the services: this test is about what the migration does to
        // a shape, and `hash_password` on a literal is what the repository's
        // code scanning refuses.
        const ACCOUNT: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000011";
        const LINK: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000012";
        for (statement, binds) in [
            (
                "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
                   VALUES (?, 'listener', 'this-account-never-authenticates', 'user', 0, 0)",
                vec![ACCOUNT],
            ),
            (
                "INSERT INTO scrobble_link (id, user_id, provider, status, credential_nonce, \
                                            credential_ciphertext, created_at, updated_at) \
                   VALUES (?, ?, 'listenbrainz', 'active', x'000000000000000000000000', \
                           x'00', 0, 0)",
                vec![LINK, ACCOUNT],
            ),
        ] {
            let mut query = sqlx::query(statement);
            for bind in binds {
                query = query.bind(bind.to_owned());
            }
            query.execute(&mut *connection).await.expect("seed row");
        }

        // Three rows in the shape #191 leaves behind: an ambiguous entry whose
        // joker was spent, the retry that spent it, and a second ambiguous
        // entry nobody has answered.
        const ANSWERED: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000013";
        const RETRY: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000014";
        const UNANSWERED: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000015";
        for (public_id, state, created_at, updated_at) in [
            (ANSWERED, "uncertain", SETTLED_AT, SETTLED_AT),
            (RETRY, "sent", RETRIED_AT, RETRIED_AT),
            (UNANSWERED, "uncertain", SETTLED_AT, SETTLED_AT),
        ] {
            sqlx::query(
                "INSERT INTO scrobble_outbox (public_id, link_id, played_at, title, \
                 artists_json, state, attempts, next_attempt_at, created_at, updated_at) \
                 VALUES (?, ?, 0, 'Zenith', '[\"Nova Kern\"]', ?, 1, 0, ?, ?)",
            )
            .bind(public_id)
            .bind(LINK)
            .bind(state)
            .bind(created_at)
            .bind(updated_at)
            .execute(&mut *connection)
            .await
            .expect("seed outbox row");
        }
        // The link between them is written by lookup rather than by assuming
        // the first row took rowid 1: `retry_of` speaks in rowids, and a
        // fixture that guessed one would pass for a reason unrelated to the
        // migration.
        sqlx::query(
            "UPDATE scrobble_outbox \
                SET retry_of = (SELECT id FROM scrobble_outbox WHERE public_id = ?) \
              WHERE public_id = ?",
        )
        .bind(ANSWERED)
        .bind(RETRY)
        .execute(&mut *connection)
        .await
        .expect("point the retry at what it retried");
        drop(connection);

        database
            .migrate()
            .await
            .expect("a database with a queue migrates");

        let mut connection = database.pool.acquire().await.expect("connection");
        let answered: Option<i64> =
            sqlx::query_scalar("SELECT retried_at FROM scrobble_outbox WHERE public_id = ?")
                .bind(ANSWERED)
                .fetch_one(&mut *connection)
                .await
                .expect("the answered entry");
        assert_eq!(
            answered,
            Some(RETRIED_AT),
            "an entry retried before the column must carry the instant its retry was queued"
        );

        let unanswered: Option<i64> =
            sqlx::query_scalar("SELECT retried_at FROM scrobble_outbox WHERE public_id = ?")
                .bind(UNANSWERED)
                .fetch_one(&mut *connection)
                .await
                .expect("the unanswered entry");
        assert_eq!(
            unanswered, None,
            "an entry nobody answered must still be asking"
        );

        let retry: Option<i64> =
            sqlx::query_scalar("SELECT retried_at FROM scrobble_outbox WHERE public_id = ?")
                .bind(RETRY)
                .fetch_one(&mut *connection)
                .await
                .expect("the retry");
        assert_eq!(
            retry, None,
            "the retry spent nobody's joker; it is the one that was spent"
        );
    }

    /// Rows that finished before the column knew when they finished.
    ///
    /// The purge counts from `terminal_at`, so a row written before
    /// `20260914010000` would carry none and never be eligible: on a server
    /// that has been running for months, retention would arrive and apply to
    /// nothing it was added for. They take their `updated_at`, which is the
    /// write that made them terminal — true because every `UPDATE`
    /// `scrobble_outbox` knows requires a non-terminal starting state, so a
    /// finished row's last write *is* its transition.
    ///
    /// `uncertain` and `pending` take none, and that is half the assertion:
    /// an entry still asking a person must not become purgeable by a migration.
    #[tokio::test]
    async fn rows_that_finished_before_the_column_are_dated_by_their_last_write() {
        use sqlx::migrate::Migrate;

        /// The last migration before `scrobble_outbox.terminal_at`.
        const BEFORE_THE_COLUMN: i64 = 20260914000000;

        let temp = tempfile::tempdir().expect("temporary directory");
        let database = Database::open(&Config::for_data_dir(temp.path().join("data")))
            .await
            .expect("open database");

        let mut connection = database.pool.acquire().await.expect("connection");
        connection
            .ensure_migrations_table(&MIGRATOR.table_name)
            .await
            .expect("migrations table");
        for migration in MIGRATOR
            .iter()
            .filter(|migration| migration.version <= BEFORE_THE_COLUMN)
        {
            connection
                .apply(&MIGRATOR.table_name, migration)
                .await
                .expect("earlier migration");
        }

        const ACCOUNT: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000021";
        const LINK: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000022";
        for (statement, binds) in [
            (
                "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
                   VALUES (?, 'archivist', 'this-account-never-authenticates', 'user', 0, 0)",
                vec![ACCOUNT],
            ),
            (
                "INSERT INTO scrobble_link (id, user_id, provider, status, credential_nonce, \
                                            credential_ciphertext, created_at, updated_at) \
                   VALUES (?, ?, 'listenbrainz', 'active', x'000000000000000000000000', \
                           x'00', 0, 0)",
                vec![LINK, ACCOUNT],
            ),
        ] {
            let mut query = sqlx::query(statement);
            for bind in binds {
                query = query.bind(bind.to_owned());
            }
            query.execute(&mut *connection).await.expect("seed row");
        }

        // Every state the queue knows, each with its own `updated_at`, so an
        // assertion cannot pass by reading a neighbour's instant. `created_at`
        // is deliberately a different value throughout: a backfill that read
        // *it* would date a listen from when it was queued rather than from
        // when it stopped moving, which is the distinction the column exists
        // for.
        const QUEUED_AT: i64 = 1_700_000_000_000;
        let expected = [
            ("sent", 1_700_000_001_000_i64),
            ("rejected", 1_700_000_002_000),
            ("abandoned", 1_700_000_003_000),
            ("cancelled", 1_700_000_004_000),
            ("discarded", 1_700_000_005_000),
        ];
        let untouched = [
            ("uncertain", 1_700_000_006_000_i64),
            ("pending", 1_700_000_007_000),
        ];
        for (nth, (state, updated_at)) in expected.iter().chain(untouched.iter()).enumerate() {
            sqlx::query(
                "INSERT INTO scrobble_outbox (public_id, link_id, played_at, title, \
                 artists_json, state, attempts, next_attempt_at, created_at, updated_at) \
                 VALUES (?, ?, 0, 'Zenith', '[\"Nova Kern\"]', ?, 1, 0, ?, ?)",
            )
            .bind(format!("6f1d1b6e-0f1a-4a3d-9c21-0000000003{:02}", nth))
            .bind(LINK)
            .bind(*state)
            .bind(QUEUED_AT)
            .bind(*updated_at)
            .execute(&mut *connection)
            .await
            .expect("seed outbox row");
        }
        drop(connection);

        database
            .migrate()
            .await
            .expect("a database with a finished queue migrates");

        let mut connection = database.pool.acquire().await.expect("connection");
        for (state, updated_at) in expected {
            let terminal_at: Option<i64> =
                sqlx::query_scalar("SELECT terminal_at FROM scrobble_outbox WHERE state = ?")
                    .bind(state)
                    .fetch_one(&mut *connection)
                    .await
                    .expect("the finished row");
            assert_eq!(
                terminal_at,
                Some(updated_at),
                "a {state} row must be dated by the write that finished it"
            );
        }
        for (state, _) in untouched {
            let terminal_at: Option<i64> =
                sqlx::query_scalar("SELECT terminal_at FROM scrobble_outbox WHERE state = ?")
                    .bind(state)
                    .fetch_one(&mut *connection)
                    .await
                    .expect("the unfinished row");
            assert_eq!(
                terminal_at, None,
                "a {state} row has not finished and must not become purgeable"
            );
        }
    }
}
