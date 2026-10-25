//! Upgrades — the one path every other target skips.
//!
//! `test_app()` builds its database from nothing, so the whole suite proves
//! the schema can be **created** and never that it can be **upgraded** (#223).
//! An upgrade is what every existing installation does, and it is the failure
//! that costs most: migrations are embedded and checksummed, so one that goes
//! wrong leaves the server refusing to start — found by somebody who has just
//! updated, on a database that is already theirs.
//!
//! The fixture is built by code rather than checked in as a database file. A
//! file is a snapshot of a real installation and catches what nobody thought
//! to write; code is honest about what it exercises, survives a re-run, and
//! carries no binary blob into the repository. The second was chosen for the
//! last reason as much as the first — a blob nobody can read is a fixture
//! nobody can correct.
//!
//! Not the first migration test: `a_populated_database_survives_a_table_rebuild`
//! in `src/database.rs` already applies a prefix and migrates from there. It is
//! pinned to **one** boundary — the participants rebuild it was written for —
//! and asserts raw counts. This asks the general question instead, from every
//! schema a tag actually shipped, and answers it through the services rather
//! than through `SELECT COUNT(*)`: a row that survives a migration and is no
//! longer reachable by a query has not survived in any sense a user would
//! recognise.

use sqlx::migrate::Migrate;
use sqlx::Connection;
use sqlx::SqliteConnection;
use waveflow_server::database::Database;
use waveflow_server::database::MIGRATOR;
use waveflow_server::services::AlbumListQuery;
use waveflow_server::services::BrowsePage;
use waveflow_server::Config;

/// The schema each released tag left on disk, named by its last migration.
///
/// **Add a line when a tag ships, and never edit one.** These are the states
/// real databases are sitting in: a released schema stops being a hypothesis
/// the moment somebody installs it. Both were measured with
/// `git ls-tree --name-only <tag> migrations-v2/`, not read off the changelog,
/// because the changelog records what was written and the tree records what
/// shipped.
///
/// Remembering was the whole mechanism, so
/// `the_version_being_shipped_has_its_schema_recorded` now holds it: the PR
/// that bumps `version` in `Cargo.toml` for a release fails until it adds that
/// tag here, with the newest migration of that moment as its baseline.
const RELEASED_SCHEMAS: [(&str, i64); 2] = [
    ("v2.0.0-beta.0", 20_260_823_020_000),
    ("v2.0.0-beta.1", 20_260_914_030_000),
];

/// The crate's version is the tag it ships as, so it must name a line above.
///
/// A release is cut by a PR that bumps `version` and is then tagged, and the
/// version stays put until the next one. So this holds between releases, and
/// fails exactly on the bump that forgot the schema — before the tag exists,
/// which is the only moment the baseline is still the newest migration and
/// needs no archaeology to find.
#[test]
fn the_version_being_shipped_has_its_schema_recorded() {
    let tag = concat!("v", env!("CARGO_PKG_VERSION"));
    assert!(
        RELEASED_SCHEMAS
            .iter()
            .any(|(released, _)| *released == tag),
        "Cargo.toml is at {tag}, and RELEASED_SCHEMAS has no line for it: add \
         (\"{tag}\", <version of the newest file under migrations-v2/>)"
    );
}

/// The identifiers the fixture below writes and the assertions read back.
///
/// UUIDs because every real row's is: a fixture shaped unlike production is
/// one a production-shaped defect can pass through.
const ACCOUNT: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a1";
const LIBRARY: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a2";
const ARTIST_LEAD: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a3";
const ARTIST_GUEST: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a4";
const ALBUM: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a5";
const TRACK: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a6";
const GENRE: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a7";
const PLAYLIST: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a8";
const SCAN: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a9";

