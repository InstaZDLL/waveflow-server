//! The sweep of the artwork store.

use super::*;

/// Sets a file's modification time back far enough to clear `WRITE_GRACE`.
///
/// The sweep leaves a young file alone whatever the database says, which is
/// what stands in for the lock it cannot take over `waveflow_core`'s writer. A
/// test that wants to see a removal has to age the file rather than wait an
/// hour for it.
fn age_beyond_the_grace(path: &std::path::Path) {
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
}

/// Fails if anything has touched a file since [`age_beyond_the_grace`] set it
/// back.
///
/// `extract_cover` spawns a detached thread that writes `<hash>_1x.jpg` and
/// `<hash>_2x.jpg` into the same directory, and it writes them under the names
/// this test uses. It produces nothing for these fixtures — the store holds the
/// cover alone, checked at two seconds — so the files below are this test's
/// alone. Depending on that silently is the problem: if the thread ever did
/// write, it would restore a fresh timestamp and the sweep would spare a file
/// the test expects it to take, intermittently and for a reason nothing states.
/// So the ages are read back where they matter, and interference fails loudly
/// on the pass it happens rather than flaking on some later one.
fn assert_still_aged(path: &std::path::Path) {
    let age = std::fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .elapsed()
        .unwrap();
    assert!(
        age > std::time::Duration::from_secs(60 * 60),
        "{} was touched after it was aged: something else writes this name",
        path.display()
    );
}

/// The one name in the store the sweep must never take: a cover in use.
///
/// `artwork_dir` had no sweep at all, so the first thing worth proving is not
/// that the new one collects — it is that it collects nothing. A pass over a
/// freshly scanned library must leave every byte where it is, and must not
/// report the store as broken while doing it.
#[tokio::test]
async fn the_artwork_sweep_takes_nothing_a_library_still_names() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("sweep-live", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("sweep-live");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("One.wav"));
    write_test_png(&music.join("cover.png"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Live", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Live".into(),
            root_path: root,
        },
    )
    .await;

    // The name comes from the scan, never from this test: `extract_cover`
    // chooses it, and a fixture that spelled it independently would agree with
    // itself while the server did something else.
    let cover = std::fs::read_dir(&config.artwork_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "png"))
        .expect("the scan wrote a cover");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(stored, 1);

    // Aged deliberately: the grace period must not be what saves it here, or
    // this test would pass on a sweep that takes live covers a day later.
    age_beyond_the_grace(&cover);
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.rows_removed, 0);
    assert_eq!(swept.covers_removed, 0);
    assert_eq!(swept.dead_links, 0, "the row and the file agree");
    assert!(cover.exists(), "a cover in use is not the sweep's to take");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artwork")
            .fetch_one(state.db.pool())
            .await
            .unwrap(),
        1
    );

    // A name the sweep cannot read is left where it is and said out loud. The
    // store sits under the operator's `data/`, and the alternative — deleting
    // the unrecognised — is how a sweep eats something it was never told about.
    let stranger = config.artwork_dir.join("operator-notes.txt");
    std::fs::write(&stranger, b"not mine to remove").unwrap();
    age_beyond_the_grace(&stranger);
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.unknown, 1);
    assert_eq!(swept.covers_removed, 0);
    assert!(stranger.exists());
}

/// A cover nothing names any more goes, and takes its thumbnails with it.
///
/// This is the property `artwork_dir` never had. Three columns are
/// `ON DELETE SET NULL`, so a library that is deleted or rescanned returns them
/// to `NULL` and tells nobody — the row stays, the bytes stay, and both stay
/// for the life of the instance.
#[tokio::test]
async fn an_unreferenced_cover_and_its_thumbnails_go_once_they_are_old_enough() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("sweep-dead", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("sweep-dead");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("One.wav"));
    write_test_png(&music.join("cover.png"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Dead", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Dead".into(),
            root_path: root,
        },
    )
    .await;

    let stored_hash: String = sqlx::query_scalar("SELECT hash FROM artwork")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    let cover = std::fs::read_dir(&config.artwork_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "png"))
        .expect("the scan wrote a cover");

    // The thumbnail names come from `waveflow_core`, which is what writes them,
    // rather than from a convention spelled out here — a fixture that spelled
    // `_1x` itself would agree with itself while the library did something else.
    //
    // The bytes are written here rather than waited for, because the real job is
    // a detached thread with no handle to join: `spawn_thumbnail_job` returns
    // `()`. `assert_still_aged` is what keeps that from being a silent bet.
    let thumbnails: Vec<std::path::PathBuf> = [
        waveflow_core::artwork::thumbnails::THUMB_SMALL,
        waveflow_core::artwork::thumbnails::THUMB_MEDIUM,
    ]
    .into_iter()
    .map(|size| {
        let path = waveflow_core::artwork::thumbnails::thumbnail_path(
            &config.artwork_dir,
            &stored_hash,
            size,
        );
        std::fs::write(&path, b"thumbnail bytes").unwrap();
        path
    })
    .collect();

    // What deleting an album does, without deleting an album: the columns go
    // back to NULL and nothing anywhere is told.
    // Spelled out three times rather than looped over a table name: sqlx takes
    // static SQL only, which is the same rule that keeps the projection macros
    // injection-proof by construction.
    for statement in [
        "UPDATE track SET artwork_hash = NULL",
        "UPDATE album SET artwork_hash = NULL",
        "UPDATE artist SET artwork_hash = NULL",
    ] {
        sqlx::query(statement)
            .execute(state.db.pool())
            .await
            .unwrap();
    }

    // The row goes at once — it is a database fact under the writer gate, with
    // no window to wait out. The files do not, because they are young, and the
    // sweep cannot tell a cover abandoned a second ago from one a scan is at
    // this moment about to name.
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.rows_removed, 1);
    assert_eq!(swept.covers_removed, 0, "the grace period holds the bytes");
    assert_eq!(swept.thumbnails_removed, 0);
    assert!(cover.exists());

    age_beyond_the_grace(&cover);
    for thumbnail in &thumbnails {
        age_beyond_the_grace(thumbnail);
    }
    assert_still_aged(&cover);
    for thumbnail in &thumbnails {
        assert_still_aged(thumbnail);
    }
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.rows_removed, 0, "the row went on the pass before");
    assert_eq!(swept.covers_removed, 1);
    assert_eq!(
        swept.thumbnails_removed, 2,
        "a thumbnail is named by nothing, so only its stem can save it"
    );
    assert!(!cover.exists());
    for thumbnail in &thumbnails {
        assert!(!thumbnail.exists());
    }

    // The other direction, which is reported and never repaired: a row naming a
    // file the store does not hold. Deleting the row would answer a cover that
    // fails to load by removing the album's art outright.
    sqlx::query(
        "INSERT INTO artwork (hash, format, source, byte_size, created_at) \
         VALUES (?, 'png', 'embedded', 1, ?)",
    )
    .bind("b".repeat(64))
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    sqlx::query("UPDATE track SET artwork_hash = ?")
        .bind("b".repeat(64))
        .execute(state.db.pool())
        .await
        .unwrap();
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.dead_links, 1);
    assert_eq!(swept.rows_removed, 0, "a named row is not an orphan");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artwork")
            .fetch_one(state.db.pool())
            .await
            .unwrap(),
        1,
        "the dead link is reported, not repaired"
    );
}
