//! Splitting a joined artist credit, by decision, and "Needs attention"
//! (RFC-015).
//!
//! The accounts here are rows written by hand and reach the routes through an
//! API token, as in `tests/catalog/listings.rs`: nothing in this target authenticates
//! with a password, so nothing in it hands a literal to a hash.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use support::*;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::services::{
    AttentionCategory, BrowsePage, CreditDecisionKind, TrackMetadataPatch,
};
use waveflow_server::storage::catalog::CatalogTrackInput;
use waveflow_server::storage::database::{LibraryRole, LibraryVisibility};
use waveflow_server::AppState;

/// One credited row: track, role, position, name, sort form.
type Row = (String, String, i64, String, Option<String>);

struct Shelf {
    state: AppState,
    config: waveflow_server::Config,
    owner: Uuid,
    library: Uuid,
    scan: Uuid,
    _temp: tempfile::TempDir,
}

async fn account(state: &AppState, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(now)
    .bind(now)
    .execute(state.db.pool())
    .await
    .unwrap();
    id
}

async fn shelf(name: &str) -> Shelf {
    let (temp, config, state) = test_app().await;
    let owner = account(&state, name).await;
    let music = config.data_dir.join(name);
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            name,
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    Shelf {
        state,
        config,
        owner,
        library,
        scan,
        _temp: temp,
    }
}

/// A file crediting `artist`, on its own album unless one is named, with no
/// album-artist tag and no compilation flag: the album then hangs off the
/// track's own credit, which is the case a split could have moved.
fn file(index: usize, artist: &str, album: Option<&str>) -> CatalogTrackInput {
    let mut input = catalog_input(index, artist);
    input.title = format!("Track {index}");
    input.album = album.map(str::to_owned);
    input.is_compilation = false;
    input.genre = None;
    input
}

impl Shelf {
    async fn apply(&self, input: &CatalogTrackInput) {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM track WHERE library_id = ? AND relative_path = ?")
                .bind(self.library.to_string())
                .bind(&input.relative_path)
                .fetch_optional(self.state.db.pool())
                .await
                .unwrap();
        self.state
            .db
            .apply_catalog_track(
                self.library,
                self.scan,
                input,
                existing.map(|id| Uuid::parse_str(&id).unwrap()),
                false,
            )
            .await
            .unwrap();
    }

    /// The end of a scan, in the scanner's order: sort names first, because
    /// the derivations index each artist with its sort name.
    async fn settle(&self) {
        self.state
            .db
            .consolidate_sort_names(self.library)
            .await
            .unwrap();
        self.state
            .db
            .consolidate_catalog_derivations(self.library)
            .await
            .unwrap();
    }

    async fn artist(&self, name: &str) -> Uuid {
        let id: String =
            sqlx::query_scalar("SELECT id FROM artist WHERE library_id = ? AND name = ?")
                .bind(self.library.to_string())
                .bind(name)
                .fetch_one(self.state.db.pool())
                .await
                .unwrap_or_else(|_| panic!("no artist named {name:?}"));
        Uuid::parse_str(&id).unwrap()
    }

    async fn artist_names(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT name FROM artist WHERE library_id = ? ORDER BY name")
            .bind(self.library.to_string())
            .fetch_all(self.state.db.pool())
            .await
            .unwrap()
    }

    async fn split(&self, name: &str) -> Uuid {
        let artist = self.artist(name).await;
        self.state
            .services
            .decide_credit(self.owner, self.library, artist, CreditDecisionKind::Split)
            .await
            .unwrap()
            .id
    }

    async fn undo(&self, decision: Uuid) {
        self.state
            .services
            .remove_credit_decision(self.owner, self.library, decision)
            .await
            .unwrap();
    }

    /// Every credited row of the library, by file, role and position.
    async fn rows(&self) -> Vec<Row> {
        sqlx::query_as(
            "SELECT t.relative_path, tp.role, tp.position, ar.name, tp.sort_name \
             FROM track_participant tp JOIN track t ON t.id = tp.track_id \
             JOIN artist ar ON ar.id = tp.artist_id \
             WHERE t.library_id = ? ORDER BY t.relative_path, tp.role, tp.position",
        )
        .bind(self.library.to_string())
        .fetch_all(self.state.db.pool())
        .await
        .unwrap()
    }

