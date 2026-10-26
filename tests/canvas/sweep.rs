//! The sweep: orphans collected, everything else left alone.

use super::*;

// ---------------------------------------------------------------------------
// The store sweep. RFC-009 left it open; this is what it collects, and — more
// to the point — what it refuses to touch.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_sweep_collects_orphans_and_leaves_everything_else_alone() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let library = fixture(&config, &state, "canvas-sweep", 1).await;
    let placed = state
        .services
        .place_canvas(
            library.owner,
            library.tracks[0],
            &canvas_bytes(temp.path(), "loop.mp4", "1", "black"),
            None,
        )
        .await
        .unwrap();

    let store = &config.canvas_dir;
    // Bytes no row names: what a crash between the commit and the unlink
    // leaves, and what a placement that died before its row leaves.
    let orphan = format!("{}.mp4", "a".repeat(64));
    std::fs::write(store.join(&orphan), b"orphaned bytes").unwrap();
    // A working file old enough that no request could still be holding it.
    let stale = store.join(format!("{}.part", uuid::Uuid::new_v4()));
    std::fs::write(&stale, b"abandoned").unwrap();
    set_age(&stale, std::time::Duration::from_secs(2 * 60 * 60));
    // And one that a request could: this is the placement running right now.
    let fresh = store.join(format!("{}.silent", uuid::Uuid::new_v4()));
    std::fs::write(&fresh, b"in flight").unwrap();
    // Something the sweep did not write. The store sits under the operator's
    // `data/`, and a sweep that deletes what it cannot name is one nobody
    // should run.
    std::fs::write(store.join("operator-notes.txt"), b"do not delete").unwrap();
    // And the same thing wearing this module's extension. The stem is what
    // says whose file it is: a working name has a UUID for a stem because
    // `place_canvas` put one there, and `notes.part` is not that. Backdated so
    // that only the stem stands between it and deletion.
    let disguised = store.join("operator-notes.part");
    std::fs::write(&disguised, b"do not delete either").unwrap();
    set_age(&disguised, std::time::Duration::from_secs(2 * 60 * 60));

    let swept = state.services.sweep_canvas_store().await.unwrap();
    assert_eq!(swept.blobs_removed, 1, "the unreferenced blob goes");
    assert_eq!(swept.staging_removed, 1, "the abandoned working file goes");
    assert_eq!(
        swept.unknown, 2,
        "and the strangers are counted, not removed"
    );
    assert_eq!(swept.dead_links, 0);

    let left = stored_files(&config);
    assert!(left.contains(&placed.file_name()), "the live canvas stays");
    for kept in ["operator-notes.txt", "operator-notes.part"] {
        assert!(
            left.contains(&kept.to_owned()),
            "a file the sweep does not recognise is left exactly where it was: {kept}"
        );
    }
    assert!(
        left.iter().any(|name| name.ends_with(".silent")),
        "a working file young enough to belong to a live request stays"
    );
    assert!(!left.contains(&orphan), "the orphan is gone");

    // The canvas is still servable, which is the assertion that would catch a
    // sweep that removed the wrong file.
    assert_eq!(
        state
            .services
            .canvas_for_track(library.owner, library.tracks[0])
            .await
            .unwrap(),
        Some(placed.clone())
    );

    // The other direction is reported and never repaired: deleting the row
    // would answer a dead link by making it a missing one.
    std::fs::remove_file(config.canvas_dir.join(placed.file_name())).unwrap();
    let swept = state.services.sweep_canvas_store().await.unwrap();
    assert_eq!(swept.dead_links, 1);
    assert_eq!(swept.blobs_removed, 0);
    assert!(
        state
            .services
            .canvas_for_track(library.owner, library.tracks[0])
            .await
            .unwrap()
            .is_some(),
        "the row is somebody's canvas and the sweep does not take it"
    );
}

/// Backdates a file so the sweep sees it as abandoned without the test waiting
/// an hour for it.
fn set_age(path: &std::path::Path, age: std::time::Duration) {
    let when = std::time::SystemTime::now() - age;
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(when).unwrap();
}
