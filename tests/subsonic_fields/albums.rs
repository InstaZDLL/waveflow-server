//! An album's release details and disc titles.

use super::*;

#[tokio::test]
async fn an_album_reports_its_release_details_and_its_disc_titles() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("release", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let encrypted = state
        .secret_box
        .encrypt(b"dedicated-subsonic-secret")
        .unwrap();
    let api_key = "wfsk_release-key";
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("release-music");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Release library",
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
    state.db.start_scan_job(scan_id, 2, false).await.unwrap();

    for (index, disc, subtitle) in [(0usize, 1i64, "The Session"), (1, 2, "The Rehearsal")] {
        let mut input = catalog_input(index, "Nova Kern");
        input.disc_number = Some(disc);
        input.disc_subtitle = Some(subtitle.into());
        if index == 0 {
            // Only the first track carries them: the album takes the first
            // value it is given and later tracks do not overwrite it.
            input.original_release_date = Some("1998-11".into());
            input.release_date = Some("2019-04-05".into());
            input.release_types = Some("Album; Compilation".into());
            input.record_labels = Some("Nightfall Records; Second Imprint".into());
        }
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    // What a real scan does after applying its rows, before closing the job:
    // it builds the artist index and the role statistics a folder listing
    // reads. Driving the pipeline in another order would prove something about
    // an order nothing runs.
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let album_id = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .albums[0]
        .id;
    let detail = state.services.album(owner, album_id).await.unwrap();
    assert_eq!(
        detail.album.record_labels,
        vec!["Nightfall Records", "Second Imprint"]
    );
    assert_eq!(detail.album.release_types, vec!["Album", "Compilation"]);
    assert_eq!(
        detail
            .album
            .disc_titles
            .iter()
            .map(|disc| (disc.disc, disc.title.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "The Session"), (2, "The Rehearsal")]
    );

    let album = subsonic_json(&router, "getAlbum", api_key, &format!("&id={album_id}")).await;
    let album = &album["subsonic-response"]["album"];
    assert_eq!(
        album["recordLabels"],
        serde_json::json!([{"name": "Nightfall Records"}, {"name": "Second Imprint"}])
    );
    assert_eq!(
        album["releaseTypes"],
        serde_json::json!(["Album", "Compilation"])
    );
    assert_eq!(
        album["discTitles"],
        serde_json::json!([
            {"disc": 1, "title": "The Session"},
            {"disc": 2, "title": "The Rehearsal"}
        ])
    );
    // A tag naming a year and a month is a year and a month. Reporting a day
    // it never claimed would be inventing precision.
    assert_eq!(
        album["originalReleaseDate"],
        serde_json::json!({"year": 1998, "month": 11})
    );
    assert_eq!(
        album["releaseDate"],
        serde_json::json!({"year": 2019, "month": 4, "day": 5})
    );

    // An album with none of these tags declares the fields supported and
    // unset — empty arrays rather than absent keys — and names no date at all.
    let bare_scan = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(bare_scan, 1, false).await.unwrap();
    let mut bare = catalog_input(7, "Quiet Hand");
    bare.album = Some("Bare release".into());
    bare.is_compilation = false;
    state
        .db
        .apply_catalog_track(library_id, bare_scan, &bare, None, false)
        .await
        .unwrap();
    // What a real scan does after applying its rows, before closing the job:
    // it builds the artist index and the role statistics a folder listing
    // reads. Driving the pipeline in another order would prove something about
    // an order nothing runs.
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();
    state.db.finish_scan_job(bare_scan, 0).await.unwrap();
    let bare_id = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .albums
        .into_iter()
        .find(|album| album.title == "Bare release")
        .unwrap()
        .id;
    // A song is not a release. The three arrays share their element name with
    // an album inside a directory, and a recording answering `recordLabels: []`
    // would claim the server reads a label off a track.
    let song = &album["song"][0];
    assert!(omits(song, "recordLabels"));
    assert!(omits(song, "releaseTypes"));
    assert!(omits(song, "discTitles"));
    // Its own arrays are still there, empty rather than absent.
    assert_eq!(song["moods"], serde_json::json!([]));

    let bare = subsonic_json(&router, "getAlbum", api_key, &format!("&id={bare_id}")).await;
    let bare = &bare["subsonic-response"]["album"];
    assert_eq!(bare["recordLabels"], serde_json::json!([]));
    assert_eq!(bare["releaseTypes"], serde_json::json!([]));
    assert_eq!(bare["discTitles"], serde_json::json!([]));
    assert!(omits(bare, "originalReleaseDate"));
    assert!(omits(bare, "releaseDate"));

    // A third album carrying exactly one of each. This is the shape the array
    // rule is for: with one child and no rule, a record label renders as a bare
    // object instead of a list of one.
    let solo_scan = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(solo_scan, 1, false).await.unwrap();
    let mut solo_input = catalog_input(9, "Solo Hand");
    solo_input.album = Some("Single imprint".into());
    solo_input.is_compilation = false;
    solo_input.disc_number = Some(1);
    solo_input.disc_subtitle = Some("Only disc".into());
    solo_input.release_types = Some("EP".into());
    solo_input.record_labels = Some("Solo Records".into());
    state
        .db
        .apply_catalog_track(library_id, solo_scan, &solo_input, None, false)
        .await
        .unwrap();
    // What a real scan does after applying its rows, before closing the job:
    // it builds the artist index and the role statistics a folder listing
    // reads. Driving the pipeline in another order would prove something about
    // an order nothing runs.
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();
    state.db.finish_scan_job(solo_scan, 0).await.unwrap();
    let solo_id = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .albums
        .into_iter()
        .find(|album| album.title == "Single imprint")
        .unwrap()
        .id;
    let solo = subsonic_json(&router, "getAlbum", api_key, &format!("&id={solo_id}")).await;
    let solo = &solo["subsonic-response"]["album"];
    assert_eq!(
        solo["recordLabels"],
        serde_json::json!([{"name": "Solo Records"}]),
        "one record label is a list of one, not a bare object"
    );
    assert_eq!(solo["releaseTypes"], serde_json::json!(["EP"]));
    assert_eq!(
        solo["discTitles"],
        serde_json::json!([{"disc": 1, "title": "Only disc"}])
    );

    // `getMusicDirectory` renames an album to `child`, and both the array rule
    // and the injection guard are keyed on that name. Under it an album has to
    // answer exactly what it answers as `album` — for several values, for one,
    // and for none — or a client browsing folders reads a single record label
    // as a bare object and an empty list as "not supported".
    for (id, rendered) in [(album_id, album), (solo_id, solo), (bare_id, bare)] {
        let artist_id: String =
            sqlx::query_scalar("SELECT album_artist_id FROM album WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(state.db.pool())
                .await
                .unwrap();
        let directory = subsonic_json(
            &router,
            "getMusicDirectory",
            api_key,
            &format!("&id={artist_id}"),
        )
        .await;
        let children = directory["subsonic-response"]["directory"]["child"]
            .as_array()
            .unwrap()
            .clone();
        let child = children
            .iter()
            .find(|child| child["id"] == id.to_string())
            .unwrap_or_else(|| panic!("the folder of {artist_id} has to list {id}"));
        for key in ["recordLabels", "releaseTypes", "discTitles"] {
            assert_eq!(
                child[key], rendered[key],
                "{key} must read the same under `child` as under `album`"
            );
        }
        // And a song wearing that same element name carries none of the three.
        // Browsed from the album's own folder rather than the artist's: an
        // artist folder lists albums, so filtering it for songs would assert
        // nothing at all.
        let songs =
            subsonic_json(&router, "getMusicDirectory", api_key, &format!("&id={id}")).await;
        let songs = songs["subsonic-response"]["directory"]["child"]
            .as_array()
            .unwrap();
        assert!(!songs.is_empty(), "every album here has at least one track");
        for song in songs {
            assert_eq!(song["isDir"], false);
            for key in ["recordLabels", "releaseTypes", "discTitles"] {
                assert!(
                    omits(song, key),
                    "a song must not answer {key}: {}",
                    song["title"]
                );
            }
        }
    }
}