    /// One file's names in one role, in order.
    async fn credited(&self, index: usize, role: &str) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT ar.name FROM track_participant tp JOIN track t ON t.id = tp.track_id \
             JOIN artist ar ON ar.id = tp.artist_id \
             WHERE t.library_id = ? AND t.relative_path = ? AND tp.role = ? \
             ORDER BY tp.position",
        )
        .bind(self.library.to_string())
        .bind(format!("track-{index}.flac"))
        .bind(role)
        .fetch_all(self.state.db.pool())
        .await
        .unwrap()
    }

    async fn token(&self, user: Uuid, name: &str) -> String {
        self.state
            .services
            .create_api_token(user, &[], name, "credit decisions", &[])
            .await
            .unwrap()
            .1
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[tokio::test]
async fn a_split_lands_on_the_artists_the_parts_name() {
    let shelf = shelf("split-lands").await;
    shelf
        .apply(&file(0, "Ashreveal, Ashish Swargiary", Some("Duo")))
        .await;
    shelf.apply(&file(1, "Ashreveal", Some("Solo"))).await;
    shelf.settle().await;
    let solo = shelf.artist("Ashreveal").await;

    shelf.split("Ashreveal, Ashish Swargiary").await;

    assert_eq!(
        shelf.credited(0, "artist").await,
        names(&["Ashreveal", "Ashish Swargiary"])
    );
    assert_eq!(
        shelf.artist("Ashreveal").await,
        solo,
        "the split credit lands on the artist the solo tracks already name"
    );
    assert_eq!(
        shelf.artist_names().await,
        names(&["Ashish Swargiary", "Ashreveal"]),
        "the joined artist is swept once nothing credits it"
    );
}

#[tokio::test]
async fn an_album_keeps_its_id_through_a_split() {
    let shelf = shelf("album-keeps").await;
    shelf
        .apply(&file(0, "Zedd, Alessia Cara", Some("Stay")))
        .await;
    shelf.settle().await;
    let before: (String, String) =
        sqlx::query_as("SELECT id, album_artist_name FROM album WHERE library_id = ?")
            .bind(shelf.library.to_string())
            .fetch_one(shelf.state.db.pool())
            .await
            .unwrap();

    shelf.split("Zedd, Alessia Cara").await;
    // Right after the decision, and again after the scan that reads the file
    // afterwards: the decision's own pass and the scan each write the album.
    for moment in ["after the decision", "after a rescan"] {
        if moment == "after a rescan" {
            shelf
                .apply(&file(0, "Zedd, Alessia Cara", Some("Stay")))
                .await;
            shelf.settle().await;
        }
        let after: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT id, album_artist_name, album_artist_id FROM album WHERE library_id = ?",
        )
        .bind(shelf.library.to_string())
        .fetch_all(shelf.state.db.pool())
        .await
        .unwrap();
        assert_eq!(
            after.len(),
            1,
            "{moment}: one album, not a second one minted beside it"
        );
        assert_eq!(
            (after[0].0.clone(), after[0].1.clone()),
            before,
            "{moment}: the album's id and display stay what the file said"
        );
        assert_eq!(
            after[0].2,
            shelf.artist("Zedd").await.to_string(),
            "{moment}: the album hangs off the first part"
        );
        assert_eq!(
            shelf.credited(0, "albumartist").await,
            names(&["Zedd", "Alessia Cara"]),
            "{moment}"
        );
    }
}

#[tokio::test]
async fn a_scan_applies_a_decision_to_every_file_it_reads() {
    let shelf = shelf("scan-applies").await;
    shelf
        .apply(&file(0, "Ice Spice, Central Cee", Some("Did It First")))
        .await;
    shelf.settle().await;
    shelf.split("Ice Spice, Central Cee").await;

    // A file the scan reads for the first time, and one it reads again
    // because it changed: both come back split, which the desktop's own
    // split never survived.
    shelf
        .apply(&file(1, "Ice Spice, Central Cee", Some("Single")))
        .await;
    let mut changed = file(0, "Ice Spice, Central Cee", Some("Did It First"));
    changed.modified_at += 1;
    changed.title = "Did It First (retagged)".into();
    shelf.apply(&changed).await;
    shelf.settle().await;

    for index in [0, 1] {
        assert_eq!(
            shelf.credited(index, "artist").await,
            names(&["Ice Spice", "Central Cee"]),
            "file {index}"
        );
    }
    assert!(
        !shelf
            .artist_names()
            .await
            .contains(&"Ice Spice, Central Cee".to_owned()),
        "no scan brings the joined artist back"
    );
}

