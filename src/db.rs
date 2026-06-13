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
        profile_canonical_id: Option<&str>,
    ) -> Result<Option<PgRow>, sqlx::Error> {
        // Phase A.1 (RFC-003): every row also carries the HLC pair the
        // §2 total order is defined on. Until A.2 lands the wire shape
        // change that lets clients send their own `hlc`, we derive it
        // from `lamport_ts` exactly the way the 20260612000000 backfill
        // does — `(0, lamport_ts)`. That keeps the new
        // `UNIQUE (user_id, device_id, hlc_wall, hlc_logical)` invariant
        // satisfied without touching callers, and means a v2 op
        // (`hlc_wall > 0`) strictly outranks every legacy-shape row
        // under the §2 total order once A.2 ships.
        //
        // `hlc_logical` is INTEGER (i32) per the RFC §2 definition of
        // the logical counter. Validate the incoming `lamport_ts`
        // before binding so a hypothetical >2^31 value surfaces as a
        // typed error instead of Postgres's bare "integer out of
        // range" SQLSTATE 22003 — A.2's dedicated v2 column gains its
        // own narrower binding, but until then the legacy path needs
        // the gate.
        if !(0..=i64::from(i32::MAX)).contains(&lamport_ts) {
            return Err(sqlx::Error::Protocol(format!(
                "lamport_ts {lamport_ts} is out of range for hlc_logical (i32); widen the column or reset the device counter"
            )));
        }
        let hlc_logical: i32 = lamport_ts as i32;
        sqlx::query(
            "INSERT INTO sync_op \
                (user_id, device_id, operation_id, lamport_ts, hlc_wall, hlc_logical, entity, entity_id, field, op, payload, created_at, profile_canonical_id) \
             VALUES ($1, $2, $3, $4, 0, $5, $6, $7, $8, $9, $10, $11, $12) \
             ON CONFLICT (user_id, device_id, operation_id) DO NOTHING \
             RETURNING id, operation_id, device_id, lamport_ts, entity, entity_id, field, op, payload, created_at, profile_canonical_id",
        )
        .bind(user_id)
        .bind(device_id)
        .bind(operation_id)
        .bind(lamport_ts)
        .bind(hlc_logical)
        .bind(entity)
        .bind(entity_id)
        .bind(field)
        .bind(op)
        .bind(payload)
        .bind(created_at)
        .bind(profile_canonical_id)
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
            "SELECT id, operation_id, device_id, lamport_ts, entity, entity_id, field, op, payload, created_at, profile_canonical_id \
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
            "SELECT id, operation_id, device_id, lamport_ts, entity, entity_id, field, op, payload, created_at, profile_canonical_id \
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

/// SQL helpers for the public-share surface (Phase 1.g.1). All four
/// helpers key on `(user_id, profile_id, playlist_id)` so a request
/// targeting a playlist the caller doesn't own short-circuits at the
/// storage layer rather than the handler — same defence pattern as
/// the rest of the API.
pub mod share {
    use sqlx::PgPool;

    use rand::distributions::{Alphanumeric, DistString};

    /// URL-safe character length of the opaque share token. 32
    /// alphanumerics ≈ 190 bits of entropy, well above the 128-bit
    /// threshold the OWASP cheat sheet recommends for "opaque
    /// session-equivalent" tokens. Short enough to fit in a Bitly-
    /// style social card without wrapping.
    pub const TOKEN_LEN: usize = 32;

    /// Mint a fresh share token (or return the existing one if the
    /// playlist already has one) for a playlist the caller owns. The
    /// tenant chain (`user_id → profile_id → playlist`) is verified
    /// inline; a foreign-owned playlist surfaces as `Ok(None)`.
    ///
    /// Idempotent: a second call for the same playlist returns the
    /// existing token rather than rotating it. Rotation requires an
    /// explicit revoke + re-mint.
    pub async fn mint_or_get_token(
        pool: &PgPool,
        user_id: i64,
        profile_id: i64,
        playlist_id: i64,
    ) -> Result<Option<String>, sqlx::Error> {
        let candidate = Alphanumeric.sample_string(&mut rand::thread_rng(), TOKEN_LEN);
        // `COALESCE(share_token, $candidate)` — atomic and race-free.
        // If the row already had a token (mint called twice, or two
        // concurrent mints racing past our generation), the COALESCE
        // keeps the existing value and `RETURNING` echoes it back.
        // If `share_token IS NULL`, the candidate is planted. Either
        // way we never write twice and never need a re-SELECT.
        //
        // Ownership chain (`user_id → profile_id → playlist`) checked
        // inline. A foreign-owned playlist makes the WHERE match no
        // rows, `fetch_optional` returns `None`, and the handler maps
        // it to 404 — same no-existence-leak shape as the other
        // modules.
        sqlx::query_scalar::<_, String>(
            "UPDATE playlist
                SET share_token = COALESCE(share_token, $1)
              WHERE id = $2 AND profile_id = $3
                AND profile_id IN (SELECT id FROM profile WHERE user_id = $4)
              RETURNING share_token",
        )
        .bind(&candidate)
        .bind(playlist_id)
        .bind(profile_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
    }

    /// Drop the share token for a playlist the caller owns. Returns
    /// the rows-affected boolean so the handler can distinguish "no
    /// playlist" (404) from "already private" (204 no-op).
    pub async fn revoke_token(
        pool: &PgPool,
        user_id: i64,
        profile_id: i64,
        playlist_id: i64,
    ) -> Result<bool, sqlx::Error> {
        let res = sqlx::query(
            "UPDATE playlist
                SET share_token = NULL
              WHERE id = $1 AND profile_id = $2
                AND profile_id IN (SELECT id FROM profile WHERE user_id = $3)",
        )
        .bind(playlist_id)
        .bind(profile_id)
        .bind(user_id)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Public lookup — fetch the playlist row by token without any
    /// auth check. Returns the column tuple the public handler
    /// projects into its response DTO. A token that was minted then
    /// revoked surfaces as `None` (no row matches) — same shape as a
    /// token that never existed, so an attacker can't distinguish
    /// "revoked" from "never minted".
    #[allow(clippy::type_complexity)]
    pub async fn fetch_public_by_token(
        pool: &PgPool,
        token: &str,
    ) -> Result<
        Option<(
            i64,
            String,
            Option<String>,
            String,
            String,
            Option<String>,
            i64,
            i64,
        )>,
        sqlx::Error,
    > {
        sqlx::query_as(
            "SELECT p.id, p.name, p.description, p.color_id, p.icon_id,
                    p.cover_hash, p.created_at, p.updated_at
               FROM playlist p
              WHERE p.share_token = $1",
        )
        .bind(token)
        .fetch_optional(pool)
        .await
    }

    /// Variant of [`mint_or_get_token`] keyed on canonical ids
    /// instead of BIGSERIAL ids. The desktop only knows the UUIDs
    /// it mints locally; the server-side ids are an artefact of
    /// the apply pipeline that the desktop never sees directly.
    /// Same race-free `COALESCE` shape, same no-existence-leak
    /// `Ok(None)` for foreign tenants. The tenant chain becomes
    /// `(user_id, profile.canonical_id, playlist.canonical_id)`
    /// — a desktop user can only mint for their own profile, and
    /// the playlist must already have been materialised by the
    /// apply pipeline (see `apply::playlist::insert`).
    pub async fn mint_or_get_token_by_canonical(
        pool: &PgPool,
        user_id: i64,
        profile_canonical_id: &str,
        playlist_canonical_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        let candidate = Alphanumeric.sample_string(&mut rand::thread_rng(), TOKEN_LEN);
        sqlx::query_scalar::<_, String>(
            "UPDATE playlist
                SET share_token = COALESCE(share_token, $1)
              WHERE canonical_id = $2
                AND profile_id IN (
                    SELECT id FROM profile
                     WHERE user_id = $3 AND canonical_id = $4
                )
              RETURNING share_token",
        )
        .bind(&candidate)
        .bind(playlist_canonical_id)
        .bind(user_id)
        .bind(profile_canonical_id)
        .fetch_optional(pool)
        .await
    }

    /// Variant of [`revoke_token`] keyed on canonical ids.
    pub async fn revoke_token_by_canonical(
        pool: &PgPool,
        user_id: i64,
        profile_canonical_id: &str,
        playlist_canonical_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let res = sqlx::query(
            "UPDATE playlist
                SET share_token = NULL
              WHERE canonical_id = $1
                AND profile_id IN (
                    SELECT id FROM profile
                     WHERE user_id = $2 AND canonical_id = $3
                )",
        )
        .bind(playlist_canonical_id)
        .bind(user_id)
        .bind(profile_canonical_id)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }
}

/// `playlist_track` materialisation helpers (Phase 1.j.a). Reads +
/// writes against the join table populated by the apply pipeline
/// when desktops emit `entity: "playlist", field: "tracks"` ops.
/// SQL stays here, the apply handlers + the share preview SELECT
/// only call into these functions.
pub mod playlist_track {
    use sqlx::PgConnection;

    /// One row's projection for the public share preview. `snapshot_*`
    /// fields carry the displayable values cross-device — rows whose
    /// `snapshot_title IS NULL` (older desktop emitter pre-1.j.b)
    /// are excluded by the public SELECT so the preview only ever
    /// surfaces displayable rows.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PublicTrackRow {
        pub title: String,
        pub artist: Option<String>,
        pub duration_ms: i64,
    }

    /// Insert / upsert a batch of `(track_id, snapshot?)` pairs into a
    /// playlist. Position assignment is "append at the end" — we
    /// SELECT the current MAX(position) once and add 1 per inserted
    /// row, mirroring the desktop `append_tracks_conn` behaviour.
    /// `ON CONFLICT (playlist_id, track_id) DO UPDATE` lets a
    /// re-emit of the same op carry an updated snapshot without
    /// disturbing the position (the desktop never reorders + inserts
    /// in the same op, so position writes only come from the
    /// `set` / reorder path).
    pub async fn append_tracks(
        conn: &mut PgConnection,
        playlist_id: i64,
        rows: &[(i64, Option<TrackSnapshot>)],
        now_ms: i64,
    ) -> Result<(), sqlx::Error> {
        if rows.is_empty() {
            return Ok(());
        }

        // Read MAX(position) once so we don't pay a round-trip per
        // row. NULL → -1 so the first inserted lands at position 0.
        let max_position: Option<i32> =
            sqlx::query_scalar("SELECT MAX(position) FROM playlist_track WHERE playlist_id = $1")
                .bind(playlist_id)
                .fetch_one(&mut *conn)
                .await?;
        let mut next_position = max_position.unwrap_or(-1).saturating_add(1);

        for (track_id, snapshot) in rows {
            let (title, artist, duration_ms) = match snapshot {
                Some(s) => (Some(s.title.clone()), s.artist.clone(), Some(s.duration_ms)),
                None => (None, None, None),
            };
            sqlx::query(
                "INSERT INTO playlist_track
                      (playlist_id, track_id, position, added_at,
                       snapshot_title, snapshot_artist, snapshot_duration_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (playlist_id, track_id) DO UPDATE
                      SET snapshot_title = COALESCE(EXCLUDED.snapshot_title, playlist_track.snapshot_title),
                          snapshot_artist = COALESCE(EXCLUDED.snapshot_artist, playlist_track.snapshot_artist),
                          snapshot_duration_ms = COALESCE(EXCLUDED.snapshot_duration_ms, playlist_track.snapshot_duration_ms)",
            )
            .bind(playlist_id)
            .bind(track_id)
            .bind(next_position)
            .bind(now_ms)
            .bind(title)
            .bind(artist)
            .bind(duration_ms)
            .execute(&mut *conn)
            .await?;
            next_position = next_position.saturating_add(1);
        }
        Ok(())
    }

    /// Drop tracks from a playlist. The desktop's outbound op is
    /// "DELETE the set of these track_ids", same shape we model
    /// here. Unknown ids are silently filtered by the WHERE so a
    /// replay against a divergent server cache is a no-op rather
    /// than an error.
    pub async fn remove_tracks(
        conn: &mut PgConnection,
        playlist_id: i64,
        track_ids: &[i64],
    ) -> Result<u64, sqlx::Error> {
        if track_ids.is_empty() {
            return Ok(0);
        }
        let res = sqlx::query(
            "DELETE FROM playlist_track
              WHERE playlist_id = $1 AND track_id = ANY($2)",
        )
        .bind(playlist_id)
        .bind(track_ids)
        .execute(&mut *conn)
        .await?;
        Ok(res.rows_affected())
    }

    /// Reorder a single track. The desktop emits one of these per
    /// drag-and-drop; bulk reorders fan out into N separate ops.
    /// We UPDATE the target's position; the implicit gap left by
    /// the old position resolves the next time the desktop replays
    /// a full snapshot — the public preview reads ORDER BY position
    /// so gaps just produce the same visible ordering.
    pub async fn set_position(
        conn: &mut PgConnection,
        playlist_id: i64,
        track_id: i64,
        new_position: i32,
    ) -> Result<u64, sqlx::Error> {
        let res = sqlx::query(
            "UPDATE playlist_track
                SET position = $3
              WHERE playlist_id = $1 AND track_id = $2",
        )
        .bind(playlist_id)
        .bind(track_id)
        .bind(new_position)
        .execute(&mut *conn)
        .await?;
        Ok(res.rows_affected())
    }

    /// Snapshot accepted from the `payload.snapshots` map. Title is
    /// the only required field; artist + duration are present in
    /// the common case but tolerated absent so a desktop with a
    /// partial tag library still ships useful previews.
    #[derive(Debug, Clone)]
    pub struct TrackSnapshot {
        pub title: String,
        pub artist: Option<String>,
        pub duration_ms: i64,
    }

    /// Public share preview: list the tracks belonging to a
    /// playlist, in position order, filtered to rows with a non-
    /// NULL snapshot title (the only ones we can display in the
    /// public preview). Desktops still on the pre-1.j.b wire emit
    /// without snapshots; their rows stay invisible until they
    /// re-sync on a newer client.
    pub async fn fetch_for_share(
        pool: &sqlx::PgPool,
        playlist_id: i64,
    ) -> Result<Vec<PublicTrackRow>, sqlx::Error> {
        let rows = sqlx::query_as::<_, (String, Option<String>, i64)>(
            "SELECT snapshot_title, snapshot_artist, COALESCE(snapshot_duration_ms, 0)
               FROM playlist_track
              WHERE playlist_id = $1 AND snapshot_title IS NOT NULL
              ORDER BY position ASC",
        )
        .bind(playlist_id)
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(title, artist, duration_ms)| PublicTrackRow {
                title,
                artist,
                duration_ms,
            })
            .collect())
    }

    /// One row's projection for the owner-facing track list. Carries
    /// the source desktop's local `track_id` (the `playlist_track`
    /// PK component, NOT a server-canonical reference) + position +
    /// added_at + the optional snapshot fields. The owner is allowed
    /// to see rows whose snapshot is NULL — pre-1.j.b desktops
    /// emitted ops without snapshots; the row still belongs to the
    /// playlist and the owner can see "Track #<id>" placeholders
    /// until they re-sync on a newer client. The public share
    /// preview filters NULL snapshots, the owner read does not.
    #[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
    pub struct OwnerTrackRow {
        pub track_id: i64,
        pub position: i32,
        pub added_at: i64,
        pub snapshot_title: Option<String>,
        pub snapshot_artist: Option<String>,
        pub snapshot_duration_ms: Option<i64>,
    }

    /// Owner-facing track list: validate the tenant chain
    /// `playlist → profile → user` first, then fetch every row in
    /// position order. `Ok(None)` covers "no such playlist", "wrong
    /// profile", and "wrong user" with the same response so the
    /// handler can blur the three into a single 404.
    ///
    /// Two round-trips on purpose: a single CTE-joined SELECT would
    /// conflate "playlist not owned" with "playlist owned but empty"
    /// in the result set (both yield zero rows), and we need the
    /// 404 vs `[]` distinction at the HTTP boundary. The window
    /// between the ownership check and the fetch is benign
    /// *because* `playlist_track.playlist_id` carries
    /// `ON DELETE CASCADE` (migration `20260609000000_playlist_track.sql`):
    /// the only way a row can vanish between the two queries is via
    /// parent-playlist deletion, which makes `[]` the correct answer
    /// — same shape a brand-new empty playlist returns. If a future
    /// migration adds another row-hiding mechanism (soft-delete,
    /// archive flag, conditional unique), the rationale here needs
    /// to be re-evaluated.
    pub async fn fetch_for_owner(
        pool: &sqlx::PgPool,
        playlist_id: i64,
        profile_id: i64,
        user_id: i64,
    ) -> Result<Option<Vec<OwnerTrackRow>>, sqlx::Error> {
        let owned: Option<(i64,)> = sqlx::query_as(
            "SELECT pl.id
               FROM playlist pl
               INNER JOIN profile p ON p.id = pl.profile_id
              WHERE pl.id = $1 AND pl.profile_id = $2 AND p.user_id = $3",
        )
        .bind(playlist_id)
        .bind(profile_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
        if owned.is_none() {
            return Ok(None);
        }
        let rows = sqlx::query_as::<_, OwnerTrackRow>(
            "SELECT track_id, position, added_at,
                    snapshot_title, snapshot_artist, snapshot_duration_ms
               FROM playlist_track
              WHERE playlist_id = $1
              ORDER BY position ASC, track_id ASC",
        )
        .bind(playlist_id)
        .fetch_all(pool)
        .await?;
        Ok(Some(rows))
    }
}

/// Track-sync materialisation helpers (Phase 4.d.0.2). Upserts
/// album, artist, track_artist, and track rows from inbound sync
/// ops. Every helper takes a `&mut PgConnection` so the caller's
/// transaction wraps the whole chain atomically — a partial apply
/// (track minted but track_artist write failed) would corrupt the
/// per-library invariant we just built in 4.d.0.1.
pub mod track_sync {
    use sqlx::PgConnection;

    /// Per-row payload for the multi-artist link. `position` is the
    /// index in the source desktop's `; `-split artist tag — the
    /// apply pipeline computes it from the array order so the wire
    /// shape stays compact (`payload.artists: ["A", "B"]`).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ArtistLinkInput {
        pub name: String,
        pub position: i32,
    }

    /// Track upsert payload. Mirrors `TrackResponse` minus the
    /// server-assigned `id` and the joined `album_id` (the apply
    /// pipeline resolves the album first, then passes the id here).
    /// All optional audio specs default to NULL when absent — the
    /// desktop omits them for codecs that don't expose the value.
    #[derive(Debug, Clone)]
    pub struct TrackInput<'a> {
        pub library_id: i64,
        pub file_hash: &'a str,
        pub title: &'a str,
        pub file_path: &'a str,
        pub file_size: i64,
        pub duration_ms: i64,
        pub track_number: Option<i64>,
        pub disc_number: Option<i64>,
        pub year: Option<i64>,
        pub bitrate: Option<i64>,
        pub sample_rate: Option<i64>,
        pub channels: Option<i64>,
        pub bit_depth: Option<i64>,
        pub codec: Option<&'a str>,
        pub musical_key: Option<&'a str>,
        pub added_at: i64,
        pub album_id: Option<i64>,
    }

    /// Insert or update the artist row keyed on `(library_id,
    /// name)`. Returns the row's id either way. `updated_at` is
    /// bumped on the upsert path so a later list-by-recently-
    /// updated query reflects the activity.
    pub async fn upsert_artist(
        conn: &mut PgConnection,
        library_id: i64,
        name: &str,
        now: i64,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO artist (library_id, name, created_at, updated_at)
             VALUES ($1, $2, $3, $3)
             ON CONFLICT (library_id, name) DO UPDATE
                 SET updated_at = EXCLUDED.updated_at
             RETURNING id",
        )
        .bind(library_id)
        .bind(name)
        .bind(now)
        .fetch_one(&mut *conn)
        .await
    }

    /// Insert or update the album row keyed on the natural key
    /// `(library_id, canonical_title, album_artist_id)` (UNIQUE
    /// NULLS NOT DISTINCT so the compilation case with NULL
    /// `album_artist_id` collapses to one row per
    /// `(library, title)`). `year` and `is_compilation` propagate
    /// on the upsert path — a re-emit with corrected metadata
    /// updates the row in place.
    pub async fn upsert_album(
        conn: &mut PgConnection,
        library_id: i64,
        canonical_title: &str,
        album_artist_id: Option<i64>,
        year: Option<i64>,
        is_compilation: bool,
        now: i64,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO album
                (library_id, canonical_title, album_artist_id,
                 year, is_compilation, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $6)
             ON CONFLICT (library_id, canonical_title, album_artist_id) DO UPDATE
                 SET year = COALESCE(EXCLUDED.year, album.year),
                     is_compilation = album.is_compilation OR EXCLUDED.is_compilation,
                     updated_at = EXCLUDED.updated_at
             RETURNING id",
        )
        .bind(library_id)
        .bind(canonical_title)
        .bind(album_artist_id)
        .bind(year)
        .bind(is_compilation)
        .bind(now)
        .fetch_one(&mut *conn)
        .await
    }

    /// Insert or update the track row keyed on
    /// `(library_id, file_path)` — the existing natural key from
    /// `20260530000003_track.sql:64`. Every scalar field
    /// (including `file_hash`) overwrites the existing value on
    /// the upsert path so a tag-edit re-emit reflects the latest
    /// state. `file_path` is the right upsert key here because the
    /// desktop's tag-editor rewrites the audio file's metadata
    /// frames, which changes the BLAKE3 hash but NOT the path —
    /// keying on `file_hash` would treat every tag edit as a new
    /// row and trip the pre-existing `(library_id, file_path)`
    /// UNIQUE on the second pass.
    pub async fn upsert_track(
        conn: &mut PgConnection,
        input: &TrackInput<'_>,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO track (
                library_id, file_hash, title, file_path, file_size,
                duration_ms, track_number, disc_number, year,
                bitrate, sample_rate, channels, bit_depth, codec,
                musical_key, added_at, album_id
             )
             VALUES (
                 $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                 $12, $13, $14, $15, $16, $17
             )
             ON CONFLICT (library_id, file_path) DO UPDATE
                 SET title = EXCLUDED.title,
                     file_hash = EXCLUDED.file_hash,
                     file_size = EXCLUDED.file_size,
                     duration_ms = EXCLUDED.duration_ms,
                     track_number = EXCLUDED.track_number,
                     disc_number = EXCLUDED.disc_number,
                     year = EXCLUDED.year,
                     bitrate = EXCLUDED.bitrate,
                     sample_rate = EXCLUDED.sample_rate,
                     channels = EXCLUDED.channels,
                     bit_depth = EXCLUDED.bit_depth,
                     codec = EXCLUDED.codec,
                     musical_key = EXCLUDED.musical_key,
                     album_id = EXCLUDED.album_id
             RETURNING id",
        )
        .bind(input.library_id)
        .bind(input.file_hash)
        .bind(input.title)
        .bind(input.file_path)
        .bind(input.file_size)
        .bind(input.duration_ms)
        .bind(input.track_number)
        .bind(input.disc_number)
        .bind(input.year)
        .bind(input.bitrate)
        .bind(input.sample_rate)
        .bind(input.channels)
        .bind(input.bit_depth)
        .bind(input.codec)
        .bind(input.musical_key)
        .bind(input.added_at)
        .bind(input.album_id)
        .fetch_one(&mut *conn)
        .await
    }

    /// Replace the multi-artist link rows for a track. DELETE then
    /// a single UNNEST-driven INSERT so the apply pipeline pays one
    /// round-trip for the insert regardless of artist count (a
    /// per-row loop would be N+1 on the wire). Two-step (replace
    /// rather than diff) so the apply pipeline doesn't have to
    /// compute the diff between the inbound payload and the
    /// existing rows.
    ///
    /// Safe because the cross-library composite FK on
    /// `track_artist` guarantees BOTH `track_id` and `artist_id`
    /// already live in `library_id` when this is called (the
    /// caller resolved them before passing in `links`).
    pub async fn replace_track_artists(
        conn: &mut PgConnection,
        track_id: i64,
        library_id: i64,
        links: &[(i64, i32)],
    ) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM track_artist WHERE track_id = $1")
            .bind(track_id)
            .execute(&mut *conn)
            .await?;
        if links.is_empty() {
            return Ok(());
        }
        let artist_ids: Vec<i64> = links.iter().map(|(id, _)| *id).collect();
        let positions: Vec<i32> = links.iter().map(|(_, pos)| *pos).collect();
        sqlx::query(
            "INSERT INTO track_artist (track_id, artist_id, library_id, position)
             SELECT $1, artist_id, $2, position
             FROM UNNEST($3::bigint[], $4::int[]) AS t(artist_id, position)",
        )
        .bind(track_id)
        .bind(library_id)
        .bind(&artist_ids)
        .bind(&positions)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Delete a track keyed on `(library_id, file_path)`. Using
    /// `file_path` (not `file_hash`) means a per-library file-path
    /// delete only ever removes one row even when the user has
    /// the same audio content at two paths in the same library —
    /// matches the desktop's "delete one file" event. The composite
    /// FK from `track_artist` cascades the link rows automatically;
    /// `track.album_id` SET NULL propagates through the schema
    /// constraints. Returns the number of rows actually removed
    /// (0 when the file_path isn't known to this library).
    pub async fn delete_track(
        conn: &mut PgConnection,
        library_id: i64,
        file_path: &str,
    ) -> Result<u64, sqlx::Error> {
        let res = sqlx::query("DELETE FROM track WHERE library_id = $1 AND file_path = $2")
            .bind(library_id)
            .bind(file_path)
            .execute(&mut *conn)
            .await?;
        Ok(res.rows_affected())
    }

    /// Resolve a library canonical id to its server `library.id`
    /// scoped to the calling profile. `None` covers "library not
    /// found" / "library belongs to a different profile" — the
    /// apply pipeline treats both as `Skipped` so the op stays in
    /// the durable log for replay after the library's own insert
    /// lands.
    pub async fn lookup_library_id(
        conn: &mut PgConnection,
        profile_id: i64,
        library_canonical_id: &str,
    ) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM library
              WHERE profile_id = $1 AND canonical_id = $2",
        )
        .bind(profile_id)
        .bind(library_canonical_id)
        .fetch_optional(&mut *conn)
        .await
    }
}

