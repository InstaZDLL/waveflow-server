//! SQLite v2 connection, migrations and tenant-safe foundation repositories.

use std::{path::Path, str::FromStr, sync::Arc};

use sha2::{Digest, Sha384};
use sqlx::{
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Connection, SqliteConnection, SqlitePool,
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

use crate::config::Config;

mod accounts;
mod keys;
mod libraries;
mod metadata;
mod sessions;
mod subsonic_credentials;
mod sync_operations;

pub use accounts::{AccountRecord, AccountRole};
pub use keys::{KeyMaterial, Rekey};
pub use libraries::{LibraryRole, LibraryVisibility};
pub use sessions::{
    AccessRecord, ApiTokenRecord, AuthorizationRecord, NewApiToken, NewAuthorization, NewSession,
    SessionRecord,
};
pub use subsonic_credentials::SubsonicCredentialStatus;
pub(crate) use sync_operations::SyncOperationReservation;

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

impl Database {
    pub async fn open(config: &Config) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&config.data_dir).await?;
        let options = SqliteConnectOptions::new()
            .filename(&config.database_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(config.sqlite_busy_timeout)
            // Numbers in names by value, for listings paged in SQL; see
            // `natural_order` for why no schema object may name it.
            .collation(
                crate::natural_order::COLLATION,
                crate::natural_order::compare,
            );

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

    /// A write transaction, opened `BEGIN IMMEDIATE` under the writer gate.
    ///
    /// The gate is not the whole of the lock. A transaction dropped on an
    /// error path does not roll back where it is dropped: sqlx queues the
    /// `ROLLBACK` to its connection's worker thread, the gate is given back at
    /// once, and the next writer — on another connection — can find SQLite's
    /// write lock still held for as long as that thread takes to run it.
    ///
    /// A deferred `BEGIN` cannot wait that out. Its first read opens a read
    /// transaction, and SQLite never calls the busy handler to upgrade one, so
    /// the first write fails at once with `SQLITE_BUSY` — a 503, whatever
    /// `busy_timeout` says. `BEGIN IMMEDIATE` takes the write lock before
    /// anything is read, where the busy handler does apply. A read-only
    /// transaction keeps a plain `begin()`: it needs no lock, and must not
    /// take one.
    pub(crate) async fn begin_write(
        &self,
        _writer_guard: &OwnedMutexGuard<()>,
    ) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, sqlx::Error> {
        self.pool.begin_with("BEGIN IMMEDIATE").await
    }
}

fn parse_uuid(value: String) -> Result<Uuid, sqlx::Error> {
    Uuid::parse_str(&value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

fn parse_role(value: String) -> Result<AccountRole, sqlx::Error> {
    AccountRole::from_str(&value).map_err(|error| sqlx::Error::Decode(error.into_boxed_dyn_error()))
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
mod tests;