/// One account, one library, and a catalogue with every shape a later
/// migration might have to carry.
///
/// Written against the **oldest** schema in [`RELEASED_SCHEMAS`], which every
/// later one is a superset of, so one fixture serves them all. Every column is
/// named explicitly for the same reason: a migration that adds one must not
/// silently change what this inserts.
///
/// What it holds is chosen, not arbitrary. An album credited to **two**
/// artists is here because a credit model that only ever saw one was how the
/// participants rebuild shipped; a genre, a playlist, a favourite and a play
/// event are the user data that no rescan can reconstruct, which is what makes
/// losing it unrecoverable rather than merely slow.
async fn seed_an_installation(connection: &mut SqliteConnection) {
    // `password_hash` is a placeholder written straight into the row rather
    // than derived: nothing here ever authenticates, and putting a literal
    // through Argon2 is what makes CodeQL report a hard-coded cryptographic
    // value on a test that has no credentials in it.
    for (statement, binds) in [
        (
            "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
               VALUES (?, 'upgrade-owner', 'placeholder', 'admin', 1000, 1000)",
            vec![ACCOUNT],
        ),
        (
            "INSERT INTO library (id, owner_user_id, name, root_path, visibility, \
                                  created_at, updated_at) \
               VALUES (?, ?, 'Upgraded library', '/music/upgraded', 'private', 1000, 1000)",
            vec![LIBRARY, ACCOUNT],
        ),
        (
            "INSERT INTO library_member (library_id, user_id, role, created_at) \
               VALUES (?, ?, 'owner', 1000)",
            vec![LIBRARY, ACCOUNT],
        ),
        (
            "INSERT INTO artist (id, library_id, name, canonical_name, created_at, updated_at) \
               VALUES (?, ?, 'Nova Kern', 'nova kern', 1000, 1000)",
            vec![ARTIST_LEAD, LIBRARY],
        ),
        (
            "INSERT INTO artist (id, library_id, name, canonical_name, created_at, updated_at) \
               VALUES (?, ?, 'Vale Ortiz', 'vale ortiz', 1000, 1000)",
            vec![ARTIST_GUEST, LIBRARY],
        ),
        (
            "INSERT INTO artist_fts (artist_id, library_id, name, sort_name) \
               VALUES (?, ?, 'Nova Kern', '')",
            vec![ARTIST_LEAD, LIBRARY],
        ),
        (
            "INSERT INTO artist_fts (artist_id, library_id, name, sort_name) \
               VALUES (?, ?, 'Vale Ortiz', '')",
            vec![ARTIST_GUEST, LIBRARY],
        ),
        (
            "INSERT INTO album (id, library_id, title, canonical_title, album_artist_id, \
                                album_artist_name, year, created_at, updated_at) \
               VALUES (?, ?, 'Convergence', 'convergence', ?, 'Nova Kern', 2026, 1000, 1000)",
            vec![ALBUM, LIBRARY, ARTIST_LEAD],
        ),
        (
            "INSERT INTO album_participant (album_id, artist_id, library_id, role, position) \
               VALUES (?, ?, ?, 'albumartist', 0)",
            vec![ALBUM, ARTIST_LEAD, LIBRARY],
        ),
        (
            "INSERT INTO album_participant (album_id, artist_id, library_id, role, position) \
               VALUES (?, ?, ?, 'artist', 0)",
            vec![ALBUM, ARTIST_LEAD, LIBRARY],
        ),
        (
            "INSERT INTO album_participant (album_id, artist_id, library_id, role, position) \
               VALUES (?, ?, ?, 'artist', 1)",
            vec![ALBUM, ARTIST_GUEST, LIBRARY],
        ),
        (
            "INSERT INTO genre (id, library_id, name, canonical_name, created_at) \
               VALUES (?, ?, 'Ambient', 'ambient', 1000)",
            vec![GENRE, LIBRARY],
        ),
        (
            // The scan that last saw the track, with an error of its own: the
            // two rows that point at `scan_job`, which RFC-021 rebuilds.
            "INSERT INTO scan_job (id, library_id, requested_by, trigger, status, total_files, \
                                   processed_files, added, errors, created_at, started_at, \
                                   completed_at) \
               VALUES (?, ?, ?, 'boot', 'completed', 1, 1, 1, 1, 1000, 1000, 1000)",
            vec![SCAN, LIBRARY, ACCOUNT],
        ),
        (
            "INSERT INTO scan_error (scan_id, relative_path, message, created_at) \
               VALUES (?, 'convergence/02.flac', 'unreadable', 1000)",
            vec![SCAN],
        ),
        (
            "INSERT INTO track (id, library_id, album_id, relative_path, file_size, \
                                file_modified_at, quick_hash, full_hash, title, album_title, \
                                artist_display, genre_display, year, track_number, disc_number, \
                                duration_ms, is_available, last_seen_scan_id, created_at, \
                                updated_at) \
               VALUES (?, ?, ?, 'convergence/01.flac', 4096, 1000, ?, ?, 'Zenith', \
                       'Convergence', 'Nova Kern', 'Ambient', 2026, 1, 1, 210000, 1, ?, 1000, \
                       1000)",
            vec![
                TRACK,
                LIBRARY,
                ALBUM,
                &"1".repeat(64),
                &"2".repeat(64),
                SCAN,
            ],
        ),
        (
            "INSERT INTO track_participant (track_id, artist_id, library_id, role, sub_role, \
                                            position) \
               VALUES (?, ?, ?, 'artist', '', 0)",
            vec![TRACK, ARTIST_LEAD, LIBRARY],
        ),
        (
            "INSERT INTO track_participant (track_id, artist_id, library_id, role, sub_role, \
                                            position) \
               VALUES (?, ?, ?, 'artist', '', 1)",
            vec![TRACK, ARTIST_GUEST, LIBRARY],
        ),
        (
            // `albumartist`, not only `artist`: the scanner copies every
            // track role onto the album, and the services that answer an
            // album's credits select this one alone. Seeding `artist` by
            // itself left `albums[0].artists` empty, so an upgrade losing
            // every album credit would have passed (constat de revue,
            // pris).
            "INSERT INTO track_participant (track_id, artist_id, library_id, role, sub_role, \
                                            position) \
               VALUES (?, ?, ?, 'albumartist', '', 0)",
            vec![TRACK, ARTIST_LEAD, LIBRARY],
        ),
        (
            "INSERT INTO track_genre (track_id, genre_id, library_id) VALUES (?, ?, ?)",
            vec![TRACK, GENRE, LIBRARY],
        ),
        (
            "INSERT INTO track_fts (track_id, library_id, title, album, artists, genres) \
               VALUES (?, ?, 'Zenith', 'Convergence', 'Nova Kern Vale Ortiz', 'Ambient')",
            vec![TRACK, LIBRARY],
        ),
        (
            "INSERT INTO artist_role_stats (artist_id, library_id, role, album_count, song_count) \
               VALUES (?, ?, 'artist', 1, 1)",
            vec![ARTIST_LEAD, LIBRARY],
        ),
        (
            "INSERT INTO artist_role_stats (artist_id, library_id, role, album_count, song_count) \
               VALUES (?, ?, 'artist', 1, 1)",
            vec![ARTIST_GUEST, LIBRARY],
        ),
        (
            "INSERT INTO playlist (id, owner_user_id, name, comment, public, created_at, \
                                   updated_at) \
               VALUES (?, ?, 'Carried over', 'written before the upgrade', 0, 1000, 1000)",
            vec![PLAYLIST, ACCOUNT],
        ),
        (
            "INSERT INTO playlist_track (playlist_id, track_id, position, added_at) \
               VALUES (?, ?, 0, 1000)",
            vec![PLAYLIST, TRACK],
        ),
        (
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
               VALUES (?, 'album', ?, 1000)",
            vec![ACCOUNT, ALBUM],
        ),
        (
            "INSERT INTO play_event (user_id, track_id, submission, played_at) \
               VALUES (?, ?, 1, 1000)",
            vec![ACCOUNT, TRACK],
        ),
    ] {
        let mut query = sqlx::query(statement);
        for bind in binds {
            query = query.bind(bind.to_owned());
        }
        query
            .execute(&mut *connection)
            .await
            .unwrap_or_else(|error| panic!("seeding `{statement}` failed: {error}"));
    }
}

