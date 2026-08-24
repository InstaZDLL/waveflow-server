//! Derived identifiers: the specs that govern them and what a change costs.
//!
//! Split out of `v2_foundations.rs`.

use tempfile::TempDir;
use waveflow_server::authentication::now_ms;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;
use waveflow_server::Config;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

/// Fixture for the two remap shapes: a library whose recorded artist spec a
/// later boot can be shown to disagree with.
async fn remap_fixture(
    state: &waveflow_server::AppState,
    config: &Config,
    label: &str,
) -> (uuid::Uuid, uuid::Uuid) {
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account(label, &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join(format!("{label}-music"));
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Remap library",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 1, false).await.unwrap();
    state
        .db
        .apply_catalog_track(
            library_id,
            scan_id,
            &catalog_input(0, "Seed Artist"),
            None,
            false,
        )
        .await
        .unwrap();
    state.db.finish_scan_job(scan_id, 0).await.unwrap();
    (owner, library_id)
}

async fn seed_artist(
    state: &waveflow_server::AppState,
    library_id: uuid::Uuid,
    id: uuid::Uuid,
    name: &str,
) {
    sqlx::query(
        "INSERT INTO artist (id, library_id, name, canonical_name, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(library_id.to_string())
    .bind(name)
    .bind(name.to_lowercase().replace(' ', ""))
    .bind(now_ms())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
}

async fn seed_star(
    state: &waveflow_server::AppState,
    owner: uuid::Uuid,
    entity_id: uuid::Uuid,
    starred_at: i64,
) {
    sqlx::query(
        "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
         VALUES (?, 'artist', ?, ?)",
    )
    .bind(owner.to_string())
    .bind(entity_id.to_string())
    .bind(starred_at)
    .execute(state.db.pool())
    .await
    .unwrap();
}

async fn artist_stars(state: &waveflow_server::AppState, owner: uuid::Uuid) -> Vec<(String, i64)> {
    let mut rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT entity_id, starred_at FROM user_star WHERE user_id = ? AND entity_type = 'artist'",
    )
    .bind(owner.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    rows.sort();
    rows
}

/// The instance remembers its own settings between boots.
#[tokio::test]
async fn server_properties_round_trip_and_overwrite() {
    let (_temp, _config, state) = test_app().await;
    assert_eq!(state.db.server_property("pid.album").await.unwrap(), None);
    state
        .db
        .set_server_property("pid.album", "albumartistid,album")
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .server_property("pid.album")
            .await
            .unwrap()
            .as_deref(),
        Some("albumartistid,album")
    );
    state
        .db
        .set_server_property("pid.album", "musicbrainz_albumid")
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .server_property("pid.album")
            .await
            .unwrap()
            .as_deref(),
        Some("musicbrainz_albumid")
    );
}

/// A catalogue keyed under a rule the server no longer follows.
///
/// Nothing about the files reveals this: their bytes and timestamps are
/// unchanged, only the rule reading them moved. Comparing what the last scan
/// recorded against what this instance is configured with is the only way the
/// difference is visible at all.
#[tokio::test]
async fn a_changed_identity_rule_schedules_a_full_rescan_everywhere() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("identity", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    for name in ["First", "Second"] {
        let root = config.data_dir.join(name.to_lowercase());
        std::fs::create_dir_all(&root).unwrap();
        state
            .db
            .create_library(
                owner,
                name,
                &std::fs::canonicalize(&root).unwrap(),
                LibraryVisibility::Private,
                now_ms(),
            )
            .await
            .unwrap();
    }

    // A catalogue built before the property existed is an older server, not a
    // different rule: nothing is scheduled.
    assert_eq!(
        state
            .db
            .reconcile_catalog_identity(&config.pid)
            .await
            .unwrap(),
        0
    );

    // Once a scan has recorded what it used, agreeing costs nothing either.
    state
        .db
        .set_server_property("pid.album", config.pid.album.source())
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .reconcile_catalog_identity(&config.pid)
            .await
            .unwrap(),
        0
    );

    // The track spec is recorded but not compared: nothing derives from it,
    // so changing it re-identifies nothing and must not cost a rescan.
    state
        .db
        .set_server_property("pid.track", "folder,title")
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .reconcile_catalog_identity(&config.pid)
            .await
            .unwrap(),
        0,
        "a track spec that governs nothing schedules nothing"
    );

    // Disagreeing marks every library, because the rule is instance-wide.
    state
        .db
        .set_server_property("pid.album", "folder")
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .reconcile_catalog_identity(&config.pid)
            .await
            .unwrap(),
        2
    );
    let libraries = state.db.libraries_for_user(owner).await.unwrap();
    assert_eq!(libraries.len(), 2);
    for library in libraries {
        assert!(
            state.db.full_scan_requested(library.id).await.unwrap(),
            "{} was asked to rescan in full",
            library.name
        );
    }
}