/// Album browse helpers (Phase 4.d.0.4). Reads against the `album`
/// table for the `GET /api/v1/profiles/{p}/libraries/{l}/albums`
/// surface plus the per-album drill-down. Writes still go through
/// `track_sync::upsert_album` from the apply pipeline — this module
/// is read-only.
///
/// Same 2-query ownership-check + fetch shape as
/// [`playlist_track::fetch_for_owner`]: ownership SELECT first to
/// distinguish 404 ("library not owned" / "album not owned") from
/// 200 `[]` ("owned but empty"), then the bulk row fetch. The race
/// window between the two is benign because every parent (library →
/// profile) carries `ON DELETE CASCADE` — the only way a row can
/// vanish between the two queries is parent-tenant deletion, which
/// makes `[]` the correct answer (same shape a brand-new empty
/// library returns).
pub mod album {
    use sqlx::PgPool;
    use waveflow_core::domain::track::TrackRow;

    /// Wire-shaped album row. Joins `artist` once to surface the
    /// `album_artist_name` so the album-grid UI on the web doesn't
    /// have to fan out N artist lookups. `album_artist_name` is
    /// `None` for compilations (the schema's `NULLS NOT DISTINCT`
    /// collapse keeps one row per `(library, title)` with NULL
    /// `album_artist_id`).
    #[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
    pub struct AlbumRow {
        pub id: i64,
        pub canonical_title: String,
        pub album_artist_id: Option<i64>,
        pub album_artist_name: Option<String>,
        pub year: Option<i64>,
        pub cover_hash: Option<String>,
        pub is_compilation: bool,
        pub created_at: i64,
        pub updated_at: i64,
    }