async fn count(connection: &mut SqliteConnection, query: &'static str) -> i64 {
    sqlx::query_scalar(query)
        .fetch_one(&mut *connection)
        .await
        .unwrap_or_else(|error| panic!("`{query}` failed: {error}"))
}

#[tokio::test]
async fn every_released_schema_upgrades_with_the_rows_it_was_holding() {
    let total = MIGRATOR.iter().count();

    for (tag, baseline) in RELEASED_SCHEMAS {
        let temp = tempfile::tempdir().expect("temporary directory");
        let config = Config::for_data_dir(temp.path().join("data"));

        // Step one: a database at the released schema, holding rows.
        let database = Database::open(&config).await.expect("open the database");
        let mut connection = database.pool().acquire().await.expect("connection");
        connection
            .ensure_migrations_table(&MIGRATOR.table_name)
            .await
            .expect("migrations table");
        // A baseline that names no migration is a typo, and a typo here is
        // silent: the filter below would still apply *some* prefix, and every
        // assertion would then hold about a schema no tag ever shipped
        // (constat de revue, pris).
        assert!(
            MIGRATOR
                .iter()
                .any(|migration| migration.version == baseline),
            "{tag}: no migration has version {baseline}"
        );
        for migration in MIGRATOR
            .iter()
            .filter(|migration| migration.version <= baseline)
        {
            connection
                .apply(&MIGRATOR.table_name, migration)
                .await
                .unwrap_or_else(|error| {
                    panic!("{tag}: migration {} failed: {error}", migration.version)
                });
        }
        seed_an_installation(&mut connection).await;

        // **This is what keeps the test from being a fresh-install test in
        // disguise.** If the prefix silently applied everything, the rows
        // above would have been written into today's schema and the upgrade
        // below would have nothing left to do — green, on the right lines, and
        // proving none of what it claims.
        let applied_before = count(&mut connection, "SELECT COUNT(*) FROM _sqlx_migrations").await;
        assert!(
            (applied_before as usize) < total,
            "{tag}: the baseline applied all {total} migrations, so nothing is being upgraded"
        );
        drop(connection);
        database.pool().close().await;
        drop(database);

        // Step two: the upgrade, through the boot path rather than a bare
        // `migrate()` call — an installation that updates runs all of it, and
        // the key binding and identity reconciliation read the same rows.
        let state = waveflow_server::initialize(&config)
            .await
            .unwrap_or_else(|error| panic!("{tag}: a populated database refused to boot: {error}"));

        let mut connection = state.db.pool().acquire().await.expect("connection");
        let applied_after = count(&mut connection, "SELECT COUNT(*) FROM _sqlx_migrations").await;
        assert_eq!(
            applied_after as usize, total,
            "{tag}: the upgrade left migrations unapplied"
        );

        // Step three: the rows are still there — and still *mean* something.
        // A count proves the statements ran; reading them back through the
        // services proves the upgraded schema is the one the server queries.
        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *connection)
            .await
            .expect("foreign key check");
        assert!(
            violations.is_empty(),
            "{tag}: the upgrade left {} broken reference(s)",
            violations.len()
        );
        drop(connection);

        let account: uuid::Uuid = ACCOUNT.parse().expect("account id");
        let album: uuid::Uuid = ALBUM.parse().expect("album id");
        let track: uuid::Uuid = TRACK.parse().expect("track id");

        let albums = state
            .services
            .list_albums(account, &AlbumListQuery::default())
            .await
            .expect("list albums");
        assert_eq!(albums.len(), 1, "{tag}: the album did not survive");
        assert_eq!(albums[0].title, "Convergence", "{tag}");
        assert_eq!(
            albums[0]
                .artists
                .iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Nova Kern"],
            "{tag}: the album credit did not survive the upgrade"
        );

        let found = state
            .services
            .catalog_search(
                account,
                &[],
                "Zenith",
                BrowsePage::default(),
                BrowsePage::default(),
                BrowsePage::default(),
            )
            .await
            .expect("search");
        assert_eq!(
            found.songs.len(),
            1,
            "{tag}: the search index did not come through the upgrade"
        );

        let playlists = state.services.playlists(account).await.expect("playlists");
        assert_eq!(playlists.len(), 1, "{tag}: the playlist did not survive");
        assert_eq!(playlists[0].name, "Carried over", "{tag}");

        // The three relations the fixture carries that nothing read back
        // until now (constat de revue, pris). A seeded row no assertion reads
        // is how a fixture comes to claim more than it proves — and the
        // second credit is the one that matters most here, because a credit
        // model that could only ever see the first is exactly what the
        // participants rebuild replaced.
        let playlist = state
            .services
            .playlist(account, PLAYLIST.parse().expect("playlist id"))
            .await
            .expect("playlist");
        assert_eq!(
            playlist
                .songs
                .iter()
                .map(|song| song.id)
                .collect::<Vec<_>>(),
            vec![track],
            "{tag}: the playlist lost the track it held"
        );
        assert_eq!(
            playlist.songs[0]
                .artists
                .iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Nova Kern", "Vale Ortiz"],
            "{tag}: a credit did not survive the upgrade"
        );
        assert_eq!(
            playlist.songs[0].genres,
            vec!["Ambient".to_owned()],
            "{tag}: the genre relation did not survive the upgrade"
        );

        let starred = state.services.starred(account, &[]).await.expect("starred");
        assert_eq!(
            starred
                .albums
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![album],
            "{tag}: the favourite did not survive"
        );

        // Listening history is the one thing a rescan cannot rebuild, and the
        // only reader that proves the rows are still joinable to a track.
        let top = state
            .services
            .artist_top_songs_by_name(account, "Nova Kern", 10)
            .await
            .expect("top songs");
        assert_eq!(
            top.len(),
            1,
            "{tag}: the play history did not survive the upgrade"
        );
        assert_eq!(top[0].title, "Zenith", "{tag}");

        // The scan history, through the table RFC-021 rebuilt to admit two
        // more triggers: the job, its error, and the track that names it as
        // the scan that last saw it. A rebuild run with foreign keys on would
        // have cascaded the error away and nulled the track's reference — and
        // a null there makes the next scan treat the track as never seen.
        let scan: uuid::Uuid = SCAN.parse().expect("scan id");
        let job = state
            .db
            .scan_job(scan)
            .await
            .expect("read the job")
            .unwrap_or_else(|| panic!("{tag}: the scan history did not survive"));
        assert_eq!(
            (job.status.as_str(), job.added, job.errors),
            ("completed", 1, 1),
            "{tag}"
        );
        assert_eq!((job.excluded, job.skipped_links), (0, 0), "{tag}");
        let mut connection = state.db.pool().acquire().await.expect("connection");
        assert_eq!(
            count(
                &mut connection,
                "SELECT COUNT(*) FROM scan_error WHERE scan_id = '6f1d1b6e-0f1a-4a3d-9c21-0000000000a9'"
            )
            .await,
            1,
            "{tag}: the scan's error did not survive"
        );
        assert_eq!(
            count(
                &mut connection,
                "SELECT COUNT(*) FROM track WHERE last_seen_scan_id = '6f1d1b6e-0f1a-4a3d-9c21-0000000000a9'"
            )
            .await,
            1,
            "{tag}: the track lost the scan that last saw it"
        );
    }
}

