//! The frozen v2.0-beta contract, and what a foreign catalogue may not see.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::authentication::now_ms;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

#[tokio::test]
async fn subsonic_xml_json_auth_catalog_and_user_data_are_compatible() {
    use md5::{Digest, Md5};

    let (_temp, config, state) = test_app().await;
    let web_password = "correct horse battery staple";
    let subsonic_password = "subsonic-secret-123";
    let api_key = "wfsk_golden-api-key";
    let web_hash = security::hash_password(web_password).unwrap();
    let admin = state
        .db
        .create_account("sub-admin", &web_hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let encrypted = state
        .secret_box
        .encrypt(subsonic_password.as_bytes())
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("subsonic-music");
    std::fs::create_dir_all(&music).unwrap();
    generate_audio_fixture(&music.join("Golden.wav"), "pcm_s16le", "wav");
    std::fs::write(
        music.join("Golden.lrc"),
        "[00:01.25]Golden opening\n[00:02.500]Golden chorus",
    )
    .unwrap();
    write_test_wav(&music.join("NoArtist.wav"));
    write_test_png(&music.join("cover.png"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            admin,
            "Subsonic library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        admin,
        LibraryRecord {
            id: library,
            name: "Subsonic library".into(),
            root_path: root,
        },
    )
    .await;
    let secondary_music = config.data_dir.join("subsonic-secondary");
    std::fs::create_dir_all(&secondary_music).unwrap();
    let secondary_root = std::fs::canonicalize(&secondary_music).unwrap();
    let secondary_library = state
        .db
        .create_library(
            admin,
            "Secondary Subsonic library",
            &secondary_root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let foreign_owner = state
        .db
        .create_account("sub-foreign-owner", &web_hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let foreign_music = config.data_dir.join("subsonic-foreign");
    std::fs::create_dir_all(&foreign_music).unwrap();
    let foreign_library = state
        .db
        .create_library(
            foreign_owner,
            "Foreign Subsonic library",
            &std::fs::canonicalize(&foreign_music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let foreign_scan = state
        .db
        .create_scan_job(foreign_library, Some(foreign_owner), "manual")
        .await
        .unwrap();
    state
        .db
        .start_scan_job(foreign_scan, 1, false)
        .await
        .unwrap();
    let mut foreign_input = browse_input(
        7_000,
        "Foreign track",
        "Foreign album",
        "Foreign artist",
        Some(1),
        Some(1),
    );
    foreign_input.lyrics = vec![waveflow_server::lyrics::LyricsInput {
        source: "embedded",
        lang: "eng".into(),
        synced: false,
        content: "private words".into(),
    }];
    foreign_input.lyrics_hash = blake3::hash(b"private words").to_hex().to_string();
    state
        .db
        .apply_catalog_track(foreign_library, foreign_scan, &foreign_input, None, false)
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(foreign_library)
        .await
        .unwrap();
    state.db.finish_scan_job(foreign_scan, 0).await.unwrap();
    let foreign_artist = state
        .services
        .catalog_snapshot(foreign_owner, &[])
        .await
        .unwrap()
        .artists
        .first()
        .unwrap()
        .artist
        .id;
    let foreign_song = state
        .services
        .catalog_snapshot(foreign_owner, &[])
        .await
        .unwrap()
        .songs
        .first()
        .unwrap()
        .id;
    let foreign_album = state
        .services
        .catalog_snapshot(foreign_owner, &[])
        .await
        .unwrap()
        .albums
        .first()
        .unwrap()
        .id;
    let snapshot = state.services.catalog_snapshot(admin, &[]).await.unwrap();
    let song = snapshot
        .songs
        .iter()
        .find(|song| song.title == "Matrix wav")
        .expect("the tagged Golden fixture is present")
        .id;
    let no_artist_song = snapshot
        .songs
        .iter()
        .find(|song| song.artist_id.is_none())
        .expect("the untagged fixture has no track_artist row")
        .id;
    let artist = snapshot.artists.first().unwrap().artist.id;
    let album = snapshot.albums.first().unwrap().id;
    let artwork = snapshot
        .songs
        .first()
        .unwrap()
        .artwork_hash
        .clone()
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let plain_auth = format!("u=sub-admin&p={subsonic_password}&v=1.16.1&c=golden");

    let ping = router
        .clone()
        .oneshot(
            Request::get(format!("/rest/ping.view?{plain_auth}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ping.status(), StatusCode::OK);
    let ping_xml = body_text(ping).await;
    assert!(ping_xml.starts_with("<subsonic-response"));
    assert!(ping_xml.contains("status=\"ok\""));
    assert!(!ping_xml.contains("<ping"));

    let symfonium_probe = router
        .clone()
        .oneshot(
            Request::get("/rest/ping.view?u=test&p=test&v=1.13.0&c=Symfonium&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(symfonium_probe.status(), StatusCode::OK);
    assert_eq!(
        json_body(symfonium_probe).await["subsonic-response"]["status"],
        "ok"
    );

    for path in [
        "/rest/getMusicFolders.view?u=test&p=test&v=1.13.0&c=Symfonium&f=json",
        "/rest/ping.view?u=test&p=test&v=1.13.0&c=another-client&f=json",
        "/rest/ping.view?u=test&p=test&apiKey=invalid&v=1.13.0&c=Symfonium&f=json",
        "/rest/ping.view?u=test&u=another&p=test&v=1.13.0&c=Symfonium&f=json",
    ] {
        let rejected = router
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        // A refused credential is an HTTP 200 carrying error code 40: the
        // Subsonic contract puts the outcome in the body, and a client that
        // trusted the status line would never read it.
        assert_eq!(rejected.status(), StatusCode::OK, "{path}");
        let body = json_body(rejected).await;
        assert_eq!(body["subsonic-response"]["error"]["code"], 40, "{path}");
        // A failed response still identifies the server. WaveFlow Desktop
        // decides whether to enable the native /api/v2 surface from `type`
        // alone, before it holds any credential, so this must survive a
        // rewrite of the error envelope. getOpenSubsonicExtensions cannot
        // serve that purpose: it needs an authenticated call, and this decision
        // is made before there is a credential.
        assert_eq!(body["subsonic-response"]["type"], "waveflow", "{path}");
        assert_eq!(body["subsonic-response"]["openSubsonic"], true, "{path}");
        assert!(
            body["subsonic-response"]["serverVersion"].is_string(),
            "{path}"
        );
    }

    // Under `formPost` the requested format arrives in the body, not the query
    // string. A refused credential must still be answered in it: a client that
    // asked for JSON and received XML cannot read the error at all.
    let rejected_post = router
        .clone()
        .oneshot(
            Request::post("/rest/getMusicFolders.view")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("apiKey=wfsk_wrong&v=1.16.1&c=golden&f=json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected_post.status(), StatusCode::OK);
    assert_eq!(
        rejected_post.headers()["content-type"],
        "application/json; charset=utf-8"
    );
    assert_eq!(
        json_body(rejected_post).await["subsonic-response"]["error"]["code"],
        40
    );

    let salt = "golden-salt";
    let mut digest = Md5::new();
    digest.update(subsonic_password.as_bytes());
    digest.update(salt.as_bytes());
    let token_auth = format!(
        "u=sub-admin&t={}&s={salt}&v=1.16.1&c=golden&f=json",
        hex::encode(digest.finalize())
    );
    let token_ping = router
        .clone()
        .oneshot(
            Request::get(format!("/rest/ping?{token_auth}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(token_ping.status(), StatusCode::OK);
    assert_eq!(
        json_body(token_ping).await["subsonic-response"]["status"],
        "ok"
    );

    let post_body = format!("apiKey={api_key}&v=1.16.1&c=golden&f=json");
    let folders = router
        .clone()
        .oneshot(
            Request::post("/rest/getMusicFolders.view")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(post_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(folders.status(), StatusCode::OK);
    let folders = json_body(folders).await;
    let folder_ids = folders["subsonic-response"]["musicFolders"]["musicFolder"]
        .as_array()
        .unwrap()
        .iter()
        .map(|folder| folder["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(folder_ids.contains(&library.to_string()));
    assert!(folder_ids.contains(&secondary_library.to_string()));

    // The same request with a conformant spelling of the media type: case is not
    // significant and a charset parameter is allowed, so neither may turn a
    // valid form POST into a protocol error.
    let cased = router
        .clone()
        .oneshot(
            Request::post("/rest/getMusicFolders.view")
                .header(
                    "content-type",
                    "Application/X-WWW-Form-Urlencoded; charset=UTF-8",
                )
                .body(Body::from(format!(
                    "apiKey={api_key}&v=1.16.1&c=golden&f=json"
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cased.status(), StatusCode::OK);
    assert_eq!(json_body(cased).await["subsonic-response"]["status"], "ok");

    // A type that merely starts with the expected one is a different type. The
    // format comes from the query string here because a rejected body is never
    // read, so it cannot carry `f`.
    let lookalike = router
        .clone()
        .oneshot(
            Request::post("/rest/getMusicFolders.view?f=json")
                .header("content-type", "application/x-www-form-urlencodedish")
                .body(Body::from(format!(
                    "apiKey={api_key}&v=1.16.1&c=golden&f=json"
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lookalike.status(), StatusCode::OK);
    assert_eq!(
        json_body(lookalike).await["subsonic-response"]["error"]["code"],
        10
    );

    let cases = [
        ("getLicense", String::new()),
        ("getOpenSubsonicExtensions", String::new()),
        ("tokenInfo", String::new()),
        ("getBookmarks", String::new()),
        ("getIndexes", format!("&musicFolderId={library}")),
        ("getArtists", format!("&musicFolderId={library}")),
        ("getArtist", format!("&id={artist}")),
        ("getArtistInfo", format!("&id={artist}")),
        ("getArtistInfo2", format!("&id={artist}")),
        ("getAlbumInfo", format!("&id={album}")),
        ("getAlbumInfo2", format!("&id={album}")),
        ("getAlbum", format!("&id={album}")),
        ("getSong", format!("&id={song}")),
        ("getLyrics", "&title=Matrix%20wav".into()),
        ("getLyricsBySongId", format!("&id={song}")),
        ("getGenres", String::new()),
        ("getMusicDirectory", format!("&id={album}")),
        ("getAlbumList", "&type=newest&size=10".into()),
        ("getAlbumList2", "&type=alphabeticalByName&size=10".into()),
        ("getRandomSongs", "&size=10".into()),
        ("getSongsByGenre", "&genre=Electronic&count=10".into()),
        ("search3", "&query=Matrix&songCount=10".into()),
    ];
    for (method, extra) in cases {
        let response = subsonic_json(&router, method, api_key, &extra).await;
        assert_eq!(response["subsonic-response"]["status"], "ok", "{method}");
        if method == "getAlbumList" {
            assert!(response["subsonic-response"]["albumList"].is_object());
            assert!(response["subsonic-response"]["albumList"]["album"].is_array());
            assert!(response["subsonic-response"].get("albumList2").is_none());
        }
        if method == "getOpenSubsonicExtensions" {
            let extensions = &response["subsonic-response"]["openSubsonicExtensions"];
            assert!(
                extensions.is_array(),
                "the extension list is an array whether empty or populated, never an object"
            );
            let advertised = extensions
                .as_array()
                .unwrap()
                .iter()
                .map(|extension| {
                    assert!(
                        extension["versions"].is_array(),
                        "versions must be an array of integers"
                    );
                    extension["name"].as_str().unwrap().to_owned()
                })
                .collect::<Vec<_>>();
            // Advertising an extension the server does not honour is worse than
            // advertising none: the client stops probing and starts relying on
            // it. Each of these is exercised elsewhere in this suite — form
            // POST, apiKey authentication and timeOffset on a transcode.
            assert!(advertised.contains(&"formPost".to_owned()));
            assert!(advertised.contains(&"apiKeyAuthentication".to_owned()));
            assert!(advertised.contains(&"transcodeOffset".to_owned()));
            assert!(advertised.contains(&"songLyrics".to_owned()));
            let song_lyrics = extensions
                .as_array()
                .unwrap()
                .iter()
                .find(|extension| extension["name"] == "songLyrics")
                .unwrap();
            assert_eq!(song_lyrics["versions"], serde_json::json!([1]));
        }
        if method == "getBookmarks" {
            assert!(response["subsonic-response"]["bookmarks"].is_object());
        }
        if method == "getSong" {
            assert_eq!(
                response["subsonic-response"]["song"]["artistId"],
                artist.to_string()
            );
        }
        if method == "getLyrics" {
            let lyrics = &response["subsonic-response"]["lyrics"];
            assert_eq!(lyrics["title"], "Matrix wav");
            assert_eq!(lyrics["value"], "Golden opening\nGolden chorus");
        }
        if method == "getLyricsBySongId" {
            let lyrics = &response["subsonic-response"]["lyricsList"]["structuredLyrics"];
            assert!(lyrics.is_array());
            assert_eq!(lyrics[0]["displayTitle"], "Matrix wav");
            assert_eq!(lyrics[0]["lang"], "xxx");
            assert_eq!(lyrics[0]["synced"], true);
            assert_eq!(lyrics[0]["line"][0]["start"], 1_250);
            assert_eq!(lyrics[0]["line"][0]["value"], "Golden opening");
        }
        if method == "getArtistInfo" || method == "getArtistInfo2" {
            let container = if method == "getArtistInfo" {
                "artistInfo"
            } else {
                "artistInfo2"
            };
            assert_eq!(
                response["subsonic-response"][container],
                serde_json::json!({})
            );
        }
        // Feishin and Symfonium open an album with this call. WaveFlow enriches
        // nothing yet, so the honest answer is the standard empty container —
        // not the code 0 that made the client treat the album as broken.
        if method == "getAlbumInfo" || method == "getAlbumInfo2" {
            let container = if method == "getAlbumInfo" {
                "albumInfo"
            } else {
                "albumInfo2"
            };
            assert_eq!(
                response["subsonic-response"][container],
                serde_json::json!({})
            );
        }
        // The second half of apiKeyAuthentication: a key holder can ask which
        // account it speaks for. The extension is advertised, so this must
        // answer.
        if method == "tokenInfo" {
            assert_eq!(
                response["subsonic-response"]["tokenInfo"]["username"],
                "sub-admin"
            );
        }
    }

    let artist_info = router
        .clone()
        .oneshot(
            Request::get(format!("/rest/getArtistInfo.view?{plain_auth}&id={artist}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(artist_info.status(), StatusCode::OK);
    assert!(body_text(artist_info).await.contains("<artistInfo/>"));

    let song_xml = router
        .clone()
        .oneshot(
            Request::get(format!("/rest/getSong.view?{plain_auth}&id={song}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(song_xml.status(), StatusCode::OK);
    assert!(body_text(song_xml)
        .await
        .contains(&format!("artistId=\"{artist}\"")));

    let lyrics_xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getLyricsBySongId.view?{plain_auth}&id={song}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lyrics_xml.status(), StatusCode::OK);
    let lyrics_xml = body_text(lyrics_xml).await;
    assert!(lyrics_xml.contains("<lyricsList>"));
    assert!(lyrics_xml.contains("<structuredLyrics"));
    assert!(lyrics_xml.contains("<line start=\"1250\">Golden opening</line>"));

    let native_token = login_token(&router, "sub-admin", web_password).await;
    let native_lyrics = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/tracks/{song}/lyrics"))
                .header("authorization", format!("Bearer {native_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native_lyrics.status(), StatusCode::OK);
    let native_lyrics = json_body(native_lyrics).await;
    assert_eq!(native_lyrics["trackId"], song.to_string());
    assert_eq!(
        native_lyrics["structuredLyrics"][0]["line"][1]["start"],
        2_500
    );

    let hidden_lyrics = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getLyricsBySongId.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={foreign_song}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden_lyrics.status(), StatusCode::OK);
    let hidden_lyrics = json_body(hidden_lyrics).await;
    assert_eq!(hidden_lyrics["subsonic-response"]["status"], "failed");
    assert_eq!(hidden_lyrics["subsonic-response"]["error"]["code"], 70);
    let hidden_native_lyrics = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/tracks/{foreign_song}/lyrics"))
                .header("authorization", format!("Bearer {native_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden_native_lyrics.status(), StatusCode::NOT_FOUND);

    let empty_lyrics = subsonic_json(
        &router,
        "getLyricsBySongId",
        api_key,
        &format!("&id={no_artist_song}"),
    )
    .await;
    assert_eq!(
        empty_lyrics["subsonic-response"]["lyricsList"]["structuredLyrics"],
        serde_json::json!([])
    );

    let no_artist_json = subsonic_json(
        &router,
        "getSong",
        api_key,
        &format!("&id={no_artist_song}"),
    )
    .await;
    assert!(no_artist_json["subsonic-response"]["song"]
        .get("artistId")
        .is_none());
    let no_artist_xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getSong.view?{plain_auth}&id={no_artist_song}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_artist_xml.status(), StatusCode::OK);
    assert!(!body_text(no_artist_xml).await.contains("artistId="));

    let dsub_artist_info = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getArtistInfo2.view?{plain_auth}&id={artist}&includeNotPresent=true"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dsub_artist_info.status(), StatusCode::OK);
    let dsub_artist_info = body_text(dsub_artist_info).await;
    assert!(dsub_artist_info.contains("<artistInfo2/>"));

    // XML is the default, and an empty container renders as a self-closing tag
    // there rather than as the `{}` the JSON branch produces. Album pages open on
    // this call, so both encodings are asserted.
    for (method, container) in [
        ("getAlbumInfo", "<albumInfo/>"),
        ("getAlbumInfo2", "<albumInfo2/>"),
    ] {
        let empty_album_info = router
            .clone()
            .oneshot(
                Request::get(format!("/rest/{method}.view?{plain_auth}&id={album}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(empty_album_info.status(), StatusCode::OK, "{method}");
        let empty_album_info = body_text(empty_album_info).await;
        assert!(
            empty_album_info.starts_with("<subsonic-response"),
            "{method}"
        );
        assert!(empty_album_info.contains(container), "{method}");
    }

    for (method, id) in [
        ("getArtistInfo", foreign_artist),
        ("getArtistInfo2", foreign_artist),
        ("getArtistInfo2", Uuid::nil()),
        ("getAlbumInfo", foreign_album),
        ("getAlbumInfo", Uuid::nil()),
        ("getAlbumInfo2", foreign_album),
        ("getAlbumInfo2", Uuid::nil()),
    ] {
        let hidden_artist_info = router
            .clone()
            .oneshot(
                Request::get(format!(
                    "/rest/{method}.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={id}"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(hidden_artist_info.status(), StatusCode::OK);
        let hidden_artist_info = json_body(hidden_artist_info).await;
        assert_eq!(hidden_artist_info["subsonic-response"]["status"], "failed");
        assert_eq!(hidden_artist_info["subsonic-response"]["error"]["code"], 70);
    }
    let match_all_search = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=%22%22&artistCount=500&albumCount=500&songCount=500",
    )
    .await;
    let match_all = &match_all_search["subsonic-response"]["searchResult3"];
    assert!(!match_all["artist"].as_array().unwrap().is_empty());
    assert!(!match_all["album"].as_array().unwrap().is_empty());
    assert!(!match_all["song"].as_array().unwrap().is_empty());

    let exhausted_search = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=Matrix&artistCount=0&albumCount=0&songCount=1&songOffset=1",
    )
    .await;
    assert!(exhausted_search["subsonic-response"]["searchResult3"]
        .get("song")
        .is_none());
    let repeated_folder_search = subsonic_json(
        &router,
        "search3",
        api_key,
        &format!(
            "&query=Matrix&artistCount=0&albumCount=0&songCount=10&musicFolderId={secondary_library}&musicFolderId={library}"
        ),
    )
    .await;
    assert_eq!(
        repeated_folder_search["subsonic-response"]["searchResult3"]["song"][0]["id"],
        song.to_string()
    );
    let secondary_only_search = subsonic_json(
        &router,
        "search3",
        api_key,
        &format!(
            "&query=Matrix&artistCount=0&albumCount=0&songCount=10&musicFolderId={secondary_library}"
        ),
    )
    .await;
    assert!(secondary_only_search["subsonic-response"]["searchResult3"]
        .get("song")
        .is_none());

    let created = subsonic_json(
        &router,
        "createPlaylist",
        api_key,
        &format!("&name=Golden&songId={song}"),
    )
    .await;
    let playlist = created["subsonic-response"]["playlist"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Feishin decides whether a playlist is editable from `owner`. Playlist
    // reads are already scoped to their owner, so the empty string this used to
    // emit made every playlist look like someone else's.
    assert_eq!(
        created["subsonic-response"]["playlist"]["owner"],
        "sub-admin"
    );
    // Given a playlistId, songId names every song of the playlist. A client
    // that removes a song sends back what remains, so treating those ids as
    // additions left the removed song in place and the edit looked lost.
    let replaced = subsonic_json(
        &router,
        "createPlaylist",
        api_key,
        &format!("&playlistId={playlist}&songId={no_artist_song}"),
    )
    .await;
    assert_eq!(replaced["subsonic-response"]["playlist"]["songCount"], 1);
    let reread = subsonic_json(&router, "getPlaylist", api_key, &format!("&id={playlist}")).await;
    let entries = reread["subsonic-response"]["playlist"]["entry"]
        .as_array()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], no_artist_song.to_string());
    // Put the original song back, so the checks below see the playlist they
    // were written against.
    subsonic_json(
        &router,
        "createPlaylist",
        api_key,
        &format!("&playlistId={playlist}&songId={song}"),
    )
    .await;
    for (method, extra) in [
        ("getPlaylists", String::new()),
        ("getPlaylist", format!("&id={playlist}")),
        (
            "updatePlaylist",
            format!("&playlistId={playlist}&comment=Updated"),
        ),
        (
            "star",
            format!("&id={song}&albumId={album}&artistId={artist}"),
        ),
        ("getStarred2", String::new()),
        ("setRating", format!("&id={song}&rating=5")),
        ("scrobble", format!("&id={song}&submission=false")),
        ("getNowPlaying", String::new()),
        (
            "savePlayQueue",
            format!("&id={song}&current={song}&position=25"),
        ),
        ("getPlayQueue", String::new()),
    ] {
        let response = subsonic_json(&router, method, api_key, &extra).await;
        assert_eq!(response["subsonic-response"]["status"], "ok", "{method}");
        if method == "getPlaylists" {
            assert_eq!(
                response["subsonic-response"]["playlists"]["playlist"][0]["owner"],
                "sub-admin"
            );
        }
        if method == "getPlaylist" {
            assert_eq!(
                response["subsonic-response"]["playlist"]["owner"],
                "sub-admin"
            );
        }
    }

    let decorated_song = subsonic_json(&router, "getSong", api_key, &format!("&id={song}")).await;
    assert_eq!(decorated_song["subsonic-response"]["song"]["userRating"], 5);
    assert!(decorated_song["subsonic-response"]["song"]["starred"]
        .as_str()
        .is_some());
    let decorated_search = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=Matrix&artistCount=0&albumCount=0&songCount=10",
    )
    .await;
    assert_eq!(
        decorated_search["subsonic-response"]["searchResult3"]["song"][0]["userRating"],
        5
    );
    assert!(
        decorated_search["subsonic-response"]["searchResult3"]["song"][0]["starred"]
            .as_str()
            .is_some()
    );

    // DSub 5.5.3 sends albums and artists through the generic `id`
    // parameter instead of the newer albumId/artistId parameters.
    for id in [album, artist] {
        assert_eq!(
            subsonic_json(&router, "unstar", api_key, &format!("&id={id}")).await
                ["subsonic-response"]["status"],
            "ok"
        );
        assert_eq!(
            subsonic_json(&router, "star", api_key, &format!("&id={id}")).await
                ["subsonic-response"]["status"],
            "ok"
        );
    }
    assert_eq!(
        subsonic_json(
            &router,
            "setRating",
            api_key,
            &format!("&id={album}&rating=4")
        )
        .await["subsonic-response"]["status"],
        "ok"
    );
    assert_eq!(
        subsonic_json(
            &router,
            "scrobble",
            api_key,
            &format!("&id={song}&submission=true")
        )
        .await["subsonic-response"]["status"],
        "ok"
    );
    for (kind, extra) in [
        ("highest", String::new()),
        ("frequent", String::new()),
        ("recent", String::new()),
        ("starred", String::new()),
        ("byGenre", "&genre=Electronic".to_owned()),
    ] {
        let response = subsonic_json(
            &router,
            "getAlbumList2",
            api_key,
            &format!("&type={kind}{extra}"),
        )
        .await;
        assert_eq!(
            response["subsonic-response"]["albumList2"]["album"][0]["id"],
            album.to_string(),
            "album list type {kind}"
        );
    }

    let share = subsonic_json(
        &router,
        "createShare",
        api_key,
        &format!("&id={song}&description=Golden"),
    )
    .await;
    let share_id = share["subsonic-response"]["shares"]["share"][0]["id"]
        .as_str()
        .unwrap();
    // Same reasoning as playlist.owner: a share is read by its owner, so the
    // empty username told the client the share belonged to nobody.
    assert_eq!(
        share["subsonic-response"]["shares"]["share"][0]["username"],
        "sub-admin"
    );
    let share_url = share["subsonic-response"]["shares"]["share"][0]["url"]
        .as_str()
        .unwrap();
    assert!(share_url.starts_with("http://waveflow.test/share/"));
    let public = router
        .clone()
        .oneshot(Request::get(share_url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(public.headers()["cache-control"], "no-store");
    let public = json_body(public).await;
    assert_eq!(public["tracks"][0]["id"], song.to_string());
    let public_stream_url = public["tracks"][0]["streamUrl"].as_str().unwrap();
    let public_stream = router
        .clone()
        .oneshot(
            Request::get(public_stream_url)
                .header("range", "bytes=0-3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_stream.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        public_stream
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
        "RIFF"
    );
    let foreign_public_stream = router
        .clone()
        .oneshot(
            Request::get(format!("{share_url}/tracks/{}/stream", Uuid::new_v4(),))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(foreign_public_stream.status(), StatusCode::NOT_FOUND);
    let listed_shares = subsonic_json(&router, "getShares", api_key, "").await;
    assert_eq!(listed_shares["subsonic-response"]["status"], "ok");
    assert!(listed_shares["subsonic-response"]["shares"]["share"][0]
        .get("url")
        .is_none());
    assert_eq!(
        listed_shares["subsonic-response"]["shares"]["share"][0]["username"],
        "sub-admin"
    );
    assert_eq!(
        subsonic_json(
            &router,
            "updateShare",
            api_key,
            &format!("&id={share_id}&description=Changed")
        )
        .await["subsonic-response"]["status"],
        "ok"
    );
    let journal_entities = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT entity_type FROM sync_event WHERE user_id=? ORDER BY entity_type",
    )
    .bind(admin.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert!(journal_entities.contains(&"playlist".to_owned()));
    assert!(journal_entities.contains(&"favorite".to_owned()));
    assert!(journal_entities.contains(&"rating".to_owned()));
    assert!(journal_entities.contains(&"scrobble".to_owned()));
    assert!(journal_entities.contains(&"queue".to_owned()));
    assert!(journal_entities.contains(&"share".to_owned()));

    let default_user = subsonic_json(
        &router,
        "createUser",
        api_key,
        "&username=sub-default&password=default-secret&email=default@example.invalid",
    )
    .await;
    assert!(default_user["subsonic-response"].get("user").is_none());
    let default_user = subsonic_json(&router, "getUser", api_key, "&username=sub-default").await;
    let default_folders = default_user["subsonic-response"]["user"]["folder"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(default_folders.contains(&library.to_string()));
    assert!(default_folders.contains(&secondary_library.to_string()));

    let unicode_username = subsonic_json(
        &router,
        "createUser",
        api_key,
        "&username=%C3%A9lodie&password=unicode-user-secret&email=unicode@example.invalid",
    )
    .await;
    assert_eq!(unicode_username["subsonic-response"]["status"], "ok");
    assert!(state
        .db
        .account_by_username("élodie")
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        subsonic_json(&router, "deleteUser", api_key, "&username=%C3%A9lodie").await
            ["subsonic-response"]["status"],
        "ok"
    );
    assert!(state
        .db
        .account_by_username("élodie")
        .await
        .unwrap()
        .is_none());

    assert_eq!(
        subsonic_json(&router, "deleteUser", api_key, "&username=sub-default").await
            ["subsonic-response"]["status"],
        "ok"
    );

    assert_eq!(
        subsonic_json(&router, "getUsers", api_key, "").await["subsonic-response"]["status"],
        "ok"
    );
    let encoded_listener_password = hex::encode("listener-secret");
    let created_user = subsonic_json(
        &router,
        "createUser",
        api_key,
        &format!(
            "&username=sub-listener&password=enc:{encoded_listener_password}&email=listener@example.invalid&adminRole=false&musicFolderId={library}"
        ),
    )
    .await;
    assert!(created_user["subsonic-response"].get("user").is_none());

    let user = subsonic_json(&router, "getUser", api_key, "&username=sub-listener").await;
    assert_eq!(
        user["subsonic-response"]["user"]["folder"],
        serde_json::json!([library.to_string()])
    );
    let listener_before = state
        .db
        .account_by_username("sub-listener")
        .await
        .unwrap()
        .unwrap();
    let listener_folders = router
        .clone()
        .oneshot(
            Request::get(
                "/rest/getMusicFolders?u=sub-listener&p=listener-secret&v=1.16.1&c=golden&f=json",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listener_folders.status(), StatusCode::OK);
    assert_eq!(
        json_body(listener_folders).await["subsonic-response"]["musicFolders"]["musicFolder"][0]
            ["id"],
        library.to_string()
    );
    let listener_now_playing = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/scrobble?u=sub-listener&p=listener-secret&v=1.16.1&c=golden&f=json&id={song}&submission=false"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listener_now_playing.status(), StatusCode::OK);
    let all_now_playing = subsonic_json(&router, "getNowPlaying", api_key, "").await;
    assert!(all_now_playing["subsonic-response"]["nowPlaying"]["song"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["username"] == "sub-listener"));

    let updated_password = hex::encode("updated-secret");
    let updated_user = subsonic_json(
        &router,
        "updateUser",
        api_key,
        &format!(
            "&username=sub-listener&locked=false&password=enc:{updated_password}&musicFolderId={secondary_library}"
        ),
    )
    .await;
    assert!(updated_user["subsonic-response"].get("user").is_none());
    let listener_folders = router
        .clone()
        .oneshot(
            Request::get(
                "/rest/getMusicFolders?u=sub-listener&p=updated-secret&v=1.16.1&c=golden&f=json",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listener_folders.status(), StatusCode::OK);
    assert_eq!(
        json_body(listener_folders).await["subsonic-response"]["musicFolders"]["musicFolder"][0]
            ["id"],
        secondary_library.to_string()
    );

    let denied_admin = router
        .clone()
        .oneshot(
            Request::get("/rest/getUsers?u=sub-listener&p=updated-secret&v=1.16.1&c=golden&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied_admin.status(), StatusCode::OK);
    assert_eq!(
        json_body(denied_admin).await["subsonic-response"]["error"]["code"],
        50
    );

    let changed_password = hex::encode("changed-secret");
    assert_eq!(
        subsonic_json(
            &router,
            "changePassword",
            api_key,
            &format!("&username=sub-listener&password=enc:{changed_password}")
        )
        .await["subsonic-response"]["status"],
        "ok"
    );
    let listener_after = state
        .db
        .account_by_username("sub-listener")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listener_before.password_hash, listener_after.password_hash);
    assert_eq!(
        subsonic_json(&router, "deleteUser", api_key, "&username=sub-listener").await
            ["subsonic-response"]["status"],
        "ok"
    );

    let cover = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getCoverArt?apiKey={api_key}&id={artwork}&v=1.16.1&c=golden"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cover.status(), StatusCode::OK);
    assert!(cover.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("image/"));

    // The same cover over the native API. Deliberately authenticated with a
    // native session rather than the Subsonic key above: the whole point is
    // that a native client needs no second set of credentials. Without this
    // route a remote catalogue rendered with no covers at all — payloads carry
    // `artwork_hash`, and only the Subsonic facade could resolve it.
    let session = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "sub-admin",
                "password": web_password,
                "device_name": "artwork-probe"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::OK);
    let native_token = json_body(session).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let native_cover = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/artwork/{artwork}"))
                .header("authorization", format!("Bearer {native_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native_cover.status(), StatusCode::OK);
    assert!(native_cover.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("image/"));
    // An entity id resolves too, so a client holding only a song need not first
    // read its hash.
    let by_song = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/artwork/{song}"))
                .header("authorization", format!("Bearer {native_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(by_song.status(), StatusCode::OK);
    // No bearer, no cover: the image is not public just because the hash is
    // unguessable.
    let anonymous_cover = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/artwork/{artwork}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous_cover.status(), StatusCode::UNAUTHORIZED);

    let download = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/download?apiKey={api_key}&id={song}&v=1.16.1&c=golden"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.headers()["content-disposition"], "attachment");
    assert!(
        download
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len()
            > 44
    );

    let source_bitrate = snapshot.songs.first().unwrap().bitrate.unwrap() as u32;
    let unlimited_stream = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/stream.view?apiKey={api_key}&id={song}&maxBitRate=0&v=1.16.1&c=golden"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unlimited_stream.status(), StatusCode::OK);
    assert_eq!(unlimited_stream.headers()["content-type"], "audio/wav");
    let ranged_stream = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/stream.view?apiKey={api_key}&id={song}&maxBitRate={source_bitrate}&v=1.16.1&c=golden"
            ))
            .header("range", "bytes=0-3")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ranged_stream.status(), StatusCode::PARTIAL_CONTENT);
    assert!(ranged_stream.headers()["content-range"]
        .to_str()
        .unwrap()
        .starts_with("bytes 0-3/"));
    assert_eq!(
        ranged_stream
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
        "RIFF"
    );

    let invalid_range = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/download.view?apiKey={api_key}&id={song}&v=1.16.1&c=golden"
            ))
            .header("range", "bytes=999999-")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_range.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert!(invalid_range.headers()["content-range"]
        .to_str()
        .unwrap()
        .starts_with("bytes */"));

    let direct_stream = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/stream.view?apiKey={api_key}&id={song}&maxBitRate={source_bitrate}&v=1.16.1&c=DSub"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(direct_stream.status(), StatusCode::OK);
    assert_eq!(direct_stream.headers()["content-type"], "audio/wav");
    assert!(direct_stream
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .starts_with(b"RIFF"));

    let transcoded_stream = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/stream.view?apiKey={api_key}&id={song}&maxBitRate=32&v=1.16.1&c=DSub"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(transcoded_stream.status(), StatusCode::OK);
    assert_eq!(transcoded_stream.headers()["content-type"], "audio/mpeg");
    assert!(
        transcoded_stream
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len()
            > 32
    );

    for (method, extra) in [
        ("deleteShare", format!("&id={share_id}")),
        ("deletePlaylist", format!("&id={playlist}")),
    ] {
        assert_eq!(
            subsonic_json(&router, method, api_key, &extra).await["subsonic-response"]["status"],
            "ok"
        );
    }

    let wrong = router
        .oneshot(
            Request::get("/rest/ping?u=sub-admin&p=wrong&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::OK);
    assert_eq!(
        json_body(wrong).await["subsonic-response"]["error"]["code"],
        40
    );
}

#[tokio::test]
async fn subsonic_blurs_foreign_catalog_and_rate_limits_failed_authentication() {
    let (_temp, config, state) = test_app().await;
    let web_hash = security::hash_password("web-password-for-test").unwrap();
    let owner = state
        .db
        .create_account("sub-owner", &web_hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let outsider = state
        .db
        .create_account("sub-outsider", &web_hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    // A dedicated account for the throttling assertion: the rate window is a
    // process-wide map keyed by the supplied username, so reusing one of the
    // accounts asserted on elsewhere would leak between tests.
    let throttled = state
        .db
        .create_account("sub-throttled", &web_hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    for (actor, user, password, key) in [
        (owner, owner, "owner-sub-password", "wfsk_owner"),
        (owner, outsider, "outsider-sub-password", "wfsk_outsider"),
        (owner, throttled, "throttled-sub-password", "wfsk_throttled"),
    ] {
        state
            .db
            .set_subsonic_credential(
                actor,
                user,
                &state.secret_box.encrypt(password.as_bytes()).unwrap(),
                &security::token_hash(key),
                now_ms(),
            )
            .await
            .unwrap();
    }
    let music = config.data_dir.join("isolated-subsonic");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Private.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Private Subsonic",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library,
            name: "Private Subsonic".into(),
            root_path: root,
        },
    )
    .await;
    let song = state.db.list_tracks_for_user(owner, library).await.unwrap()[0].id;
    let router = waveflow_server::app(&config, state);
    let foreign = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getSong?apiKey=wfsk_outsider&id={song}&f=json"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::OK);
    assert_eq!(
        json_body(foreign).await["subsonic-response"]["error"]["code"],
        70
    );

    let foreign_star = router
        .clone()
        .oneshot(
            Request::get(format!("/rest/star?apiKey=wfsk_outsider&id={song}&f=json"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(foreign_star.status(), StatusCode::OK);
    assert_eq!(
        json_body(foreign_star).await["subsonic-response"]["error"]["code"],
        70
    );

    // Repeated failures throttle the credential. That refusal is no longer
    // visible in the status line — every Subsonic answer is HTTP 200 — so the
    // limiter is asserted where it actually bites: once the window is full,
    // even the correct password is refused.
    for _ in 0..=20 {
        let refused = router
            .clone()
            .oneshot(
                Request::get("/rest/ping?u=sub-throttled&p=wrong&f=json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::OK);
        assert_eq!(
            json_body(refused).await["subsonic-response"]["error"]["code"],
            40
        );
    }
    let throttled_but_correct = router
        .clone()
        .oneshot(
            Request::get("/rest/ping?u=sub-throttled&p=throttled-sub-password&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(throttled_but_correct.status(), StatusCode::OK);
    assert_eq!(
        json_body(throttled_but_correct).await["subsonic-response"]["error"]["code"],
        40
    );

    let unknown = router
        .clone()
        .oneshot(
            Request::get("/rest/ping?u=unknown-enum&p=wrong&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let wrong = router
        .oneshot(
            Request::get("/rest/ping?u=sub-outsider&p=wrong&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status(), wrong.status());
    assert_eq!(body_text(unknown).await, body_text(wrong).await);
}
