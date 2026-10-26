//! Importing an `.m3u` file as a playlist.

use super::*;

/// An `.m3u` import places what it can and reports every entry it cannot.
///
/// The report is the feature. A playlist that silently arrives a third shorter
/// is worse than a refused import, so every shape of failure here is asserted
/// by name and not merely counted: a path that matches nothing, a bare file
/// name that matches two tracks, a web radio, a track named twice, and a track
/// that exists in a library this account cannot see.
///
/// The successes are the shapes a playlist file actually arrives in — relative
/// to its own folder, absolute on someone else's filesystem, Windows-shaped,
/// a percent-encoded `file://` URL, and one that only the `#EXTINF` credit can
/// place — plus a file that is not UTF-8, which is the common case for `.m3u`
/// and the one that mangles every accented title when it is guessed wrong.
///
/// It ends on the round trip: the playlist is exported and imported again, and
/// everything resolves. That is the strongest evidence the reader and the
/// writer agree, and it costs one assertion.
#[tokio::test]
async fn an_m3u_import_places_what_it_can_and_reports_the_rest() {
    let (_temp, config, state) = test_app().await;
    // Inserted rather than created, and authenticating by API token rather
    // than by password. Hashing a password here put a string literal into a
    // cryptographic call, which `rust/hard-coded-cryptographic-value` reads as
    // a hard-coded password however random the rest of the token is — measured
    // on this very branch, where CodeQL failed on the generated prefix. These
    // accounts never log in, so there is nothing to hash.
    let owner = uuid::Uuid::new_v4();
    let stranger = uuid::Uuid::new_v4();
    for (id, username) in [(owner, "import-owner"), (stranger, "import-stranger")] {
        sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
             VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(now_ms())
        .bind(now_ms())
        .execute(state.db.pool())
        .await
        .unwrap();
    }

    let shelf = |who: uuid::Uuid, name: &'static str| {
        let state = &state;
        let config = &config;
        async move {
            let music = config.data_dir.join(name);
            std::fs::create_dir_all(&music).unwrap();
            let library = state
                .db
                .create_library(
                    who,
                    name,
                    &std::fs::canonicalize(&music).unwrap(),
                    LibraryVisibility::Private,
                    now_ms(),
                )
                .await
                .unwrap();
            let scan = state
                .db
                .create_scan_job(library, Some(who), "manual")
                .await
                .unwrap();
            state.db.start_scan_job(scan, 8, false).await.unwrap();
            (library, scan)
        }
    };
    let (library, scan) = shelf(owner, "import-music").await;
    // Paths rather than titles are what this test is about, so each fixture
    // names its own place on disk. Two of them share a file name on purpose.
    for (index, path, title, artist) in [
        (
            900,
            "Björk/Vespertine/01 Hidden Place.flac",
            "Hidden Place",
            "Björk",
        ),
        (901, "Björk/Vespertine/02 Cocoon.flac", "Cocoon", "Björk"),
        (902, "Björk/Post/01 Army of Me.flac", "Army of Me", "Björk"),
        (
            903,
            "Compilations/Shared/common.flac",
            "A Shared Name",
            "First Band",
        ),
        (
            904,
            "Compilations/Other/common.flac",
            "A Shared Name Too",
            "Second Band",
        ),
        (905, "Moved/elsewhere.flac", "Pagan Poetry", "Björk"),
        (906, "Björk/Homogenic/03 Unravel.flac", "Unravel", "Björk"),
        // Two names the format cannot carry as written: a line starting with
        // "#" is a directive, and a "%" would come back percent-decoded.
        // Reachable only through the export, which is where they are asserted.
        (907, "#1 Hit.flac", "Number One", "Chart Band"),
        (908, "Mix 100% Live.flac", "Hundred Percent", "Chart Band"),
    ] {
        let mut input = browse_input(index, title, "Vespertine", artist, Some(1), Some(1));
        input.relative_path = path.into();
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

    // A second account, with a track of its own at a path the import below
    // names. Nothing about it may reach the owner's playlist, and the report
    // must not say it exists either.
    let (foreign, foreign_scan) = shelf(stranger, "stranger-music").await;
    let mut hidden = browse_input(910, "Private Thing", "Private", "Nobody", Some(1), Some(1));
    hidden.relative_path = "Nobody/Private/01 Private Thing.flac".into();
    state
        .db
        .apply_catalog_track(foreign, foreign_scan, &hidden, None, false)
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(foreign)
        .await
        .unwrap();
    state.db.finish_scan_job(foreign_scan, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let (_record, token) = state
        .services
        .create_api_token(owner, &[], "import-owner", "importer", &[])
        .await
        .unwrap();
    let import = |uri: String, body: Vec<u8>, token: String| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "audio/x-mpegurl")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            (status, json_body(response).await)
        }
    };

    // The file. Written as bytes because one line of it is not UTF-8: "Björk"
    // spelled the way a Windows player writes it.
    let mut file: Vec<u8> = Vec::new();
    file.extend_from_slice(b"#EXTM3U\r\n");
    file.extend_from_slice(b"#PLAYLIST:From another player\r\n");
    // Relative to the folder the playlist sat in.
    file.extend_from_slice(b"#EXTINF:300,Bj\xF6rk - Hidden Place\r\n");
    file.extend_from_slice(b"Bj\xF6rk/Vespertine/01 Hidden Place.flac\r\n");
    // Absolute, on a filesystem that is not this server's.
    file.extend_from_slice(b"/home/someone/Music/Bj\xF6rk/Vespertine/02 Cocoon.flac\r\n");
    // Windows-shaped, with a drive letter.
    file.extend_from_slice(b"C:\\Users\\someone\\Music\\Bj\xF6rk\\Post\\01 Army of Me.flac\r\n");
    // A percent-encoded file URL.
    file.extend_from_slice(b"file:///home/someone/Music/Moved/elsewhere.flac\r\n");
    // Nothing of its path survives, and only the credit can place it.
    file.extend_from_slice(b"#EXTINF:240,Bj\xF6rk - Unravel\r\n");
    file.extend_from_slice(b"D:\\Backup\\unknown\\7331.flac\r\n");
    // A bare file name two albums carry.
    file.extend_from_slice(b"common.flac\r\n");
    // A web radio.
    file.extend_from_slice(b"http://radio.example/stream\r\n");
    // A track of another account's library.
    file.extend_from_slice(b"Nobody/Private/01 Private Thing.flac\r\n");
    // Nothing at all.
    file.extend_from_slice(b"Ghost/Album/99 Nowhere.flac\r\n");
    // And the first entry again, which is placed again: a playlist holds a
    // track as many times as it is listed (RFC-020 decision 1).
    file.extend_from_slice(b"Bj\xF6rk/Vespertine/01 Hidden Place.flac\r\n");
    // The ambiguous name once more, shouted, and with its folders this time.
    file.extend_from_slice(b"COMPILATIONS/SHARED/COMMON.FLAC\r\n");

    // The dry run answers the whole report and creates nothing, which is what
    // lets a client show the losses before they happen.
    let (status, report) = import(
        "/api/v2/playlists/import?dry_run=true".into(),
        file.clone(),
        token.clone(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a dry run creates nothing: {report}"
    );
    assert!(report["playlist"].is_null());
    assert_eq!(report["name"], "From another player", "the file named it");
    assert_eq!(
        report["encoding"], "windows-1252",
        "the bytes decide the encoding, not the extension"
    );
    assert_eq!(report["entries"], 11);
    assert_eq!(report["resolved"], 7);
    let reasons = |report: &serde_json::Value| {
        report["unresolved"]
            .as_array()
            .expect("the report lists what it could not place")
            .iter()
            .map(|entry| {
                (
                    entry["line"].as_i64().unwrap(),
                    entry["reason"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        reasons(&report),
        [
            (10, "ambiguous".to_owned()),
            (11, "not-a-path".to_owned()),
            // A track in a library this account cannot reach is not found,
            // which is the same answer as one that does not exist — and the
            // two lines below are indistinguishable in this report, by design.
            (12, "no-match".to_owned()),
            (13, "no-match".to_owned()),
        ],
        "every failure named, and placed on its line in the file"
    );
    let listed = router
        .clone()
        .oneshot(
            Request::get("/api/v2/playlists")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(listed).await.as_array().unwrap().len(),
        0,
        "the dry run wrote nothing"
    );

    // And the import itself, with a name of the caller's choosing.
    let (status, created) = import(
        "/api/v2/playlists/import?name=Evening%20from%20elsewhere".into(),
        file.clone(),
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "Evening from elsewhere");
    assert_eq!(reasons(&created), reasons(&report), "the same report");
    let playlist = &created["playlist"];
    assert_eq!(playlist["name"], "Evening from elsewhere");
    let titles = playlist["songs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|song| song["title"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        titles,
        [
            "Hidden Place",
            "Cocoon",
            "Army of Me",
            "Pagan Poetry",
            "Unravel",
            "Hidden Place",
            // Matched through two folders in the wrong case: the comparison
            // folds ASCII, which is what a playlist written on Windows needs,
            // and the folders are what separate this track from the one on
            // line 10 carrying the same file name.
            "A Shared Name",
        ],
        "file order, and the fourth was placed by its path while the fifth \
         was placed by its credit alone"
    );

    // The round trip. What this server writes, this server reads.
    let playlist_id = playlist["id"].as_str().unwrap().to_owned();
    let exported = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/playlists/{playlist_id}/export"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(exported.status(), StatusCode::OK);
    assert_eq!(
        exported
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "audio/x-mpegurl; charset=utf-8"
    );
    let disposition = exported
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .expect("a header value stays a header value")
        .to_owned();
    assert!(
        disposition.contains("filename=\"Evening from elsewhere.m3u8\""),
        "{disposition}"
    );
    let text = body_text(exported).await;
    assert!(
        text.starts_with("#EXTM3U\n#PLAYLIST:Evening from elsewhere\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "#EXTINF:120,Björk - Hidden Place\n./Björk/Vespertine/01 Hidden Place.flac\n"
        ),
        "{text}"
    );

    let (status, again) = import(
        "/api/v2/playlists/import?name=Round%20trip&dry_run=true".into(),
        text.into_bytes(),
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["encoding"], "utf-8", "what it writes is UTF-8");
    assert_eq!(again["entries"], 7, "the playlist holds seven entries");
    assert_eq!(again["resolved"], 7);
    assert_eq!(
        again["unresolved"].as_array().unwrap().len(),
        0,
        "an export this server wrote resolves whole: {again}"
    );

    // **The round trip for two names the format cannot carry naively.** A
    // track at a library's root called "#1 Hit.flac" written bare would be read
    // back as a comment and vanish without being reported — not even counted —
    // and a "%" in a name would come back percent-decoded. The export prefixes
    // "./" and escapes the "%", so what this server writes is what it reads.
    // Raised in review, with both names named.
    let awkward = state
        .services
        .list_songs(owner, &Default::default())
        .await
        .unwrap()
        .into_iter()
        .filter(|song| song.title == "Number One" || song.title == "Hundred Percent")
        .map(|song| song.id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(awkward.len(), 2, "both awkward names are in the catalogue");
    let created = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v2/playlists")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "name": "Awkward", "track_ids": awkward }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let awkward_id = json_body(created).await["id"].as_str().unwrap().to_owned();
    let exported = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/playlists/{awkward_id}/export"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let text = body_text(exported).await;
    assert!(
        text.contains("./#1 Hit.flac\n") && text.contains("./Mix 100%25 Live.flac\n"),
        "the writer puts both names where the reader can find them: {text}"
    );
    let (_status, back) = import(
        "/api/v2/playlists/import?name=Awkward%20again&dry_run=true".into(),
        text.into_bytes(),
        token.clone(),
    )
    .await;
    assert_eq!(back["entries"], 2, "neither line was read as a comment");
    assert_eq!(
        back["resolved"], 2,
        "and neither name was mangled on the way back: {back}"
    );

    // A name is free text, and both outputs are structured. The file is
    // line-structured, so a newline in a name would end the `#PLAYLIST` line
    // early and make the rest look like a path — an entry of its own, in a
    // file the next import reads. A header value carries no newline at all,
    // so the same name would have failed the response instead of the parse.
    // Neither spelling of the name is passed through untouched.
    let (status, hostile) = import(
        "/api/v2/playlists/import?name=Soir%C3%A9e%0A/etc/passwd".into(),
        b"Bj\xC3\xB6rk/Vespertine/01 Hidden Place.flac\n".to_vec(),
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let hostile_id = hostile["playlist"]["id"].as_str().unwrap().to_owned();
    let exported = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/playlists/{hostile_id}/export"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        exported.status(),
        StatusCode::OK,
        "a header value with a newline in it would have failed the response"
    );
    let disposition = exported
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        disposition.contains("filename=\"Soireetcpasswd.m3u8\"")
            && disposition.contains("filename*=UTF-8''Soir%C3%A9e%0A%2Fetc%2Fpasswd.m3u8"),
        "the quoted form is an ASCII subset and the full name travels encoded: {disposition}"
    );
    let text = body_text(exported).await;
    assert!(
        text.starts_with("#EXTM3U\n#PLAYLIST:Soirée /etc/passwd\n"),
        "one name, one line: {text}"
    );
    let (_status, reread) = import(
        "/api/v2/playlists/import?name=Reread&dry_run=true".into(),
        text.into_bytes(),
        token.clone(),
    )
    .await;
    assert_eq!(
        reread["entries"], 1,
        "and the name did not become an entry of its own"
    );

    // **A UTF-16 file is read by its byte-order mark.** Notepad's "Unicode"
    // and some players' exports write it. Read as Windows-1252, every other
    // character came back a NUL and nothing in the file resolved.
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend(
        "#EXTM3U\r\n#EXTINF:300,Björk - Hidden Place\r\nBjörk/Vespertine/01 Hidden Place.flac\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    let (status, wide) = import(
        "/api/v2/playlists/import?name=Wide&dry_run=true".into(),
        utf16,
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{wide}");
    assert_eq!(wide["entries"], 1);
    assert_eq!(wide["resolved"], 1, "the path it names is found: {wide}");
    assert_eq!(
        wide["encoding"], "utf-16le",
        "and the mark named the encoding"
    );

    // A file nothing names is refused rather than stored namelessly.
    let (status, _body) = import(
        "/api/v2/playlists/import".into(),
        b"Ghost/Album/99 Nowhere.flac\n".to_vec(),
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // **A named import that places nothing still creates the playlist.** The
    // caller asked for one and the report says what became of every line, so
    // an empty playlist with an explanation beats a refusal that leaves the
    // file unexplained. Asserted because it is a decision rather than a
    // consequence — raised in review, where it was untested.
    let (status, empty) = import(
        "/api/v2/playlists/import?name=Ghost".into(),
        b"Ghost/Album/99 Nowhere.flac\n".to_vec(),
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(empty["resolved"], 0);
    assert!(
        empty["playlist"].is_object(),
        "the playlist exists, and holds nothing: {empty}"
    );
    assert_eq!(empty["playlist"]["name"], "Ghost");
    assert_eq!(empty["playlist"]["songs"].as_array().unwrap().len(), 0);
    assert_eq!(reasons(&empty), [(1, "no-match".to_owned())]);

    // Another account reaches neither the export nor the playlist, and the
    // refusal is the same 404 a missing one would answer.
    let (_record, foreign_token) = state
        .services
        .create_api_token(stranger, &[], "import-stranger", "intruder", &[])
        .await
        .unwrap();
    let refused = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/playlists/{playlist_id}/export"))
                .header("authorization", format!("Bearer {foreign_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
}
