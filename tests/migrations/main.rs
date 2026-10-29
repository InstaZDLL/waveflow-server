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
//! in `src/storage/database/tests.rs` already applies a prefix and migrates from there. It is
//! pinned to **one** boundary — the participants rebuild it was written for —
//! and asserts raw counts. This asks the general question instead, from every
//! schema a tag actually shipped, and answers it through the services rather
//! than through `SELECT COUNT(*)`: a row that survives a migration and is no
//! longer reachable by a query has not survived in any sense a user would
//! recognise.

use sqlx::migrate::Migrate;
use sqlx::Connection;
use sqlx::SqliteConnection;
use waveflow_server::services::AlbumListQuery;
use waveflow_server::services::BrowsePage;
use waveflow_server::storage::database::Database;
use waveflow_server::storage::database::MIGRATOR;
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
const RELEASED_SCHEMAS: [(&str, i64); 3] = [
    ("v2.0.0-beta.0", 20_260_823_020_000),
    ("v2.0.0-beta.1", 20_260_914_030_000),
    ("v2.0.0", 20_261_028_000_000),
];

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

mod copies;
mod upgrades;
