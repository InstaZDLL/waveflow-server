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
