//! A split decision at work: the artists and ids it lands on, what a scan and
//! an undo do with it, and what follows the parts through the journal.

use super::*;

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
