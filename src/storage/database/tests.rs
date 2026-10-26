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

/// A read takes the cache row without the gate, so a fill can replace and
/// link a newer portrait before the read's repair runs. That repair must
/// not put the hash it read back over the newer one, nor announce it.
#[tokio::test]
async fn a_repair_does_not_restore_a_portrait_the_cache_replaced() {
    let (_temp, database) = migrated_database().await;
    const ACCOUNT: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000021";
    const LIBRARY: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000022";
    const ARTIST: &str = "6f1d1b6e-0f1a-4a3d-9c21-000000000023";
    let read = "a".repeat(64);
    let replaced = "b".repeat(64);
    for (statement, binds) in [
        (
            "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
               VALUES (?, 'owner', 'this-account-never-authenticates', 'admin', 0, 0)",
            vec![ACCOUNT],
        ),
        (
            "INSERT INTO library (id, owner_user_id, name, root_path, visibility, \
                                  created_at, updated_at) \
               VALUES (?, ?, 'Library', '/music', 'private', 0, 0)",
            vec![LIBRARY, ACCOUNT],
        ),
        (
            "INSERT INTO artwork (hash, format, source, byte_size, created_at) \
               VALUES (?, 'jpg', 'deezer', 1, 0), (?, 'jpg', 'deezer', 1, 0)",
            vec![read.as_str(), replaced.as_str()],
        ),
        (
            "INSERT INTO artist (id, library_id, name, canonical_name, artwork_hash, \
                                 created_at, updated_at) \
               VALUES (?, ?, 'Nova Kern', 'nova kern', ?, 0, 0)",
            vec![ARTIST, LIBRARY, replaced.as_str()],
        ),
        (
            "INSERT INTO metadata_artist (provider, name_canonical, outcome, image_hash, \
                                          fetched_at, expires_at) \
               VALUES ('deezer', 'nova kern', 'hit', ?, 0, 0)",
            vec![replaced.as_str()],
        ),
    ] {
        let mut query = sqlx::query(statement);
        for bind in binds {
            query = query.bind(bind.to_owned());
        }
        query.execute(&database.pool).await.expect("seed row");
    }

    database
        .link_artist_portrait("nova kern", &read)
        .await
        .expect("repair");

    let linked: String = sqlx::query_scalar("SELECT artwork_hash FROM artist WHERE id = ?")
        .bind(ARTIST)
        .fetch_one(&database.pool)
        .await
        .expect("artist row");
    assert!(linked == replaced, "the newer portrait was replaced");
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM library_event")
        .fetch_one(&database.pool)
        .await
        .expect("event count");
    assert_eq!(events, 0);
}