#[tokio::test]
async fn a_changed_track_spec_drops_every_relocation_hint_without_a_rescan() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("respec", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("respec-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Kept Take.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Respec library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Respec library".into(),
        root_path: root,
    };

    run_scan(&state, owner, library.clone()).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    let track_id = tracks[0].id;
    state
        .services
        .set_star(owner, "track", track_id, true)
        .await
        .unwrap();
    let hint: Option<String> = sqlx::query_scalar("SELECT pid FROM track WHERE id = ?")
        .bind(track_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(hint.is_some(), "the first scan writes a hint");

    // A hint written under one spec must never be compared against a lookup
    // computed under another: two specs can evaluate to the same string, and
    // the hint would then hand a new file this track's identity — and this
    // track's favourite. So the stale values go.
    let altered = waveflow_server::pid::PidSpecs {
        album: config.pid.album.clone(),
        artist: config.pid.artist.clone(),
        track: waveflow_server::pid::PidSpec::parse("folder,title", true).unwrap(),
    };
    let rescans = state.db.reconcile_catalog_identity(&altered).await.unwrap();
    assert_eq!(
        rescans, 0,
        "a track spec change re-identifies nothing and must not charge a full rescan"
    );
    assert!(!state.db.full_scan_requested(library_id).await.unwrap());
    let hint: Option<String> = sqlx::query_scalar("SELECT pid FROM track WHERE id = ?")
        .bind(track_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(
        hint.is_none(),
        "no hint from the previous spec may survive to be matched against"
    );

    // And one ordinary scan puts it back, which is why clearing costs nothing.
    run_scan(&state, owner, library).await;
    let hint: Option<String> = sqlx::query_scalar("SELECT pid FROM track WHERE id = ?")
        .bind(track_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(
        hint.is_some(),
        "the backfill restores the hint without a full scan"
    );

    let starred = state.services.starred(owner, &[]).await.unwrap();
    assert_eq!(
        starred.songs.iter().map(|song| song.id).collect::<Vec<_>>(),
        vec![track_id],
        "clearing a hint must not disturb what it points at"
    );
}

#[tokio::test]
async fn a_changed_artist_spec_carries_the_favourite_onto_the_new_identifier() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("remap", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("remap-music");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Remap library",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 1, false).await.unwrap();
    state
        .db
        .apply_catalog_track(
            library_id,
            scan_id,
            &catalog_input(0, "Nova Kern"),
            None,
            false,
        )
        .await
        .unwrap();
    // The rules the catalogue was written under are recorded when the scan
    // completes, which is what a later boot compares against.
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let artist_id: String =
        sqlx::query_scalar("SELECT id FROM artist WHERE library_id = ? AND name = 'Nova Kern'")
            .bind(library_id.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    let artist_id = uuid::Uuid::parse_str(&artist_id).unwrap();
    state
        .services
        .set_star(owner, "artist", artist_id, true)
        .await
        .unwrap();

    let altered = waveflow_server::pid::PidSpecs {
        album: config.pid.album.clone(),
        track: config.pid.track.clone(),
        artist: waveflow_server::pid::PidSpec::parse("albumartistid,title", false).unwrap(),
    };
    let expected = altered.artist_id(library_id, "Nova Kern");
    assert_ne!(
        expected, artist_id,
        "the altered spec has to actually move the identifier for this to test anything"
    );

    let libraries = state.db.reconcile_catalog_identity(&altered).await.unwrap();
    assert_eq!(
        libraries, 1,
        "an artist spec change still re-identifies, and still costs a full rescan"
    );

    let starred: Vec<String> = sqlx::query_scalar(
        "SELECT entity_id FROM user_star WHERE user_id = ? AND entity_type = 'artist'",
    )
    .bind(owner.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        starred,
        vec![expected.to_string()],
        "the favourite must name the identifier the new rule derives, not the one it replaced"
    );
}

#[tokio::test]
async fn a_remap_whose_targets_chain_does_not_change_who_owns_what() {
    let (_temp, config, state) = test_app().await;
    let (owner, library_id) = remap_fixture(&state, &config, "chained").await;

    let altered = waveflow_server::pid::PidSpecs {
        album: config.pid.album.clone(),
        track: config.pid.track.clone(),
        artist: waveflow_server::pid::PidSpec::parse("albumartistid,title", false).unwrap(),
    };
    // Built by hand: no spec this engine can parse makes one artist's new
    // identifier another's old one, because only `albumartistid` carries a
    // value for an artist. The property still has to hold, because the code
    // cannot see that and a wider `PidSource` would make it reachable.
    let first_row = uuid::Uuid::new_v4();
    let first_new = altered.artist_id(library_id, "Chain One");
    let second_new = altered.artist_id(library_id, "Chain Two");
    assert_ne!(first_row, first_new);
    assert_ne!(first_new, second_new);
    seed_artist(&state, library_id, first_row, "Chain One").await;
    // Its row id is the identifier the first artist is about to move onto.
    seed_artist(&state, library_id, first_new, "Chain Two").await;
    seed_star(&state, owner, first_row, 111).await;
    seed_star(&state, owner, first_new, 222).await;

    state.db.reconcile_catalog_identity(&altered).await.unwrap();

    let mut expected = vec![(first_new.to_string(), 111), (second_new.to_string(), 222)];
    expected.sort();
    assert_eq!(
        artist_stars(&state, owner).await,
        expected,
        "each favourite has to land on its own artist's new identifier, whatever \
         order the rows came back in"
    );
}

#[tokio::test]
async fn two_artists_folding_onto_one_identifier_keep_a_single_favourite() {
    let (_temp, config, state) = test_app().await;
    let (owner, library_id) = remap_fixture(&state, &config, "folded").await;

    // `title` names nothing for an artist, so every artist evaluates to the
    // same empty string and the whole library folds onto one identifier. A
    // degenerate spec, and exactly the shape the collision handling is for.
    let altered = waveflow_server::pid::PidSpecs {
        album: config.pid.album.clone(),
        track: config.pid.track.clone(),
        artist: waveflow_server::pid::PidSpec::parse("title", false).unwrap(),
    };
    let folded = altered.artist_id(library_id, "Fold One");
    assert_eq!(folded, altered.artist_id(library_id, "Fold Two"));

    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();
    seed_artist(&state, library_id, first, "Fold One").await;
    seed_artist(&state, library_id, second, "Fold Two").await;
    seed_star(&state, owner, first, 111).await;
    seed_star(&state, owner, second, 222).await;

    state.db.reconcile_catalog_identity(&altered).await.unwrap();

    let stars = artist_stars(&state, owner).await;
    assert_eq!(
        stars.len(),
        1,
        "the two rows collide on the primary key and one is dropped: {stars:?}"
    );
    // Which also says no row was left staged: a staged value is the folded
    // identifier behind a prefix, and this is the identifier itself.
    assert_eq!(stars[0].0, folded.to_string());
}

/// What `DROP TABLE` does to the children of the table being dropped.
///
/// SQLite performs an implicit `DELETE FROM` before dropping a table when
/// foreign keys are on, and that delete fires `ON DELETE CASCADE` on every
/// child. `defer_foreign_keys` does not help: it defers constraint *checking*,
/// not referential *actions*. A rebuild of `artist` therefore empties
/// `track_artist` before anything can copy it — which is what the participants
/// migration would have done to every credit in every existing library,
/// leaving them to the rescan and to nothing else.
///
/// What survives it is a table made by `CREATE ... AS SELECT`: it carries no
/// constraints, so it is nobody's child.
#[tokio::test]
async fn a_table_rebuild_carries_its_children_out_of_the_cascade() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("probe.db");
    let url = format!(
        "sqlite://{}?mode=rwc",
        path.to_string_lossy().replace('\\', "/")
    );
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    for statement in [
        "CREATE TABLE artist (id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, \
           UNIQUE (id, name)) STRICT",
        "CREATE TABLE credit (artist_id TEXT NOT NULL, name TEXT NOT NULL, \
           FOREIGN KEY (artist_id, name) REFERENCES artist(id, name) ON DELETE CASCADE) STRICT",
        "INSERT INTO artist (id, name) VALUES ('a', 'Nova Kern')",
        "INSERT INTO credit (artist_id, name) VALUES ('a', 'Nova Kern')",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    let mut tx = pool.begin().await.unwrap();
    for statement in [
        "PRAGMA defer_foreign_keys = ON",
        "CREATE TABLE credit_carry AS SELECT * FROM credit",
        "CREATE TABLE artist_rebuilt (id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, \
           UNIQUE (id, name)) STRICT",
        "INSERT INTO artist_rebuilt SELECT id, name FROM artist",
        "DROP TABLE artist",
        "ALTER TABLE artist_rebuilt RENAME TO artist",
    ] {
        sqlx::query(statement).execute(&mut *tx).await.unwrap();
    }
    // The cascade fires. Stated rather than assumed, because assuming it did
    // not is what put a migration in this branch that copied an empty table.
    let cascaded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM credit")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(cascaded, 0, "dropping the parent emptied the child");
    let carried: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM credit_carry")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(carried, 1, "and the carry kept what the cascade took");
    tx.commit().await.unwrap();
}
