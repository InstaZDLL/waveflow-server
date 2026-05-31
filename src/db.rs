//! Postgres pool wiring + migration runner.
//!
//! [`connect`] opens the sqlx `PgPool` once at boot. The migrations
//! under `./migrations` are embedded into [`MIGRATOR`] via
//! `sqlx::migrate!()` — the `_sqlx_migrations` bookkeeping table
//! records the SHA-384 of every applied migration, so editing a
//! previously-merged file makes the server refuse to start (the rule
//! already documented in the desktop `CLAUDE.md`). Schema evolutions
//! create a new dated migration.

use std::time::Duration;

use sqlx::{
    migrate::Migrator,
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};

use crate::config::Config;

/// Compile-time embedded migrations. The path is relative to this
/// `Cargo.toml`; the macro panics at compile time if the directory is
/// missing or contains malformed files, so a broken migration shows
/// up as a build failure rather than a runtime surprise.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Build the pool from the parsed [`Config`]. Pool size is bounded by
/// `db_max_connections`; idle connections are kept warm for 10 min so
/// a quiet hour doesn't cost a fresh TLS handshake on the next call.
///
/// We don't run migrations here — that's `run_migrations` so callers
/// (binary boot, integration-test harness) can stage the steps the way
/// they prefer.
pub async fn connect(config: &Config) -> anyhow::Result<PgPool> {
    let opts: PgConnectOptions = config
        .database_url
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid DATABASE_URL: {e}"))?;

    let pool = PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(600))
        .connect_with(opts)
        .await
        .map_err(|e| anyhow::anyhow!("postgres connect failed: {e}"))?;

    Ok(pool)
}

/// Apply every pending migration. Idempotent — already-applied
/// migrations are skipped; a checksum mismatch on a previously-applied
/// row aborts with a clear error before the server starts taking
/// traffic.
pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|e| anyhow::anyhow!("migration failed: {e}"))?;
    Ok(())
}

/// Schema-agnostic connectivity probe. `SELECT 1` round-trips through
/// the pool; success means the connection is alive, failure means
/// either the pool is exhausted or Postgres is unreachable. Lives in
/// this module rather than the handler so the SQL stays inside the DB
/// layer (per the project's no-SQL-in-handlers rule); a richer
/// readiness check would land here too once the server grows
/// dependencies beyond Postgres.
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
        .map(|_| ())
}

/// Sync log helpers. Lives here for the same reason as [`users`]: the
/// handlers in `api/sync.rs` should stay pure HTTP orchestration, so
/// every `INSERT` / `SELECT` against `sync_op` /
/// `sync_compaction_watermark` lands on a function in this module.
///
/// Tx-aware functions accept `&mut sqlx::PgConnection` (= `&mut *tx`
/// for an open transaction) so the batch handler can keep its
/// transaction across N inserts. Single-statement helpers take
/// `&PgPool` since they don't compose with a transaction.
pub mod sync {
    use sqlx::{postgres::PgRow, PgConnection, PgPool};
    use uuid::Uuid;