#[tokio::test]
async fn an_undo_restores_what_a_scan_without_the_decision_writes() {
    let shelf = shelf("undo-restores").await;
    let mut lead = file(0, "X, Y", Some("Pair"));
    lead.sort_artist = Some("Xsort, Ysort".into());
    shelf.apply(&lead).await;
    shelf.apply(&file(1, "Z feat. X, Y", Some("Guest"))).await;
    shelf.settle().await;
    let whole = shelf.rows().await;

    let decision = shelf.split("X, Y").await;
    let split = shelf.rows().await;
    assert_ne!(split, whole);
    // A rescan of the same files writes exactly what the decision wrote.
    shelf.apply(&lead).await;
    shelf.apply(&file(1, "Z feat. X, Y", Some("Guest"))).await;
    shelf.settle().await;
    assert_eq!(shelf.rows().await, split, "a split and a rescan agree");

    shelf.undo(decision).await;
    assert_eq!(shelf.rows().await, whole, "the undo is a scan without it");
    assert!(shelf.artist_names().await.contains(&"X, Y".to_owned()));
    assert!(shelf
        .state
        .services
        .credit_decisions(shelf.owner, shelf.library)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn two_credits_sharing_a_part_keep_the_files_order_whichever_is_undone() {
    let shelf = shelf("shared-part").await;
    let mut both = file(0, "ignored", Some("Shared"));
    // The plural tag is never cut by the first step, so each value is one
    // credit and the two share their first fragment. Out of alphabetical
    // order on purpose, at every level: an order taken from the names rather
    // than from the file would put every list below the other way round.
    both.artists = names(&["Z, Y", "Z, X"]);
    shelf.apply(&both).await;
    shelf.settle().await;

    let first = shelf.split("Z, Y").await;
    let second = shelf.split("Z, X").await;
    assert_eq!(shelf.credited(0, "artist").await, names(&["Z", "Y", "X"]));

    shelf.undo(second).await;
    assert_eq!(
        shelf.credited(0, "artist").await,
        names(&["Z", "Y", "Z, X"])
    );

    let second = shelf.split("Z, X").await;
    shelf.undo(first).await;
    assert_eq!(
        shelf.credited(0, "artist").await,
        names(&["Z, Y", "Z", "X"]),
        "the order is the file's, not what the shared part became"
    );
    shelf.undo(second).await;
    assert_eq!(shelf.credited(0, "artist").await, names(&["Z, Y", "Z, X"]));
}

#[tokio::test]
async fn a_correction_survives_a_split_and_a_rescan() {
    let shelf = shelf("correction-final").await;
    let mut input = file(0, "Hanumankind, Kalmi", Some("Big Dawgs"));
    input.album_artist = Some("Hanumankind, Kalmi".into());
    shelf.apply(&input).await;
    shelf.settle().await;
    let track: String = sqlx::query_scalar("SELECT id FROM track WHERE library_id = ?")
        .bind(shelf.library.to_string())
        .fetch_one(shelf.state.db.pool())
        .await
        .unwrap();
    shelf
        .state
        .services
        .set_track_metadata(
            shelf.owner,
            Uuid::parse_str(&track).unwrap(),
            TrackMetadataPatch {
                artists: Some(Some(names(&["Hanumankind, Kalmi"]))),
                ..TrackMetadataPatch::default()
            },
            None,
        )
        .await
        .unwrap();

    shelf.split("Hanumankind, Kalmi").await;
    // Right after the decision, whose own pass rewrites the track, and after
    // the scan that reads the file again: each could undo the correction.
    for moment in ["after the decision", "after a rescan"] {
        if moment == "after a rescan" {
            shelf.apply(&input).await;
            shelf.settle().await;
        }
        assert_eq!(
            shelf.credited(0, "artist").await,
            names(&["Hanumankind, Kalmi"]),
            "{moment}: a list someone typed is final"
        );
        assert_eq!(
            shelf.credited(0, "albumartist").await,
            names(&["Hanumankind", "Kalmi"]),
            "{moment}: the roles the correction does not cover still follow the decision"
        );
    }
}

#[tokio::test]
async fn a_split_keeps_each_parts_sort_form() {
    let shelf = shelf("sort-forms").await;
    let mut input = file(0, "A, B, B, C", Some("Sorted"));
    input.sort_artist = Some("a, b, b, c".into());
    shelf.apply(&input).await;
    shelf.settle().await;

    shelf.split("A, B, B, C").await;

    let sorts: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT ar.name, tp.sort_name FROM track_participant tp \
         JOIN artist ar ON ar.id = tp.artist_id \
         WHERE tp.library_id = ? AND tp.role = 'artist' ORDER BY tp.position",
    )
    .bind(shelf.library.to_string())
    .fetch_all(shelf.state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        sorts,
        vec![
            ("A".to_owned(), Some("a".to_owned())),
            ("B".to_owned(), Some("b".to_owned())),
            ("C".to_owned(), Some("c".to_owned())),
        ]
    );
    // And the artists themselves carry them, in the search index as well:
    // the index is rebuilt from the artist rows, so the sort names have to be
    // voted before it is, as at the end of a scan.
    let indexed: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT ar.name, COALESCE(ar.sort_name, ''), fts.sort_name FROM artist ar          JOIN artist_fts fts ON fts.artist_id = ar.id          WHERE ar.library_id = ? ORDER BY ar.name",
    )
    .bind(shelf.library.to_string())
    .fetch_all(shelf.state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        indexed,
        vec![
            ("A".to_owned(), "a".to_owned(), "a".to_owned()),
            ("B".to_owned(), "b".to_owned(), "b".to_owned()),
            ("C".to_owned(), "c".to_owned(), "c".to_owned()),
        ]
    );
}