    /// List every album under `(library_id, profile_id, user_id)`,
    /// most-recently-updated first. `Ok(None)` covers "no such
    /// library", "library belongs to another profile", and "profile
    /// belongs to another user" — the handler blurs the three into
    /// a single 404. `Ok(Some(vec![]))` is "owned but empty" and
    /// renders as `[]`. The ORDER BY rides
    /// `album_library_updated_idx (library_id, updated_at DESC)`
    /// planted in `20260608120000_album_artist.sql:217-218`.
    ///
    /// `id ASC` is the tie-breaker on equal `updated_at` so the
    /// order is deterministic across pages even when a batch
    /// upsert (the apply pipeline runs a whole sync round in one
    /// transaction) stamps several rows with the same epoch
    /// millisecond — without it, list pagination would shuffle on
    /// each request.
    pub async fn list_for_library(
        pool: &PgPool,
        library_id: i64,
        profile_id: i64,
        user_id: i64,
    ) -> Result<Option<Vec<AlbumRow>>, sqlx::Error> {
        let owned: Option<(i64,)> = sqlx::query_as(
            "SELECT l.id
               FROM library l
               INNER JOIN profile p ON p.id = l.profile_id
              WHERE l.id = $1 AND l.profile_id = $2 AND p.user_id = $3",
        )
        .bind(library_id)
        .bind(profile_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
        if owned.is_none() {
            return Ok(None);
        }
        let rows = sqlx::query_as::<_, AlbumRow>(
            "SELECT a.id,
                    a.canonical_title,
                    a.album_artist_id,
                    ar.name AS album_artist_name,
                    a.year,
                    a.cover_hash,
                    a.is_compilation,
                    a.created_at,
                    a.updated_at
               FROM album a
               LEFT JOIN artist ar ON ar.id = a.album_artist_id
              WHERE a.library_id = $1
              ORDER BY a.updated_at DESC, a.id ASC",
        )
        .bind(library_id)
        .fetch_all(pool)
        .await?;
        Ok(Some(rows))
    }

    /// Drill-down: tracks under `album_id`, scoped to the tenant
    /// chain `track → library → profile → user`. `Ok(None)` covers
    /// every non-owned case (album missing / foreign library /
    /// foreign profile / foreign user). `Ok(Some(vec![]))` is
    /// "album owned but empty" — possible when every track linked
    /// to it was deleted (the FK is `ON DELETE SET NULL` so the
    /// album row outlives its tracks).
    ///
    /// Column projection matches `PostgresTrackRepository::
    /// list_for_library` (NULL placeholders for the joined columns
    /// that aren't yet stored server-side) so the resulting
    /// `TrackRow` flows through `From<TrackRow> for TrackResponse`
    /// unchanged. `ORDER BY disc_number, track_number` rides the
    /// `track_album_idx` planted in the schema migration.
    pub async fn list_tracks_for_album(
        pool: &PgPool,
        album_id: i64,
        library_id: i64,
        profile_id: i64,
        user_id: i64,
    ) -> Result<Option<Vec<TrackRow>>, sqlx::Error> {
        let owned: Option<(i64,)> = sqlx::query_as(
            "SELECT a.id
               FROM album a
               INNER JOIN library l ON l.id = a.library_id
               INNER JOIN profile p ON p.id = l.profile_id
              WHERE a.id = $1
                AND a.library_id = $2
                AND l.profile_id = $3
                AND p.user_id = $4",
        )
        .bind(album_id)
        .bind(library_id)
        .bind(profile_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
        if owned.is_none() {
            return Ok(None);
        }
        let rows = sqlx::query_as::<_, TrackRow>(
            "SELECT t.id,
                    t.library_id,
                    t.title,
                    t.album_id,
                    NULL::text    AS album_title,
                    NULL::bigint  AS artist_id,
                    NULL::text    AS artist_name,
                    NULL::text    AS artist_ids,
                    t.duration_ms,
                    t.track_number,
                    t.disc_number,
                    t.year,
                    t.bitrate,
                    t.sample_rate,
                    t.channels,
                    t.bit_depth,
                    t.codec,
                    t.musical_key,
                    t.file_path,
                    t.file_size,
                    t.added_at,
                    NULL::text    AS artwork_hash,
                    NULL::text    AS artwork_format,
                    t.rating
               FROM track t
              WHERE t.album_id = $1 AND t.library_id = $2
              ORDER BY t.disc_number NULLS LAST,
                       t.track_number NULLS LAST,
                       t.id ASC",
        )
        .bind(album_id)
        .bind(library_id)
        .fetch_all(pool)
        .await?;
        Ok(Some(rows))
    }
}

/// Artist browse helpers (Phase 4.d.0.4). Mirrors [`album`] for the
/// `artist` table — same 2-query ownership-check shape, same 404 /
/// 200 [] blur. Writes still go through `track_sync::upsert_artist`.
pub mod artist {
    use sqlx::PgPool;
    use waveflow_core::domain::track::TrackRow;

    /// Wire-shaped artist row. `picture_hash` is the BLAKE3 hex of
    /// the artist picture in the shared metadata cache — `None`
    /// until the artist-picture pipeline ships server-side.
    #[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
    pub struct ArtistRow {
        pub id: i64,
        pub name: String,
        pub picture_hash: Option<String>,
        pub created_at: i64,
        pub updated_at: i64,
    }

    /// List every artist under `(library_id, profile_id, user_id)`,
    /// most-recently-updated first. Same 404 vs `[]` blur as
    /// [`album::list_for_library`]. The ORDER BY rides
    /// `artist_library_updated_idx (library_id, updated_at DESC)`.
    pub async fn list_for_library(
        pool: &PgPool,
        library_id: i64,
        profile_id: i64,
        user_id: i64,
    ) -> Result<Option<Vec<ArtistRow>>, sqlx::Error> {
        let owned: Option<(i64,)> = sqlx::query_as(
            "SELECT l.id
               FROM library l
               INNER JOIN profile p ON p.id = l.profile_id
              WHERE l.id = $1 AND l.profile_id = $2 AND p.user_id = $3",
        )
        .bind(library_id)
        .bind(profile_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
        if owned.is_none() {
            return Ok(None);
        }
        let rows = sqlx::query_as::<_, ArtistRow>(
            "SELECT id, name, picture_hash, created_at, updated_at
               FROM artist
              WHERE library_id = $1
              ORDER BY updated_at DESC, id ASC",
        )
        .bind(library_id)
        .fetch_all(pool)
        .await?;
        Ok(Some(rows))
    }

    /// Drill-down: tracks contributed by `artist_id`, scoped to the
    /// tenant chain `track → library → profile → user`. Tracks
    /// surface through `track_artist`, so a multi-artist track
    /// appears under every contributor. `Ok(None)` covers every
    /// non-owned case.
    ///
    /// `DISTINCT` is defensive — the `track_artist` PK
    /// `(track_id, artist_id)` already guarantees no duplicates,
    /// but a future schema relax (e.g. a per-version row) would
    /// silently double-fetch without it. Cheap on Postgres because
    /// the index is unique already. ORDER BY matches the album
    /// drill-down for a consistent UI feel.
    pub async fn list_tracks_for_artist(
        pool: &PgPool,
        artist_id: i64,
        library_id: i64,
        profile_id: i64,
        user_id: i64,
    ) -> Result<Option<Vec<TrackRow>>, sqlx::Error> {
        let owned: Option<(i64,)> = sqlx::query_as(
            "SELECT a.id
               FROM artist a
               INNER JOIN library l ON l.id = a.library_id
               INNER JOIN profile p ON p.id = l.profile_id
              WHERE a.id = $1
                AND a.library_id = $2
                AND l.profile_id = $3
                AND p.user_id = $4",
        )
        .bind(artist_id)
        .bind(library_id)
        .bind(profile_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
        if owned.is_none() {
            return Ok(None);
        }
        let rows = sqlx::query_as::<_, TrackRow>(
            "SELECT DISTINCT
                    t.id,
                    t.library_id,
                    t.title,
                    t.album_id,
                    NULL::text    AS album_title,
                    NULL::bigint  AS artist_id,
                    NULL::text    AS artist_name,
                    NULL::text    AS artist_ids,
                    t.duration_ms,
                    t.track_number,
                    t.disc_number,
                    t.year,
                    t.bitrate,
                    t.sample_rate,
                    t.channels,
                    t.bit_depth,
                    t.codec,
                    t.musical_key,
                    t.file_path,
                    t.file_size,
                    t.added_at,
                    NULL::text    AS artwork_hash,
                    NULL::text    AS artwork_format,
                    t.rating
               FROM track t
               INNER JOIN track_artist ta ON ta.track_id = t.id
              WHERE ta.artist_id = $1 AND t.library_id = $2
              ORDER BY t.disc_number NULLS LAST,
                       t.track_number NULLS LAST,
                       t.id ASC",
        )
        .bind(artist_id)
        .bind(library_id)
        .fetch_all(pool)
        .await?;
        Ok(Some(rows))
    }
}

/// Shared artwork cache helpers (Phase 1.h.1). Reads + writes against
/// the [`metadata_artwork`] table. The bytes themselves live in
/// `object_store` at `artwork/<hash>`; this module is the metadata
/// side of the cache, keeping the SQL out of the HTTP handler so
/// `api/artwork.rs` stays pure orchestration.
pub mod artwork {
    use sqlx::PgPool;

    /// Metadata row as projected by the GET handler. Holds enough
    /// state to vend the correct Content-Type + Content-Length
    /// without touching the object store for HEAD-style probes.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ArtworkMeta {
        pub hash: String,
        pub mime: String,
        pub byte_size: i64,
    }

    /// Insert a metadata row if absent. Returns `true` if the row is
    /// new (caller still needs to upload the bytes), `false` if a row
    /// with the same hash already existed (caller can skip the
    /// upload — the bytes are identical by BLAKE3's collision
    /// resistance).
    ///
    /// Concurrency: two clients uploading the same hash race here,
    /// `ON CONFLICT DO NOTHING` makes both safe — the second insert
    /// is a no-op and `rows_affected()` returns 0. The bytes upload
    /// is idempotent in the storage layer too (same payload → same
    /// final state), so a "winner" doesn't matter.
    pub async fn insert_if_absent(
        pool: &PgPool,
        hash: &str,
        mime: &str,
        byte_size: i64,
    ) -> Result<bool, sqlx::Error> {
        let res = sqlx::query(
            "INSERT INTO metadata_artwork (hash, mime, byte_size)
             VALUES ($1, $2, $3)
             ON CONFLICT (hash) DO NOTHING",
        )
        .bind(hash)
        .bind(mime)
        .bind(byte_size)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Lookup metadata by hash. `None` when no row exists — the GET
    /// handler maps that to 404 without consulting the object store
    /// (cheaper than a backend HEAD, and "in storage but no row"
    /// shouldn't happen anyway because [`insert_if_absent`] commits
    /// after the storage write succeeds).
    pub async fn fetch_meta(pool: &PgPool, hash: &str) -> Result<Option<ArtworkMeta>, sqlx::Error> {
        sqlx::query_as::<_, (String, String, i64)>(
            "SELECT hash, mime, byte_size
               FROM metadata_artwork
              WHERE hash = $1",
        )
        .bind(hash)
        .fetch_optional(pool)
        .await
        .map(|opt| {
            opt.map(|(hash, mime, byte_size)| ArtworkMeta {
                hash,
                mime,
                byte_size,
            })
        })
    }

    /// Metadata row for one resize variant. Same shape as
    /// [`ArtworkMeta`] plus the per-variant dimensions — surfaced
    /// so a client can pre-size the layout slot without measuring
    /// the JPEG.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct VariantMeta {
        pub hash: String,
        pub mime: String,
        pub byte_size: i64,
        pub width: i32,
        pub height: i32,
    }

    /// Insert a variant row if absent. Mirrors the parent table's
    /// race-safe `ON CONFLICT DO NOTHING` shape: two callers
    /// generating the same variant for the same parent collapse
    /// into one row (`(parent_hash, variant)` is the PK), and the
    /// loser's bytes are byte-equal anyway because the resize is
    /// deterministic. Returns `true` if the row is new.
    ///
    /// Same shape as `sync::insert_op_returning`: the parameter
    /// surface mirrors the table columns one-for-one rather than
    /// folding them behind a struct, which keeps the call sites
    /// in `api/artwork.rs` symmetric with the rest of the DB
    /// helpers. `#[allow(clippy::too_many_arguments)]` is the
    /// pre-existing convention for that pattern.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_variant_if_absent(
        pool: &PgPool,
        parent_hash: &str,
        variant: &str,
        hash: &str,
        mime: &str,
        byte_size: i64,
        width: i32,
        height: i32,
    ) -> Result<bool, sqlx::Error> {
        let res = sqlx::query(
            "INSERT INTO metadata_artwork_variant
                  (parent_hash, variant, hash, mime, byte_size, width, height)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (parent_hash, variant) DO NOTHING",
        )
        .bind(parent_hash)
        .bind(variant)
        .bind(hash)
        .bind(mime)
        .bind(byte_size)
        .bind(width)
        .bind(height)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Lookup one variant by `(parent_hash, variant)`. The handler
    /// hits this when the URL looks like
    /// `/api/v1/artwork/{parent}/{variant}`.
    pub async fn fetch_variant(
        pool: &PgPool,
        parent_hash: &str,
        variant: &str,
    ) -> Result<Option<VariantMeta>, sqlx::Error> {
        sqlx::query_as::<_, (String, String, i64, i32, i32)>(
            "SELECT hash, mime, byte_size, width, height
               FROM metadata_artwork_variant
              WHERE parent_hash = $1 AND variant = $2",
        )
        .bind(parent_hash)
        .bind(variant)
        .fetch_optional(pool)
        .await
        .map(|opt| {
            opt.map(|(hash, mime, byte_size, width, height)| VariantMeta {
                hash,
                mime,
                byte_size,
                width,
                height,
            })
        })
    }

    /// Lookup a variant by its OWN hash. Lets a client that already
    /// cached the variant hash hit the bare `GET /api/v1/artwork/{hash}`
    /// route without paying the parent-lookup detour. Returns the
    /// projected metadata (same shape the parent endpoint vends), so
    /// the handler stays uniform across "is this a parent or a
    /// variant?" cases.
    pub async fn fetch_meta_by_variant_hash(
        pool: &PgPool,
        hash: &str,
    ) -> Result<Option<ArtworkMeta>, sqlx::Error> {
        sqlx::query_as::<_, (String, String, i64)>(
            "SELECT hash, mime, byte_size
               FROM metadata_artwork_variant
              WHERE hash = $1
              LIMIT 1",
        )
        .bind(hash)
        .fetch_optional(pool)
        .await
        .map(|opt| {
            opt.map(|(hash, mime, byte_size)| ArtworkMeta {
                hash,
                mime,
                byte_size,
            })
        })
    }

    /// List the parents whose variant cache is incomplete — the
    /// drive for the background scanner ([`crate::artwork_jobs`]).
    /// Returns the BLAKE3 hex of each parent whose
    /// `metadata_artwork_variant` row count is below `expected`,
    /// filtered by the repair-backoff window and ordered so
    /// recoverable rows always lead.
    ///
    /// The query lives here (not in the scanner module) for the same
    /// reason every other SQL lives in `db.rs` — handlers + jobs stay
    /// pure orchestration, all schema knowledge funnels through the
    /// DB layer.
    ///
    /// `backoff_cutoff_ms` is the epoch-millis cutoff before which
    /// a previous failure no longer suppresses the row — pass
    /// `now_ms - backoff`. Rows whose `last_repair_failure_at`
    /// falls inside the window are skipped, so an irrecoverable
    /// parent (e.g. one whose source bytes were lost from
    /// object_store) can't dominate every cycle and starve
    /// recoverable parents behind it. The `NULLS FIRST` sort means
    /// never-tried rows always lead; freshly-failed ones recede
    /// until the cooldown expires.
    ///
    /// Epoch-millis (BIGINT) matches the convention every other
    /// timestamp column on this server follows so the SQLite mirror
    /// in waveflow-core stays shape-compatible.
    pub async fn list_partial_parents(
        pool: &PgPool,
        expected: i64,
        limit: i64,
        backoff_cutoff_ms: i64,
    ) -> Result<Vec<String>, sqlx::Error> {
        sqlx::query_scalar::<_, String>(
            "SELECT a.hash
               FROM metadata_artwork a
          LEFT JOIN (
                  SELECT parent_hash, COUNT(*) AS variant_count
                    FROM metadata_artwork_variant
                   GROUP BY parent_hash
              ) v ON v.parent_hash = a.hash
              WHERE COALESCE(v.variant_count, 0) < $1
                AND (a.last_repair_failure_at IS NULL
                  OR a.last_repair_failure_at < $3)
              ORDER BY a.last_repair_failure_at ASC NULLS FIRST,
                       a.created_at ASC
              LIMIT $2",
        )
        .bind(expected)
        .bind(limit)
        .bind(backoff_cutoff_ms)
        .fetch_all(pool)
        .await
    }

    /// Stamp `last_repair_failure_at = now_ms` on a parent the
    /// scanner just failed to repair. Pushes the row to the back of
    /// the queue for `REPAIR_BACKOFF` so the next cycle can pick up
    /// other parents instead of retrying the same broken row
    /// immediately. Caller passes the timestamp explicitly so the
    /// scanner's clock + the DB's clock stay decoupled (a future
    /// distributed deploy can mint the timestamp at the worker
    /// rather than relying on `NOW()`).
    pub async fn mark_repair_failure(
        pool: &PgPool,
        hash: &str,
        now_ms: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE metadata_artwork SET last_repair_failure_at = $1 WHERE hash = $2")
            .bind(now_ms)
            .bind(hash)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// List every variant of a parent. The upload handler returns
    /// these in the response so the client knows what's available
    /// without a second round-trip. Ordered by `variant` for stable
    /// serialisation across calls.
    pub async fn fetch_variants_for_parent(
        pool: &PgPool,
        parent_hash: &str,
    ) -> Result<Vec<(String, VariantMeta)>, sqlx::Error> {
        let rows = sqlx::query_as::<_, (String, String, String, i64, i32, i32)>(
            "SELECT variant, hash, mime, byte_size, width, height
               FROM metadata_artwork_variant
              WHERE parent_hash = $1
              ORDER BY variant ASC",
        )
        .bind(parent_hash)
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(variant, hash, mime, byte_size, width, height)| {
                (
                    variant,
                    VariantMeta {
                        hash,
                        mime,
                        byte_size,
                        width,
                        height,
                    },
                )
            })
            .collect())
    }
}