    /// Append one op. Returns the inserted row, or `None` when the
    /// `(user_id, device_id, operation_id)` UNIQUE absorbed an
    /// idempotent replay. The `(user_id, device_id, lamport_ts)`
    /// UNIQUE is *not* covered by `ON CONFLICT` — a violation there
    /// bubbles up as a `sqlx::Error::Database` with SQLSTATE 23505
    /// for the caller to map to its 409 path.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_op_returning(
        conn: &mut PgConnection,
        user_id: i64,
        device_id: &str,
        operation_id: Uuid,
        lamport_ts: i64,
        entity: &str,
        entity_id: &str,
        field: Option<&str>,
        op: &str,
        payload: Option<&serde_json::Value>,
        created_at: i64,
    ) -> Result<Option<PgRow>, sqlx::Error> {
        sqlx::query(
            "INSERT INTO sync_op \
                (user_id, device_id, operation_id, lamport_ts, entity, entity_id, field, op, payload, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT (user_id, device_id, operation_id) DO NOTHING \
             RETURNING id, operation_id, device_id, lamport_ts, entity, entity_id, field, op, payload, created_at",
        )
        .bind(user_id)
        .bind(device_id)
        .bind(operation_id)
        .bind(lamport_ts)
        .bind(entity)
        .bind(entity_id)
        .bind(field)
        .bind(op)
        .bind(payload)
        .bind(created_at)
        .fetch_optional(conn)
        .await
    }

    /// Fetch the row matching a previously-accepted `operation_id`.
    /// Caller has already confirmed the row exists via the
    /// `ON CONFLICT DO NOTHING` returning `None`, so this is a plain
    /// `fetch_one`.
    pub async fn fetch_op_by_operation_id(
        conn: &mut PgConnection,
        user_id: i64,
        device_id: &str,
        operation_id: Uuid,
    ) -> Result<PgRow, sqlx::Error> {
        sqlx::query(
            "SELECT id, operation_id, device_id, lamport_ts, entity, entity_id, field, op, payload, created_at \
             FROM sync_op \
             WHERE user_id = $1 AND device_id = $2 AND operation_id = $3",
        )
        .bind(user_id)
        .bind(device_id)
        .bind(operation_id)
        .fetch_one(conn)
        .await
    }

    /// Current `MAX(lamport_ts)` for a device. Returns `0` when the
    /// device has no rows yet. Used after a lamport-regression 23505
    /// to tell the client how far ahead the server is.
    pub async fn lamport_max(
        pool: &PgPool,
        user_id: i64,
        device_id: &str,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT COALESCE(MAX(lamport_ts), 0) FROM sync_op \
             WHERE user_id = $1 AND device_id = $2",
        )
        .bind(user_id)
        .bind(device_id)
        .fetch_one(pool)
        .await
    }

    /// Read the compaction watermark for a user. `None` means the
    /// compaction job hasn't touched this tenant yet (no row), which
    /// the pull guard treats as "no floor". A transport / pool error
    /// is propagated — silently treating it as `None` would let a
    /// resurrected-device case slip through during a DB hiccup.
    pub async fn fetch_compacted_up_to(
        pool: &PgPool,
        user_id: i64,
    ) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT compacted_up_to FROM sync_compaction_watermark \
             WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(pool)
        .await
    }

    /// Fetch the next page of ops with `id > since`, capped at
    /// `limit`, ordered ascending so the client can stream straight
    /// into its local replay.
    pub async fn pull_ops_since(
        pool: &PgPool,
        user_id: i64,
        since: i64,
        limit: i64,
    ) -> Result<Vec<PgRow>, sqlx::Error> {
        sqlx::query(
            "SELECT id, operation_id, device_id, lamport_ts, entity, entity_id, field, op, payload, created_at \
             FROM sync_op \
             WHERE user_id = $1 AND id > $2 \
             ORDER BY id ASC \
             LIMIT $3",
        )
        .bind(user_id)
        .bind(since)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

/// User-table helpers. Keeps the raw SQL out of handlers — same
/// boundary the project's no-SQL-in-handlers rule enforces for the
/// `/ready` probe.
pub mod users {
    use sqlx::PgPool;

    /// Resolve a JWT `sub` to an internal `users.id`, inserting a
    /// row if the sub is unknown. Used by the JWT middleware
    /// (Phase 1.c.3a) so a fresh Better Auth signup doesn't require
    /// a separate "onboard the user on waveflow-server" round-trip —
    /// the first authenticated request lazy-provisions the row.
    ///
    /// Read-first, write-on-miss. The common path — every JWT
    /// request after the first for a given user — hits the SELECT
    /// only and produces zero writes, avoiding the heap-tuple churn
    /// (and autovacuum cost) a pure `DO UPDATE … RETURNING` UPSERT
    /// would generate per-request. The miss path falls through to
    /// an `ON CONFLICT DO UPDATE` UPSERT so two concurrent first
    /// requests for the same fresh sub collapse atomically to one
    /// row — the loser's UPDATE is a no-op assignment that still
    /// fires `RETURNING id` so both callers get the winner's id.
    ///
    /// Trust source: a valid JWT verified against the Better Auth
    /// JWKS is the authoritative statement that this `sub` is a
    /// real user. The middleware never reaches this helper without
    /// signature + claims + `kid` validation passing first.
    pub async fn find_or_provision_by_external_id(
        pool: &PgPool,
        external_id: &str,
        created_at_ms: i64,
    ) -> Result<i64, sqlx::Error> {
        if let Some(id) =
            sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE external_id = $1")
                .bind(external_id)
                .fetch_optional(pool)
                .await?
        {
            return Ok(id);
        }

        // Miss path — INSERT, with an UPSERT fallback for the case
        // where a concurrent request lazy-provisioned the same sub
        // between our SELECT and INSERT. The no-op `DO UPDATE` keeps
        // `RETURNING id` firing so the loser of the race still gets
        // the winner's id rather than a NULL.
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (created_at, external_id) VALUES ($1, $2) \
             ON CONFLICT (external_id) DO UPDATE SET external_id = EXCLUDED.external_id \
             RETURNING id",
        )
        .bind(created_at_ms)
        .bind(external_id)
        .fetch_one(pool)
        .await
    }
}