#[tokio::test]
async fn a_retagged_spelling_is_still_split_and_still_undone() {
    let shelf = shelf("retagged").await;
    shelf
        .apply(&file(0, "Drake, Lil Durk", Some("Laugh Now")))
        .await;
    shelf.settle().await;
    let decision = shelf.split("Drake, Lil Durk").await;

    let mut retagged = file(0, "drake, lil durk", Some("Laugh Now"));
    retagged.modified_at += 1;
    shelf.apply(&retagged).await;
    shelf.settle().await;
    assert_eq!(
        shelf.credited(0, "artist").await,
        names(&["Drake", "Lil Durk"]),
        "the key folds case, so the new spelling is the same credit"
    );

    shelf.undo(decision).await;
    assert_eq!(
        shelf.credited(0, "artist").await,
        names(&["drake, lil durk"]),
        "the undo restores this file's own spelling"
    );
}

#[tokio::test]
async fn favourites_and_ratings_follow_the_parts_through_the_journal() {
    let shelf = shelf("favourites-follow").await;
    shelf
        .apply(&file(0, "Tame Impala, JENNIE", Some("Dracula")))
        .await;
    shelf.apply(&file(1, "JENNIE", Some("Ruby"))).await;
    shelf.settle().await;
    let joined = shelf.artist("Tame Impala, JENNIE").await;
    let jennie = shelf.artist("JENNIE").await;
    let services = &shelf.state.services;
    services
        .set_star(shelf.owner, "artist", joined, true)
        .await
        .unwrap();
    services
        .set_rating(shelf.owner, "artist", joined, 4)
        .await
        .unwrap();
    services
        .set_rating(shelf.owner, "artist", jennie, 2)
        .await
        .unwrap();

    shelf.split("Tame Impala, JENNIE").await;
    let tame = shelf.artist("Tame Impala").await;

    let starred: Vec<String> = sqlx::query_scalar(
        "SELECT entity_id FROM user_star WHERE user_id = ? AND entity_type = 'artist' \
         ORDER BY entity_id",
    )
    .bind(shelf.owner.to_string())
    .fetch_all(shelf.state.db.pool())
    .await
    .unwrap();
    let mut expected = vec![tame.to_string(), jennie.to_string()];
    expected.sort();
    assert_eq!(starred, expected, "the favourite is on every part");

    let rated: Vec<(String, i64)> = sqlx::query_as(
        "SELECT entity_id, rating FROM user_rating WHERE user_id = ? AND entity_type = 'artist' \
         ORDER BY entity_id",
    )
    .bind(shelf.owner.to_string())
    .fetch_all(shelf.state.db.pool())
    .await
    .unwrap();
    let mut expected = vec![(tame.to_string(), 4), (jennie.to_string(), 2)];
    expected.sort();
    assert_eq!(rated, expected, "a part rated on its own keeps its rating");

    // The journal says what moved, so a mirror converges on the same state.
    let journal: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT entity_type, entity_id, action FROM sync_event WHERE user_id = ? \
         AND entity_id IN (?, ?, ?) ORDER BY cursor",
    )
    .bind(shelf.owner.to_string())
    .bind(joined.to_string())
    .bind(tame.to_string())
    .bind(jennie.to_string())
    .fetch_all(shelf.state.db.pool())
    .await
    .unwrap();
    for entry in [
        ("favorite", joined, "delete"),
        ("favorite", tame, "upsert"),
        ("favorite", jennie, "upsert"),
        ("rating", joined, "delete"),
        ("rating", tame, "upsert"),
    ] {
        let entry = (entry.0.to_owned(), entry.1.to_string(), entry.2.to_owned());
        assert!(
            journal.contains(&entry),
            "{entry:?} missing from {journal:?}"
        );
    }
}

