//! The order of the tracks listed in a filed folder, or among the unfiled.

use super::*;

/// A folder lists its tracks album by album, as their sleeves do: by the
/// album's artist, then the album, then disc and track.
///
/// By each track's own artist, an album whose tracks name different artists
/// — a compilation, a guest on one song — came apart across the listing, its
/// tracks scattered among other albums' by the name on each. Here the
/// compilation's two tracks credit `Zed` and `Abe`, and its album artist
/// sorts after `Mid`: by track artist, `Abe` came first and `Zed` last, with
/// `Mid`'s album between them. And the album is sorted by its own title, not
/// each file's spelling of it: a file tagging it in lower case would otherwise
/// sort apart from its album's other tracks.
#[tokio::test]
async fn a_folder_lists_its_tracks_album_by_album() {
    let (_temp, config, state) = test_app().await;
    // Read through the service, signed in as nobody: no password is needed.
    let owner = state
        .db
        .create_account("order-owner", "unused", AccountRole::User, now_ms())
        .await
        .unwrap();
    let root = std::fs::canonicalize(&config.data_dir).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Order", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    // (title, album, album artist, track artist, disc, track)
    let tracks = [
        ("Second", "Compilation", "Various Artists", "Abe", 1, 2),
        ("Side B", "B Side", "Mid", "Mid", 1, 2),
        ("Disc Two", "Compilation", "Various Artists", "Abe", 2, 1),
        ("First", "Compilation", "Various Artists", "Zed", 1, 1),
        // The same album, its title spelt in another case on one file: one
        // album, whose tracks keep their sleeve order.
        ("Side A", "b side", "Mid", "Mid", 1, 1),
    ];
    state
        .db
        .start_scan_job(scan_id, tracks.len() as i64, false)
        .await
        .unwrap();
    for (index, (title, album, album_artist, artist, disc, track)) in tracks.iter().enumerate() {
        let mut input = browse_input(
            41_000 + index,
            title,
            album,
            artist,
            Some(*track),
            Some(*disc),
        );
        input.album_artist = Some((*album_artist).to_owned());
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let page = waveflow_server::services::BrowsePage::new(None, None).unwrap();
    let listed = state
        .services
        .folder_tracks(owner, library_id, None, page)
        .await
        .unwrap();
    let titles: Vec<&str> = listed.iter().map(|song| song.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Side A", "Side B", "First", "Second", "Disc Two"],
        "Mid's album, then the compilation whole, disc by disc"
    );
}
