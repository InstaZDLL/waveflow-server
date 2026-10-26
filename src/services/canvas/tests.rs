use super::*;

/// The lock in [`DomainServices::discard_orphan_blob`], demonstrated.
///
/// Its own comment said the suite covered the re-read and not the lock:
/// removing the count fails a test, removing the lock fails nothing,
/// because the damage needs a placement and a sweep to interleave at one
/// instant and no *integration* test can force that ordering.
///
/// A unit test can, because it sits in the module and may hold the lock
/// itself — which is exactly what a placement holds between writing its
/// bytes and inserting its row. So the interleaving is not raced for, it is
/// arranged: the sweep starts, finds the blob, and blocks. The row is
/// committed while it waits. The lock is released, the sweep re-reads, and
/// the file it was about to take is now somebody's canvas.
///
/// Removing the lock fails this: the sweep reads no row at once and unlinks
/// before the placement ever commits, leaving a live row naming nothing.
#[tokio::test]
async fn a_placement_that_commits_while_the_sweep_waits_keeps_its_bytes() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = crate::Config::for_data_dir(temp.path().join("data"));
    let state = crate::initialize(&config).await.expect("initialize");
    let services = state.services.clone();

    // Bytes in the store and no row: what a placement looks like from
    // outside, one instant before it commits.
    tokio::fs::create_dir_all(&config.canvas_dir)
        .await
        .expect("canvas store");
    let hash = "c".repeat(64);
    let path = config.canvas_dir.join(format!("{hash}.mp4"));
    tokio::fs::write(&path, b"loop bytes").await.expect("write");

    let lock = services.canvas_lock(&hash);
    let held = lock.lock().await;

    let sweeper = {
        let services = services.clone();
        tokio::spawn(async move { services.sweep_canvas_store().await })
    };

    // Every chance to run and find no row. Without the lock this is where
    // the bytes go.
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        path.exists(),
        "the sweep took a blob whose lock was held: it never waited"
    );

    // Under the writer gate, because a placement writes this row under it:
    // `link_canvas` takes it and then opens the transaction holding this
    // exact statement. A test standing in for a placement that skipped the
    // barrier would be standing in for something the server never does —
    // and this one is about concurrency, which is where that stops being
    // cosmetic.
    //
    // Nested inside the blob's lock and released first, which is the order
    // the real path holds them in: `place_canvas` takes the lock, and the
    // gate is taken and dropped within it. The sweep waiting on the lock
    // takes no gate of its own — `discard_orphan_blob` only reads — so
    // there is nothing here to deadlock against.
    {
        let _writer = services.db.writer_guard().await;
        sqlx::query(
            "INSERT INTO canvas (hash, format, byte_size, created_at) VALUES (?, 'mp4', 10, ?)",
        )
        .bind(&hash)
        .bind(now_ms())
        .execute(services.db.pool())
        .await
        .expect("the placement commits");
    }
    drop(held);

    let swept = sweeper
        .await
        .expect("the sweep finishes")
        .expect("no error");
    assert_eq!(
        swept.blobs_removed, 0,
        "the re-read under the lock is what sees the new row"
    );
    assert!(
        path.exists(),
        "a live row must not be left naming bytes the sweep carried off"
    );
    assert_eq!(swept.dead_links, 0);
}