#[tokio::test]
async fn the_feed_announces_what_a_split_moved() {
    let shelf = shelf("feed").await;
    shelf.apply(&file(0, "Curio, Lucy", Some("Echoes"))).await;
    shelf.settle().await;
    let joined = shelf.artist("Curio, Lucy").await;
    let cursor: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(cursor), 0) FROM library_event")
        .fetch_one(shelf.state.db.pool())
        .await
        .unwrap();

    shelf.split("Curio, Lucy").await;

    let events: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT entity_type, entity_id, action FROM library_event \
         WHERE library_id = ? AND cursor > ? ORDER BY cursor",
    )
    .bind(shelf.library.to_string())
    .bind(cursor)
    .fetch_all(shelf.state.db.pool())
    .await
    .unwrap();
    let kinds: Vec<(&str, &str)> = events
        .iter()
        .map(|(kind, _, action)| (kind.as_str(), action.as_str()))
        .collect();
    assert!(kinds.contains(&("track", "upsert")), "{events:?}");
    assert!(kinds.contains(&("album", "upsert")), "{events:?}");
    assert!(
        events.contains(&("artist".into(), joined.to_string(), "delete".into())),
        "{events:?}"
    );
}

/// The migration fills `track_credit` from the participants. Rebuilding it
/// that way on a scanned library must answer what the scan's own credits
/// answer, under no decision, a split and its undo — including a file that
/// repeats a credit and one whose credits share a part.
#[tokio::test]
async fn the_migrations_copy_gives_what_a_scan_gives() {
    let migration = include_str!("../../migrations-v2/20261024020000_artist_credit_decision.sql");
    let copy = &migration[migration
        .find("INSERT INTO track_credit")
        .expect("the migration copies the participants")..];

    let inputs = || {
        let mut repeated = file(0, "ignored", Some("Twice"));
        repeated.artists = names(&["A, C", "A, C"]);
        let mut shared = file(1, "ignored", Some("Shared"));
        shared.artists = names(&["A, B", "A, C"]);
        let mut sorted = file(2, "A, B", Some("Sorted"));
        sorted.sort_artist = Some("a, b".into());
        [repeated, shared, sorted]
    };
    let mut answers = Vec::new();
    for copied in [false, true] {
        let shelf = shelf(if copied { "copied" } else { "scanned" }).await;
        for input in inputs() {
            shelf.apply(&input).await;
        }
        shelf.settle().await;
        if copied {
            sqlx::query("DELETE FROM track_credit")
                .execute(shelf.state.db.pool())
                .await
                .unwrap();
            sqlx::raw_sql(copy)
                .execute(shelf.state.db.pool())
                .await
                .unwrap();
        }
        let whole = shelf.rows().await;
        let first = shelf.split("A, B").await;
        let second = shelf.split("A, C").await;
        let split = shelf.rows().await;
        shelf.undo(first).await;
        let half = shelf.rows().await;
        shelf.undo(second).await;
        let undone = shelf.rows().await;
        assert_eq!(undone, whole, "copied: {copied}");
        answers.push((whole, split, half));
    }
    assert_eq!(answers[0], answers[1]);
}