const TRACK_TWO: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a9";
const SHARE: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000aa";

/// The two lists RFC-020 rebuilt come through with every row, in its order,
/// and take a repeat afterwards.
///
/// `playlist_track` and `share_track` were keyed on `(list, track)` and are
/// keyed on `(list, position)` since `20261026000000`. A rebuild is a copy into
/// a new table, and the failure it invites is quiet: rows dropped by a
/// mistyped column, or positions renumbered so the order changes. So the
/// fixture stores its two tracks **against** their insertion order — the
/// second track first — which a rebuild that numbered rows as it read them
/// would get wrong, and reads them back through the services. Then it adds a
/// copy of a track the playlist already holds, which is what the new key is
/// for and what the old one refused.
#[tokio::test]
async fn the_rebuilt_lists_keep_their_rows_in_order_and_then_take_a_repeat() {
    for (tag, baseline) in RELEASED_SCHEMAS {
        let temp = tempfile::tempdir().expect("temporary directory");
        let config = Config::for_data_dir(temp.path().join("data"));
        let database = Database::open(&config).await.expect("open the database");
        let mut connection = database.pool().acquire().await.expect("connection");
        connection
            .ensure_migrations_table(&MIGRATOR.table_name)
            .await
            .expect("migrations table");
        for migration in MIGRATOR
            .iter()
            .filter(|migration| migration.version <= baseline)
        {
            connection
                .apply(&MIGRATOR.table_name, migration)
                .await
                .unwrap_or_else(|error| {
                    panic!("{tag}: migration {} failed: {error}", migration.version)
                });
        }
        seed_an_installation(&mut connection).await;
        for (statement, binds) in [
            (
                "INSERT INTO track (id, library_id, album_id, relative_path, file_size, \
                                    file_modified_at, quick_hash, full_hash, title, album_title, \
                                    artist_display, duration_ms, is_available, created_at, \
                                    updated_at) \
                   VALUES (?, ?, ?, 'convergence/02.flac', 4096, 1000, ?, ?, 'Nadir', \
                           'Convergence', 'Nova Kern', 180000, 1, 1000, 1000)",
                vec![TRACK_TWO, LIBRARY, ALBUM, &"5".repeat(64), &"6".repeat(64)],
            ),
            // The fixture's playlist already holds TRACK at position 0; it is
            // moved behind the new one, so the stored order is not the order
            // the rows were written in.
            (
                "UPDATE playlist_track SET position=1 WHERE playlist_id=? AND track_id=?",
                vec![PLAYLIST, TRACK],
            ),
            (
                "INSERT INTO playlist_track (playlist_id, track_id, position, added_at) \
                   VALUES (?, ?, 0, 1000)",
                vec![PLAYLIST, TRACK_TWO],
            ),
            (
                "INSERT INTO share (id, owner_user_id, token_hash, description, created_at, \
                                    updated_at, visit_count) \
                   VALUES (?, ?, randomblob(32), 'Carried share', 1000, 1000, 7)",
                vec![SHARE, ACCOUNT],
            ),
            (
                "INSERT INTO share_track (share_id, track_id, position) VALUES (?, ?, 1)",
                vec![SHARE, TRACK],
            ),
            (
                "INSERT INTO share_track (share_id, track_id, position) VALUES (?, ?, 0)",
                vec![SHARE, TRACK_TWO],
            ),
        ] {
            let mut query = sqlx::query(statement);
            for bind in binds {
                query = query.bind(bind.to_owned());
            }
            query
                .execute(&mut *connection)
                .await
                .unwrap_or_else(|error| panic!("{tag}: `{statement}` failed: {error}"));
        }
        drop(connection);
        database.pool().close().await;
        drop(database);

        let state = waveflow_server::initialize(&config)
            .await
            .unwrap_or_else(|error| panic!("{tag}: a populated database refused to boot: {error}"));
        let account: uuid::Uuid = ACCOUNT.parse().expect("account id");
        let playlist_id: uuid::Uuid = PLAYLIST.parse().expect("playlist id");
        let track: uuid::Uuid = TRACK.parse().expect("track id");
        let track_two: uuid::Uuid = TRACK_TWO.parse().expect("track id");

        let playlist = state
            .services
            .playlist(account, playlist_id)
            .await
            .expect("playlist");
        assert_eq!(
            playlist
                .songs
                .iter()
                .map(|song| song.id)
                .collect::<Vec<_>>(),
            vec![track_two, track],
            "{tag}: the playlist lost a row or its order"
        );
        assert_eq!(playlist.owner_username, "upgrade-owner", "{tag}");

        let shares = state.services.shares(account).await.expect("shares");
        assert_eq!(shares.len(), 1, "{tag}: the share did not survive");
        assert_eq!(shares[0].visit_count, 7, "{tag}");
        assert_eq!(shares[0].max_visits, None, "{tag}: an old share has no cap");
        assert_eq!(
            shares[0]
                .songs
                .iter()
                .map(|song| song.id)
                .collect::<Vec<_>>(),
            vec![track_two, track],
            "{tag}: the share lost a row or its order"
        );

        let repeated = state
            .services
            .update_playlist(
                account,
                playlist_id,
                None,
                None,
                None,
                &[track_two],
                &[],
                Default::default(),
            )
            .await
            .unwrap_or_else(|error| panic!("{tag}: a repeat was refused: {error}"));
        assert_eq!(
            repeated
                .songs
                .iter()
                .map(|song| song.id)
                .collect::<Vec<_>>(),
            vec![track_two, track, track_two],
            "{tag}"
        );
    }
}

