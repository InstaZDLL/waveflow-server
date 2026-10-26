//! Lyrics uploaded to a track.

use super::*;

/// Lyrics uploaded to a track (RFC-012 decision 6), through the router.
///
/// Passwords are drawn at run time: a literal handed to `hash_password` is
/// what CodeQL's `rust/hard-coded-cryptographic-value` reports, and the
/// ruleset blocks a pull request on it.
#[tokio::test]
async fn uploaded_lyrics_lead_the_order_download_as_they_came_and_stay_off_subsonic() {
    let (_temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("lyrics-owner", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let listener = state
        .db
        .create_account("lyrics-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("uploaded-lyrics");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Song.wav"));
    // A scanned synced sheet, so every rank below has a neighbour to beat.
    std::fs::write(music.join("Song.lrc"), "[00:01.00]from the sidecar\n").unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Lyrics", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner,
            library_id,
            listener,
            waveflow_server::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Lyrics".into(),
            root_path: root,
        },
    )
    .await;
    let track = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()[0]
        .id;
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "lyrics-owner", &password).await;
    let listener_token = login_token(&router, "lyrics-listener", &password).await;

    let send = |method: Method, uri: String, token: &str, body: Vec<u8>| {
        let router = router.clone();
        let token = token.to_owned();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    let sources = |list: &serde_json::Value| -> Vec<String> {
        list["structuredLyrics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|sheet| sheet["source"].as_str().unwrap().to_owned())
            .collect()
    };
    let lyrics_uri = format!("/api/v2/tracks/{track}/lyrics");

    // Before any upload: the scanned sheet, naming its source.
    let before = json_body(send(Method::GET, lyrics_uri.clone(), &owner_token, vec![]).await).await;
    assert_eq!(sources(&before), ["sidecar_lrc"]);

    // A plain upload does not take the timings away from a synced scan.
    let txt = send(
        Method::PUT,
        format!("{lyrics_uri}/txt"),
        &owner_token,
        b"plain words\n".to_vec(),
    )
    .await;
    assert_eq!(txt.status(), StatusCode::OK);
    assert_eq!(
        sources(&json_body(txt).await),
        ["sidecar_lrc", "upload_txt"]
    );

    // A synced upload outranks the scan, and TTML outranks LRC.
    let lrc = send(
        Method::PUT,
        format!("{lyrics_uri}/lrc"),
        &owner_token,
        b"[la:en]\n[00:02.00]uploaded lrc\n".to_vec(),
    )
    .await;
    assert_eq!(lrc.status(), StatusCode::OK);
    let ttml_body = concat!(
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="fr"><body><div>"#,
        r#"<p begin="0:03.000"><span>bonjour</span> <span>tout</span></p>"#,
        r#"</div></body></tt>"#
    );
    let ttml = send(
        Method::PUT,
        format!("{lyrics_uri}/ttml"),
        &owner_token,
        ttml_body.as_bytes().to_vec(),
    )
    .await;
    assert_eq!(ttml.status(), StatusCode::OK);
    let ordered = json_body(ttml).await;
    assert_eq!(
        sources(&ordered),
        ["upload_ttml", "upload_lrc", "sidecar_lrc", "upload_txt"]
    );
    assert_eq!(ordered["structuredLyrics"][0]["lang"], "fr");
    assert_eq!(
        ordered["structuredLyrics"][0]["line"][0]["value"],
        "bonjour tout"
    );
    assert_eq!(ordered["structuredLyrics"][0]["line"][0]["start"], 3000);

    // The download gives the first sheet back as it was uploaded, never
    // rendered; with `format`, the upload named.
    let file = send(
        Method::GET,
        format!("{lyrics_uri}/file"),
        &owner_token,
        vec![],
    )
    .await;
    assert_eq!(file.status(), StatusCode::OK);
    assert_eq!(file.headers()["content-type"], "application/octet-stream");
    assert_eq!(file.headers()["x-content-type-options"], "nosniff");
    let disposition = file.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        disposition.starts_with("attachment;") && disposition.ends_with(".ttml"),
        "{disposition}"
    );
    assert_eq!(body_text(file).await, ttml_body);
    let txt_file = send(
        Method::GET,
        format!("{lyrics_uri}/file?format=txt"),
        &owner_token,
        vec![],
    )
    .await;
    assert_eq!(body_text(txt_file).await, "plain words\n");

    // A listener reads, and may not write: refused like a track it cannot see.
    let read = send(Method::GET, lyrics_uri.clone(), &listener_token, vec![]).await;
    assert_eq!(read.status(), StatusCode::OK);
    for (method, uri) in [
        (Method::PUT, format!("{lyrics_uri}/lrc")),
        (Method::DELETE, format!("{lyrics_uri}/lrc")),
    ] {
        let refused = send(method, uri, &listener_token, b"[00:01.00]x".to_vec()).await;
        assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    }

    // Nothing usable is refused, and so is a format the route does not know.
    let empty = send(
        Method::PUT,
        format!("{lyrics_uri}/txt"),
        &owner_token,
        b" \n".to_vec(),
    )
    .await;
    assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let unknown = send(
        Method::PUT,
        format!("{lyrics_uri}/srt"),
        &owner_token,
        b"1".to_vec(),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    // More than a mebibyte is refused before it is read.
    let large = send(
        Method::PUT,
        format!("{lyrics_uri}/txt"),
        &owner_token,
        // One byte past the cap: the route's own ceiling answers it, not the
        // service after the body has been read.
        vec![b'a'; 1024 * 1024 + 1],
    )
    .await;
    assert_eq!(large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    // And a sheet well past the router's 16 KiB default is taken: the route
    // carries its own ceiling.
    let long = send(
        Method::PUT,
        format!("{lyrics_uri}/txt"),
        &owner_token,
        "a line of a long song
"
        .repeat(10_000)
        .into_bytes(),
    )
    .await;
    assert_eq!(long.status(), StatusCode::OK);

    // Deleting an upload falls back; deleting it twice finds nothing.
    for format in ["ttml", "lrc", "txt"] {
        let removed = send(
            Method::DELETE,
            format!("{lyrics_uri}/{format}"),
            &owner_token,
            vec![],
        )
        .await;
        assert_eq!(removed.status(), StatusCode::NO_CONTENT, "{format}");
    }
    let again = send(
        Method::DELETE,
        format!("{lyrics_uri}/txt"),
        &owner_token,
        vec![],
    )
    .await;
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
    let after = json_body(send(Method::GET, lyrics_uri.clone(), &owner_token, vec![]).await).await;
    assert_eq!(sources(&after), ["sidecar_lrc"]);
    // A scanned synced sheet downloads as LRC.
    let scanned = send(
        Method::GET,
        format!("{lyrics_uri}/file"),
        &owner_token,
        vec![],
    )
    .await;
    assert_eq!(body_text(scanned).await, "[00:01.00]from the sidecar\n");

    // The Subsonic service the façade reads never sees an upload, and its
    // sheets carry no `source`: its contract is frozen.
    send(
        Method::PUT,
        format!("{lyrics_uri}/ttml"),
        &owner_token,
        ttml_body.as_bytes().to_vec(),
    )
    .await;
    let facade = state.services.lyrics(owner, track).await.unwrap();
    assert_eq!(facade.structured_lyrics.len(), 1);
    assert!(facade.structured_lyrics[0].source.is_none());
    assert!(serde_json::to_value(&facade.structured_lyrics[0])
        .unwrap()
        .get("source")
        .is_none());

    // A track whose file is gone takes no upload: the read refuses it, and a
    // write the read would refuse is no write at all.
    sqlx::query("UPDATE track SET is_available = 0 WHERE id = ?")
        .bind(track.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let gone = send(
        Method::PUT,
        format!("{lyrics_uri}/lrc"),
        &owner_token,
        b"[00:01.00]too late".to_vec(),
    )
    .await;
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
    // The answer alone proves nothing: the lyrics read after the write says
    // 404 for this track whether or not the row went in.
    let stored: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM track_lyrics_upload WHERE track_id = ? AND content LIKE ?",
    )
    .bind(track.to_string())
    .bind("%too late%")
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(stored, 0);
}