#[tokio::test]
async fn the_candidates_are_the_desktops_sorted_as_the_desktop_sorts_them() {
    let shelf = shelf("candidates").await;
    shelf
        .apply(&file(0, "Megan Thee Stallion, TWICE", Some("Mamushi")))
        .await;
    shelf.apply(&file(1, "TWICE", Some("Fancy"))).await;
    shelf
        .apply(&file(2, "Didier Super, The Aro String Band", Some("A")))
        .await;
    shelf
        .apply(&file(3, "Didier Super, The Aro String Band", Some("B")))
        .await;
    shelf
        .apply(&file(4, "Tyler, The Creator", Some("Igor")))
        .await;
    shelf
        .apply(&file(5, "Simon & Garfunkel", Some("Bookends")))
        .await;
    shelf
        .apply(&file(
            6,
            "Ian Asher, Olly Alexander (Years, Years)",
            Some("Sun"),
        ))
        .await;
    shelf.apply(&file(7, "Solo,", Some("Alone"))).await;
    shelf.settle().await;

    let candidates = shelf
        .state
        .services
        .credit_candidates(shelf.owner, shelf.library)
        .await
        .unwrap();
    let listed: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        listed,
        [
            "Megan Thee Stallion, TWICE",
            "Didier Super, The Aro String Band",
            "Ian Asher, Olly Alexander (Years, Years)",
            "Tyler, The Creator",
        ],
        "a known fragment first, then the most tracks, then the name"
    );
    let megan = &candidates[0];
    assert_eq!(megan.track_count, 1);
    assert_eq!(megan.fragments[1].name, "TWICE");
    assert_eq!(
        megan.fragments[1].artist_id,
        Some(shelf.artist("TWICE").await)
    );
    assert_eq!(megan.fragments[0].artist_id, None);

    let tyler = shelf.artist("Tyler, The Creator").await;
    shelf
        .state
        .services
        .decide_credit(shelf.owner, shelf.library, tyler, CreditDecisionKind::Keep)
        .await
        .unwrap();
    assert!(
        shelf
            .state
            .services
            .credit_candidates(shelf.owner, shelf.library)
            .await
            .unwrap()
            .iter()
            .all(|candidate| candidate.name != "Tyler, The Creator"),
        "a keep takes the credit off the list"
    );
    assert_eq!(
        shelf.credited(4, "artist").await,
        names(&["Tyler, The Creator"]),
        "and changes nothing it credits"
    );
}