/// A key change reads without migrating, and copies before it does.
///
/// Here rather than beside the other key tests in `cli.rs` for the reason this
/// whole target exists: `test_app()` builds a current schema, and on a current
/// schema a reading that migrates and one that does not are the same reading.
/// Only a database *behind* the binary can tell them apart, and building one
/// is what this file already knows how to do.
///
/// Two promises, both of which only a lagging schema can hold to account:
///
/// - **a reading changes nothing**, so `key rotate` without `--confirm` must
///   leave `_sqlx_migrations` exactly as it found it — applying a migration is
///   a change, whatever else it is;
/// - **the recovery copy is the database as found**, so it must still be at
///   the old schema after the confirmed run brought the live one forward. A
///   copy taken afterwards would restore the operator to a schema they never
///   chose to run.
#[tokio::test]
async fn a_key_change_reads_without_migrating_and_copies_before_it_migrates() {
    let total = MIGRATOR.iter().count();
    let (tag, baseline) = RELEASED_SCHEMAS[RELEASED_SCHEMAS.len() - 1];
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = Config::for_data_dir(temp.path().join("data"));

    let database = Database::open(&config).await.expect("open the database");
    let mut connection = database.pool().acquire().await.expect("connection");
    connection
        .ensure_migrations_table(&MIGRATOR.table_name)
        .await
        .expect("migrations table");
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version <= baseline)
    {
        connection
            .apply(&MIGRATOR.table_name, migration)
            .await
            .unwrap_or_else(|error| {
                panic!("{tag}: migration {} failed: {error}", migration.version)
            });
    }
    // The pair that schema was running under. Written by hand because the only
    // thing this test needs from it is that the two halves agree: `rotate`
    // refuses a key the database was not bound to, and that refusal is not
    // what is being measured here.
    let key = [5u8; 32];
    std::fs::write(&config.instance_key_path, key).expect("the key file");
    sqlx::query(
        "INSERT INTO instance_metadata (singleton, key_fingerprint, created_at) VALUES (1, ?, 0)",
    )
    .bind(waveflow_server::security::bytes_hash(&key).as_slice())
    .execute(&mut *connection)
    .await
    .expect("bind the key");
    let behind = count(&mut connection, "SELECT COUNT(*) FROM _sqlx_migrations").await;
    assert!(
        (behind as usize) < total,
        "{tag}: the baseline applied all {total} migrations, so nothing is behind"
    );
    drop(connection);
    database.pool().close().await;
    drop(database);

    // One: the reading. It is told there is an upgrade waiting, and leaves it
    // waiting.
    waveflow_server::cli::key(
        &config,
        waveflow_server::cli::KeyCommand::Rotate(waveflow_server::cli::KeyArgs { confirm: false }),
    )
    .await
    .expect("a reading of a lagging database");
    assert_eq!(
        applied_in(&config.database_path).await,
        behind,
        "a reading applied a migration"
    );
    assert_eq!(
        std::fs::read(&config.instance_key_path).unwrap(),
        key,
        "a reading replaced the key"
    );

    // Two: the change. The copy is the database as it was found.
    waveflow_server::cli::key(
        &config,
        waveflow_server::cli::KeyCommand::Rotate(waveflow_server::cli::KeyArgs { confirm: true }),
    )
    .await
    .expect("a confirmed rotation of a lagging database");
    assert_ne!(
        std::fs::read(&config.instance_key_path).unwrap(),
        key,
        "the rotation kept the old key"
    );
    let recovery = std::fs::read_dir(&config.data_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("pre-key-"))
        })
        .expect("a recovery directory");
    assert_eq!(
        applied_in(&recovery.join("waveflow.db")).await,
        behind,
        "the recovery copy was taken after the schema had already moved"
    );
    assert_eq!(
        applied_in(&config.database_path).await as usize,
        total,
        "the rotation left migrations unapplied"
    );

    // And the pair it leaves behind is one the server opens.
    waveflow_server::initialize(&config)
        .await
        .expect("the rotated pair must boot");
}

