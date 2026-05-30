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

/// User-table helpers. Keeps the raw SQL out of handlers — same
/// boundary the project's no-SQL-in-handlers rule enforces for the
/// `/ready` probe. Returns the new user id so the dev caller can
/// stash it for the subsequent `X-User-Id` header.
pub mod users {
    use sqlx::PgPool;

    /// Insert a new user row and return its id. `BIGSERIAL` means
    /// we never pass the id in — Postgres allocates from its
    /// sequence. `external_id` is the seed for Phase 1.d's JWT
    /// auth: it gets matched against the verified `sub` claim of
    /// inbound Bearer tokens. Pass `None` from the dev `X-User-Id`
    /// shim path; pass `Some(sub)` once Better Auth lands.
    pub async fn create(
        pool: &PgPool,
        created_at: i64,
        external_id: Option<&str>,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (created_at, external_id) VALUES ($1, $2) RETURNING id",
        )
        .bind(created_at)
        .bind(external_id)
        .fetch_one(pool)
        .await
    }
}
