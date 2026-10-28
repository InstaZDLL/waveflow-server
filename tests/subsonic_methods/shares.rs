//! Shares through the façade: the link every answer carries, and the albums
//! `createShare` takes (both found in the client campaign of 2026-10-28).

use super::*;

/// An album of `account`'s holding one catalogue row per `(title, disc,
/// track)`, written in the order given, and the ids of those rows in that
/// order with the album's id.
///
/// No file is written: what this serves is which tracks a share holds and in
/// which order, not their bytes. `first_index` keeps the paths and hashes of
/// two albums in one test apart.
async fn seed_album(
    state: &waveflow_server::AppState,
    config: &waveflow_server::Config,
    account: Uuid,
    name: &str,
    first_index: usize,
    rows: &[(&str, Option<i64>, Option<i64>)],
) -> (Vec<Uuid>, Uuid) {
    let root = config.data_dir.join(name);
    std::fs::create_dir_all(&root).unwrap();
    let library = state
        .db
        .create_library(
            account,
            name,
            &std::fs::canonicalize(&root).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(account), "manual")
        .await
        .unwrap();
    state
        .db
        .start_scan_job(scan, rows.len() as i64, false)
        .await
        .unwrap();
    for (offset, (title, disc, track)) in rows.iter().enumerate() {
        let input = browse_input(
            first_index + offset,
            title,
            &format!("{name} album"),
            &format!("{name} artist"),
            *track,
            *disc,
        );
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let mut ids: Vec<Uuid> = Vec::with_capacity(rows.len());
    for (title, ..) in rows {
        let id: String = sqlx::query_scalar("SELECT id FROM track WHERE library_id=? AND title=?")
            .bind(library.to_string())
            .bind(*title)
            .fetch_one(state.db.pool())
            .await
            .unwrap();
        ids.push(id.parse().unwrap());
    }
    let album: String = sqlx::query_scalar("SELECT album_id FROM track WHERE id=?")
        .bind(ids[0].to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    (ids, album.parse().unwrap())
}

/// The ids of a share's entries, in order.
fn entry_ids(share: &serde_json::Value) -> Vec<String> {
    share["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| entry["id"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// `url` is required on a `share`, and DSub 5.5.3's share screen dereferenced
/// it unconditionally: every share `getShares` listed without one crashed the
/// screen. The link was handed out at creation only, because only its hash is
/// stored — but the token is derived from the instance key and the share's id,
/// so its owner is now handed it on every answer, and it is the address that
/// opens the share. Another account still never sees the share at all.
#[tokio::test]
async fn every_share_answer_hands_its_owner_the_link() {
    let (_temp, config, state) = test_app().await;
    let (owner, api_key) = subsonic_account(&state, "share-link-owner").await;
    let (_, other_key) = subsonic_account(&state, "share-link-other").await;
    let (_, songs) = seed_titles(&state, &config, owner, "share-link", 6_100, &["Linked"]).await;
    let router = waveflow_server::app(&config, state.clone());

    let created = subsonic_json(
        &router,
        "createShare",
        &api_key,
        &format!("&id={}&description=Linked", songs[0]),
    )
    .await;
    let created = &created["subsonic-response"]["shares"]["share"][0];
    let url = created["url"]
        .as_str()
        .expect("createShare names the link")
        .to_owned();
    let share_id = created["id"].as_str().unwrap().to_owned();

    let listed = subsonic_json(&router, "getShares", &api_key, "").await;
    let listed = &listed["subsonic-response"]["shares"]["share"][0];
    assert_eq!(listed["id"], share_id.as_str());
    assert_eq!(
        listed["url"],
        url.as_str(),
        "getShares must hand the owner the link createShare gave: {listed}"
    );
    let xml = subsonic_xml(&router, "getShares", &api_key).await;
    assert!(
        xml.contains(&format!("url=\"{url}\"")),
        "the XML share carries it too: {xml}"
    );

    let updated = subsonic_json(
        &router,
        "updateShare",
        &api_key,
        &format!("&id={share_id}&description=Renamed"),
    )
    .await;
    assert_eq!(
        updated["subsonic-response"]["shares"]["share"][0]["url"],
        url.as_str(),
        "updateShare answers the share with its link: {updated}"
    );

    // The link listed is one that opens the share.
    let opened = router
        .clone()
        .oneshot(Request::get(url.as_str()).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(opened.status(), StatusCode::OK);
    assert_eq!(json_body(opened).await["id"], share_id.as_str());

    // Nobody else is handed it: their listing does not hold the share.
    let foreign = subsonic_json(&router, "getShares", &other_key, "").await;
    assert!(
        !foreign.to_string().contains(&share_id),
        "another account sees nothing of the share: {foreign}"
    );
}

/// The specification takes the id "of a song, album or video" in
/// `createShare`; a share of an album answered error 70, because shares hold
/// tracks and only tracks were looked up. An album is now expanded into its
/// available tracks in the album's order — disc, then track — beside any track
/// named in the same request, in the order the ids were given. An id that is
/// neither, or an album of a library the caller cannot read, is still 70.
#[tokio::test]
async fn create_share_expands_an_album_into_its_tracks_in_album_order() {
    let (_temp, config, state) = test_app().await;
    let (owner, api_key) = subsonic_account(&state, "share-album-owner").await;
    let (stranger, _) = subsonic_account(&state, "share-album-stranger").await;
    // Written out of order, so an expansion in insertion or id order is told
    // apart from the album's own.
    let (album_tracks, album) = seed_album(
        &state,
        &config,
        owner,
        "share-album",
        6_200,
        &[
            ("Second disc opener", Some(2), Some(1)),
            ("Unnumbered", Some(1), None),
            ("Side B", Some(1), Some(2)),
            ("Gone", Some(1), Some(3)),
            ("Side A", Some(1), Some(1)),
        ],
    )
    .await;
    let [second_disc, unnumbered, side_b, gone, side_a] = album_tracks[..] else {
        unreachable!("five rows were seeded");
    };
    sqlx::query("UPDATE track SET is_available=0 WHERE id=?")
        .bind(gone.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let (_, singles) =
        seed_titles(&state, &config, owner, "share-single", 6_300, &["Single"]).await;
    let (_, foreign_album) = seed_album(
        &state,
        &config,
        stranger,
        "share-foreign",
        6_400,
        &[("Not yours", Some(1), Some(1))],
    )
    .await;
    let router = waveflow_server::app(&config, state.clone());

    let created = subsonic_json(
        &router,
        "createShare",
        &api_key,
        &format!("&id={}&id={album}", singles[0]),
    )
    .await;
    let response = &created["subsonic-response"];
    assert_eq!(
        response["status"], "ok",
        "an album id is accepted: {created}"
    );
    let expected: Vec<String> = [singles[0], side_a, side_b, unnumbered, second_disc]
        .iter()
        .map(Uuid::to_string)
        .collect();
    assert_eq!(
        entry_ids(&response["shares"]["share"][0]),
        expected,
        "the track, then the album's available tracks in its order"
    );
    // What was stored is what was answered, and what the link plays.
    let listed = subsonic_json(&router, "getShares", &api_key, "").await;
    assert_eq!(
        entry_ids(&listed["subsonic-response"]["shares"]["share"][0]),
        expected
    );

    for (case, id) in [
        ("an id that names nothing", Uuid::new_v4().to_string()),
        (
            "an album of another account's library",
            foreign_album.to_string(),
        ),
    ] {
        let refused = subsonic_json(&router, "createShare", &api_key, &format!("&id={id}")).await;
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 70,
            "{case}: {refused}"
        );
    }
}

/// The cap a share holds is counted on the tracks, after an album is expanded:
/// one album of a share's worth of tracks is accepted, and one track more is
/// refused as too many tracks are, not cut short in silence.
#[tokio::test]
async fn an_expanded_album_is_held_to_the_share_cap() {
    use waveflow_server::services::MAX_SHARE_TRACKS;

    let (_temp, config, state) = test_app().await;
    let (owner, api_key) = subsonic_account(&state, "share-cap-owner").await;
    let titles = (0..MAX_SHARE_TRACKS)
        .map(|index| format!("Capped {index:03}"))
        .collect::<Vec<_>>();
    let rows = titles
        .iter()
        .enumerate()
        .map(|(index, title)| (title.as_str(), Some(1), Some(index as i64 + 1)))
        .collect::<Vec<_>>();
    let (_, album) = seed_album(&state, &config, owner, "share-cap", 7_000, &rows).await;
    let (_, singles) = seed_titles(
        &state,
        &config,
        owner,
        "share-cap-single",
        8_000,
        &["Extra"],
    )
    .await;
    let router = waveflow_server::app(&config, state.clone());

    let whole = subsonic_json(&router, "createShare", &api_key, &format!("&id={album}")).await;
    assert_eq!(
        entry_ids(&whole["subsonic-response"]["shares"]["share"][0]).len(),
        MAX_SHARE_TRACKS,
        "an album of exactly the cap is shared whole"
    );
    let over = subsonic_json(
        &router,
        "createShare",
        &api_key,
        &format!("&id={album}&id={}", singles[0]),
    )
    .await;
    assert_eq!(
        over["subsonic-response"]["error"]["code"], 10,
        "one track past the cap is refused: {over}"
    );
}