/// A database at the newest released schema, bound to a key on disk, as an
/// installation that is about to upgrade has it. Answers the key and how many
/// migrations the database has run.
///
/// `extra` runs on the database before it is closed, for a test that needs it
/// shaped some further way.
async fn an_installation_behind(config: &Config, extra: Option<&'static str>) -> ([u8; 32], i64) {
    let (tag, baseline) = RELEASED_SCHEMAS[RELEASED_SCHEMAS.len() - 1];
    let database = Database::open(config).await.expect("open the database");
    let mut connection = database.pool().acquire().await.expect("connection");
    connection
        .ensure_migrations_table(&MIGRATOR.table_name)
        .await
        .expect("migrations table");
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version <= baseline)
    {
        connection
            .apply(&MIGRATOR.table_name, migration)
            .await
            .unwrap_or_else(|error| {
                panic!("{tag}: migration {} failed: {error}", migration.version)
            });
    }
    seed_an_installation(&mut connection).await;
    // Drawn, not written down: the test needs the two halves to agree, and a
    // key in the source would be a key in the source.
    let key: [u8; 32] = [
        *uuid::Uuid::new_v4().as_bytes(),
        *uuid::Uuid::new_v4().as_bytes(),
    ]
    .concat()
    .try_into()
    .unwrap();
    std::fs::write(&config.instance_key_path, key).expect("the key file");
    sqlx::query(
        "INSERT INTO instance_metadata (singleton, key_fingerprint, created_at) VALUES (1, ?, 0)",
    )
    .bind(waveflow_server::security::bytes_hash(&key).as_slice())
    .execute(&mut *connection)
    .await
    .expect("bind the key");
    if let Some(statement) = extra {
        sqlx::query(statement)
            .execute(&mut *connection)
            .await
            .unwrap_or_else(|error| panic!("`{statement}` failed: {error}"));
    }
    let behind = count(&mut connection, "SELECT COUNT(*) FROM _sqlx_migrations").await;
    assert!(
        (behind as usize) < MIGRATOR.iter().count(),
        "{tag}: the baseline applied every migration, so nothing is behind"
    );
    drop(connection);
    database.pool().close().await;
    (key, behind)
}