#[tokio::test]
async fn only_owners_and_managers_reach_the_page_and_its_decisions() {
    let shelf = shelf("page-access").await;
    shelf
        .apply(&file(0, "Chris Lake, Abel Balder", Some("A")))
        .await;
    shelf.apply(&file(1, "Plain", Some("B"))).await;
    shelf.settle().await;
    let listener = account(&shelf.state, "listener").await;
    let manager = account(&shelf.state, "manager").await;
    for (user, role) in [
        (listener, LibraryRole::Listener),
        (manager, LibraryRole::Manager),
    ] {
        shelf
            .state
            .db
            .add_library_member(shelf.owner, shelf.library, user, role, now_ms())
            .await
            .unwrap();
    }
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let library = shelf.library;
    let send = |token: String, request: Request<Body>| {
        let router = router.clone();
        async move {
            let (mut parts, body) = request.into_parts();
            parts
                .headers
                .insert("authorization", format!("Bearer {token}").parse().unwrap());
            router
                .oneshot(Request::from_parts(parts, body))
                .await
                .unwrap()
        }
    };
    let listening = shelf.token(listener, "listener").await;
    let managing = shelf.token(manager, "manager").await;
    let joined = shelf.artist("Chris Lake, Abel Balder").await;
    let plain = shelf.artist("Plain").await;
    let get = |path: &str| {
        Request::get(format!("/api/v2/libraries/{library}{path}"))
            .body(Body::empty())
            .unwrap()
    };
    let decide = |artist: Uuid| {
        json_request(
            &format!("/api/v2/libraries/{library}/credit-decisions"),
            json!({ "artist_id": artist, "decision": "split" }),
        )
    };

    for path in [
        "/attention",
        "/attention/credits",
        "/attention/tracks?category=missing_year",
        "/credit-decisions",
    ] {
        assert_eq!(
            send(listening.clone(), get(path)).await.status(),
            StatusCode::NOT_FOUND,
            "a listener is told nothing at {path}"
        );
        assert_eq!(
            send(managing.clone(), get(path)).await.status(),
            StatusCode::OK,
            "a manager reads {path}"
        );
    }
    assert_eq!(
        send(listening.clone(), decide(joined)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            managing.clone(),
            get("/attention/tracks?category=split_credits")
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "credits are not a track category"
    );
    assert_eq!(
        send(managing.clone(), decide(plain)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a name with nothing to split is not a candidate"
    );

    let created = send(managing.clone(), decide(joined)).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    assert_eq!(created["decision"], "split");
    assert_eq!(created["parts"], json!(["Chris Lake", "Abel Balder"]));
    assert_eq!(created["decided_by"], "manager");

    // Decided, the joined artist is gone: deciding again names nothing.
    assert_eq!(
        send(managing.clone(), decide(joined)).await.status(),
        StatusCode::NOT_FOUND
    );
    let id = created["id"].as_str().unwrap().to_owned();
    let remove = || {
        Request::delete(format!("/api/v2/libraries/{library}/credit-decisions/{id}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(listening.clone(), remove()).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(managing.clone(), remove()).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(managing.clone(), remove()).await.status(),
        StatusCode::NOT_FOUND
    );

    // Undone, the credit can be decided again; a part of it is no candidate.
    assert_eq!(
        send(managing.clone(), decide(joined)).await.status(),
        StatusCode::CREATED
    );
    let keep = json_request(
        &format!("/api/v2/libraries/{library}/credit-decisions"),
        json!({ "artist_id": shelf.artist("Chris Lake").await, "decision": "keep" }),
    );
    assert_eq!(
        send(managing.clone(), keep).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn a_second_decision_on_one_credit_is_a_conflict() {
    let shelf = shelf("conflict").await;
    shelf.apply(&file(0, "Jonas Blue, Malive", Some("A"))).await;
    shelf.settle().await;
    let joined = shelf.artist("Jonas Blue, Malive").await;
    let services = &shelf.state.services;
    services
        .decide_credit(shelf.owner, shelf.library, joined, CreditDecisionKind::Keep)
        .await
        .unwrap();
    let again = services
        .decide_credit(
            shelf.owner,
            shelf.library,
            joined,
            CreditDecisionKind::Split,
        )
        .await;
    // Asserted on the variant alone: printing the result would put the
    // decision's identifiers in the failure message for nothing.
    assert!(
        matches!(
            again,
            Err(waveflow_server::services::ServiceError::Conflict)
        ),
        "a second decision on one credit is a conflict"
    );
}

#[tokio::test]
async fn each_track_category_lists_what_its_criterion_names() {
    let shelf = shelf("categories").await;
    let mut tracks = Vec::new();
    // A gap on disc one: 1, 2, 4.
    for (index, number) in [(0, 1), (1, 2), (2, 4)] {
        let mut input = file(index, "Gap Band", Some("Gaps"));
        input.track_number = Some(number);
        tracks.push(input);
    }
    // Two tracks numbered 1 on the same disc.
    for index in [3, 4] {
        let mut input = file(index, "Dup Band", Some("Dups"));
        input.track_number = Some(1);
        tracks.push(input);
    }
    // Two years on one album.
    for (index, year) in [(5, 2001), (6, 2002)] {
        let mut input = file(index, "Year Band", Some("Years"));
        input.year = Some(year);
        tracks.push(input);
    }
    // No year, no number, no album.
    let mut bare = file(7, "Bare Band", None);
    bare.year = None;
    bare.track_number = None;
    tracks.push(bare);
    // The same recording twice, a second apart.
    for (index, album, duration) in [(8, "First", 180_000), (9, "Second", 181_000)] {
        let mut input = file(index, "Echo Band", Some(album));
        input.title = "Echo".into();
        input.duration_ms = duration;
        tracks.push(input);
    }
    // A compilation that names no album artist.
    let mut various = file(10, "Some Band", Some("Hits"));
    various.is_compilation = true;
    tracks.push(various);
    for input in &tracks {
        shelf.apply(input).await;
    }
    shelf.settle().await;

    let listed = |category: AttentionCategory| {
        let shelf = &shelf;
        async move {
            let mut paths: Vec<String> = shelf
                .state
                .services
                .attention_tracks(
                    shelf.owner,
                    shelf.library,
                    category,
                    BrowsePage::new(None, None).unwrap(),
                )
                .await
                .unwrap()
                .into_iter()
                .map(|track| track.song.relative_path)
                .collect();
            paths.sort();
            paths
        }
    };
    let paths = |indices: &[usize]| -> Vec<String> {
        let mut paths: Vec<String> = indices
            .iter()
            .map(|index| format!("track-{index}.flac"))
            .collect();
        paths.sort();
        paths
    };
    assert_eq!(
        listed(AttentionCategory::TrackNumberGap).await,
        paths(&[0, 1, 2])
    );
    assert_eq!(
        listed(AttentionCategory::DuplicateTrackNumber).await,
        paths(&[3, 4])
    );
    assert_eq!(
        listed(AttentionCategory::AlbumYearConflict).await,
        paths(&[5, 6])
    );
    assert_eq!(listed(AttentionCategory::MissingYear).await, paths(&[7]));
    assert_eq!(
        listed(AttentionCategory::MissingTrackNumber).await,
        paths(&[7])
    );
    assert_eq!(listed(AttentionCategory::MissingAlbum).await, paths(&[7]));
    assert_eq!(
        listed(AttentionCategory::ProbableDuplicate).await,
        paths(&[8, 9])
    );
    assert_eq!(
        listed(AttentionCategory::CompilationNoAlbumArtist).await,
        paths(&[10])
    );
    assert!(listed(AttentionCategory::MissingArtist).await.is_empty());
    assert_eq!(
        listed(AttentionCategory::MissingCover).await.len(),
        tracks.len(),
        "no file here carries a cover"
    );

    // A correction is an effective value: the corrected track leaves its
    // category at once, with no rescan.
    let bare: String =
        sqlx::query_scalar("SELECT id FROM track WHERE library_id = ? AND relative_path = ?")
            .bind(shelf.library.to_string())
            .bind("track-7.flac")
            .fetch_one(shelf.state.db.pool())
            .await
            .unwrap();
    shelf
        .state
        .services
        .set_track_metadata(
            shelf.owner,
            Uuid::parse_str(&bare).unwrap(),
            TrackMetadataPatch {
                year: Some(Some(1999)),
                ..TrackMetadataPatch::default()
            },
            None,
        )
        .await
        .unwrap();
    assert!(listed(AttentionCategory::MissingYear).await.is_empty());

    let summary = shelf
        .state
        .services
        .attention_summary(shelf.owner, shelf.library)
        .await
        .unwrap();
    let count = |category: AttentionCategory| {
        summary
            .iter()
            .find(|entry| entry.category == category)
            .map(|entry| entry.count)
    };
    assert_eq!(summary.len(), AttentionCategory::ALL.len());
    assert_eq!(summary[0].category, AttentionCategory::SplitCredits);
    assert_eq!(count(AttentionCategory::TrackNumberGap), Some(3));
    assert_eq!(count(AttentionCategory::ProbableDuplicate), Some(2));
    assert_eq!(count(AttentionCategory::MissingYear), Some(0));

    let duplicates = shelf
        .state
        .services
        .attention_tracks(
            shelf.owner,
            shelf.library,
            AttentionCategory::ProbableDuplicate,
            BrowsePage::new(None, None).unwrap(),
        )
        .await
        .unwrap();
    assert!(
        duplicates
            .iter()
            .all(|track| track.duplicate_group == Some(1)),
        "both are one group"
    );
}