/// The copy directories a boot made before migrating, newest first.
fn pre_migration_copies(config: &Config) -> Vec<std::path::PathBuf> {
    let mut copies: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(&config.data_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let taken_at = path
                .file_name()?
                .to_str()?
                .strip_prefix(waveflow_server::migration_backup::PREFIX)?
                .parse::<u64>()
                .ok()?;
            Some((taken_at, path))
        })
        .collect();
    copies.sort_by_key(|(taken_at, _)| std::cmp::Reverse(*taken_at));
    copies.into_iter().map(|(_, path)| path).collect()
}

/// RFC-019 decision 3: an upgrade copies the database **as it found it**, with
/// the key it was bound to, keeps the newest copies only, touches nothing it
/// did not make — and a boot with nothing to apply copies nothing.
#[tokio::test]
async fn an_upgrade_copies_the_database_and_its_key_before_migrating() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = Config::for_data_dir(temp.path().join("data"));
    let (key, behind) = an_installation_behind(&config, None).await;
    // Four older copies, and two directories that only look like one.
    for stale in ["1", "2", "3", "4"] {
        std::fs::create_dir_all(config.data_dir.join(format!(
            "{}{stale}",
            waveflow_server::migration_backup::PREFIX
        )))
        .unwrap();
    }
    let lookalike = config.data_dir.join(format!(
        "{}notes",
        waveflow_server::migration_backup::PREFIX
    ));
    let restore = config.data_dir.join("pre-restore-5");
    std::fs::create_dir_all(&lookalike).unwrap();
    std::fs::create_dir_all(&restore).unwrap();

    let state = waveflow_server::initialize(&config)
        .await
        .expect("an installation behind boots");
    state.db.pool().close().await;
    drop(state);

    let copies = pre_migration_copies(&config);
    assert_eq!(
        copies.len(),
        config.migration_backups,
        "kept {copies:?}, expected the newest {}",
        config.migration_backups
    );
    let newest = &copies[0];
    assert_eq!(
        applied_in(&newest.join("waveflow.db")).await,
        behind,
        "the copy was taken after the schema had moved"
    );
    assert_eq!(
        std::fs::read(newest.join("instance.key")).unwrap(),
        key,
        "the copy does not carry the key its database is bound to"
    );
    assert!(
        Database::check_file_instance_key(
            &newest.join("waveflow.db"),
            &waveflow_server::security::bytes_hash(&key)
        )
        .await
        .unwrap(),
        "the copied pair does not agree"
    );
    let prefix = waveflow_server::migration_backup::PREFIX;
    for gone in ["1", "2"] {
        assert!(!config.data_dir.join(format!("{prefix}{gone}")).exists());
    }
    for kept in ["3", "4"] {
        assert!(config.data_dir.join(format!("{prefix}{kept}")).exists());
    }
    assert!(
        lookalike.exists(),
        "pruning deleted a directory it did not make"
    );
    assert!(
        restore.exists(),
        "pruning deleted a restore's recovery copy"
    );

    // Nothing left to apply: nothing copied.
    waveflow_server::initialize(&config)
        .await
        .expect("an upgraded installation boots");
    assert_eq!(pre_migration_copies(&config), copies);
}

/// A migration that fails says where the copy is, and the copy is whole.
#[tokio::test]
async fn a_failed_upgrade_names_the_copy_it_can_be_restored_from() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = Config::for_data_dir(temp.path().join("data"));
    // A table a later migration creates, already there: that migration fails.
    let (key, behind) = an_installation_behind(
        &config,
        Some("CREATE TABLE radio_station (id TEXT PRIMARY KEY)"),
    )
    .await;

    let error = match waveflow_server::initialize(&config).await {
        Ok(_) => panic!("a migration that cannot apply booted"),
        Err(error) => format!("{error:#}"),
    };
    let copies = pre_migration_copies(&config);
    assert_eq!(copies.len(), 1, "{copies:?}");
    let copy = copies[0].display().to_string();
    assert!(
        error.contains(&format!("database restore --input {copy}")),
        "the failure does not say how to go back: {error}"
    );
    assert!(error.contains("v2 migration failed"), "{error}");
    assert_eq!(applied_in(&copies[0].join("waveflow.db")).await, behind);
    assert_eq!(std::fs::read(copies[0].join("instance.key")).unwrap(), key);
}

/// A copy that cannot be made stops the boot before anything is applied, and
/// leaves no half-made directory for pruning to count as a whole one.
#[tokio::test]
async fn a_copy_that_cannot_be_made_leaves_nothing_behind() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let mut config = Config::for_data_dir(temp.path().join("data"));
    let (_, behind) = an_installation_behind(&config, None).await;
    // The database copies; the key cannot, because a directory stands where
    // the key should be.
    config.instance_key_path = config.data_dir.join("key-is-a-directory");
    std::fs::create_dir_all(&config.instance_key_path).unwrap();

    let error = match waveflow_server::initialize(&config).await {
        Ok(_) => panic!("a boot without its copy went ahead"),
        Err(error) => format!("{error:#}"),
    };
    assert!(error.contains("could not copy the instance key"), "{error}");
    assert!(
        pre_migration_copies(&config).is_empty(),
        "an incomplete copy was left behind: {:?}",
        pre_migration_copies(&config)
    );
    assert_eq!(
        applied_in(&config.database_path).await,
        behind,
        "a migration ran without its copy"
    );
}

/// A database bound to a key is copied only with that key: missing or wrong,
/// the pair would be no way back, so the boot refuses before migrating — and
/// before minting a key of its own beside a database that has one. A
/// database never bound to a key is still copied alone; the released-schema
/// test above stands on that.
#[tokio::test]
async fn a_bound_database_is_not_copied_without_its_own_key() {
    for (case, damage) in [
        ("missing", None),
        ("wrong", Some(*uuid::Uuid::new_v4().as_bytes())),
    ] {
        let temp = tempfile::tempdir().expect("temporary directory");
        let config = Config::for_data_dir(temp.path().join("data"));
        let (_, behind) = an_installation_behind(&config, None).await;
        match damage {
            None => std::fs::remove_file(&config.instance_key_path).unwrap(),
            Some(other) => {
                std::fs::write(&config.instance_key_path, [other, other].concat()).unwrap()
            }
        }

        let error = match waveflow_server::initialize(&config).await {
            Ok(_) => panic!("{case}: a boot with no matching key went ahead"),
            Err(error) => format!("{error:#}"),
        };
        assert!(
            error.contains("nothing has been applied"),
            "{case}: {error}"
        );
        assert!(
            pre_migration_copies(&config).is_empty(),
            "{case}: a pair restore would refuse was kept: {:?}",
            pre_migration_copies(&config)
        );
        assert_eq!(
            applied_in(&config.database_path).await,
            behind,
            "{case}: migrated a database whose key is not there"
        );
        if damage.is_none() {
            assert!(error.contains("key adopt"), "{error}");
            assert!(
                !config.instance_key_path.exists(),
                "a key was minted beside a database bound to another"
            );
        }
    }
}

/// The copy a boot has just made is kept even when its name sorts below the
/// others — a clock that went back, or copies dated in the future.
#[tokio::test]
async fn the_copy_just_made_survives_a_clock_that_went_back() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = Config::for_data_dir(temp.path().join("data"));
    let (_, behind) = an_installation_behind(&config, None).await;
    let prefix = waveflow_server::migration_backup::PREFIX;
    // Three copies dated long after anything this boot can read off its clock.
    let future = ["99999999999991", "99999999999992", "99999999999993"];
    for stamp in future {
        std::fs::create_dir_all(config.data_dir.join(format!("{prefix}{stamp}"))).unwrap();
    }

    let state = waveflow_server::initialize(&config)
        .await
        .expect("an installation behind boots");
    state.db.pool().close().await;
    drop(state);

    let copies = pre_migration_copies(&config);
    assert_eq!(copies.len(), config.migration_backups, "{copies:?}");
    let made = copies
        .iter()
        .find(|copy| copy.join("waveflow.db").exists())
        .unwrap_or_else(|| panic!("the copy this boot made was deleted: {copies:?}"));
    assert_eq!(applied_in(&made.join("waveflow.db")).await, behind);
    // The room it needed came from the oldest of the others.
    assert!(!config
        .data_dir
        .join(format!("{prefix}{}", future[0]))
        .exists());
    for kept in &future[1..] {
        assert!(config.data_dir.join(format!("{prefix}{kept}")).exists());
    }
}

/// Zero copies is a setting: an operator with backups of their own upgrades
/// without one.
#[tokio::test]
async fn no_copy_is_taken_when_the_operator_set_none() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let mut config = Config::for_data_dir(temp.path().join("data"));
    config.migration_backups = 0;
    an_installation_behind(&config, None).await;
    waveflow_server::initialize(&config)
        .await
        .expect("an installation behind boots");
    assert!(pre_migration_copies(&config).is_empty());

    // And a fresh install, with copies on, has nothing to copy.
    let fresh = tempfile::tempdir().expect("temporary directory");
    let config = Config::for_data_dir(fresh.path().join("data"));
    waveflow_server::initialize(&config)
        .await
        .expect("a fresh install boots");
    assert!(pre_migration_copies(&config).is_empty());
}

/// How many migrations a database on disk has run.
///
/// Opened alone and closed again between readings: the key commands take the
/// file exclusively, and a connection this test forgot to close would make the
/// next one fail for a reason that has nothing to do with what it asserts.
async fn applied_in(path: &std::path::Path) -> i64 {
    let mut connection = Database::open_alone(path, std::time::Duration::from_secs(5))
        .await
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let applied = count(&mut connection, "SELECT COUNT(*) FROM _sqlx_migrations").await;
    connection.close().await.expect("close");
    applied
}
