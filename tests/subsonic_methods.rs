//! The facade methods that are neither browsing nor a wire shape.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::authentication::now_ms;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;
use waveflow_server::services::ServiceError;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

/// The facade could not trigger a rescan the native API has always been able to
/// trigger, and answered a not-implemented error for surfaces clients open by
/// default. Both gaps are closed without inventing data.
#[tokio::test]
async fn facade_controls_scans_and_answers_its_remaining_methods() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_asymmetry-key";
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("asym-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"asym-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let outsider = state
        .db
        .create_account("asym-outsider", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();

    let seed = |account: Uuid, name: &'static str, titles: Vec<&'static str>| {
        let state = state.clone();
        let root = config.data_dir.join(name);
        async move {
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
                .start_scan_job(scan, titles.len() as i64, false)
                .await
                .unwrap();
            for (index, title) in titles.into_iter().enumerate() {
                let mut input = browse_input(
                    index + 120,
                    title,
                    "Asym Album",
                    "Asym Artist",
                    Some(1),
                    Some(1),
                );
                input.relative_path = format!("{name}-{index}.flac");
                input.quick_hash = format!("{:064x}", index + 7_000 + name.len() * 100);
                input.full_hash = format!("{:064x}", index + 8_000 + name.len() * 100);
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
            library
        }
    };
    let library = seed(owner, "asym-own", vec!["One", "Two", "Three"]).await;
    seed(outsider, "asym-foreign", vec!["Hidden"]).await;

    let router = waveflow_server::app(&config, state.clone());

    // Idle, and counting only what this account can reach: the outsider's
    // fourth track must not appear in the owner's total.
    let status = subsonic_json(&router, "getScanStatus", api_key, "").await;
    let status = &status["subsonic-response"]["scanStatus"];
    assert_eq!(status["scanning"], false);
    assert_eq!(status["count"], 3);

    // A membership revoked between the lookup and the queuing must not leave
    // a job behind. The window cannot be interleaved deterministically, so
    // what is asserted is the property that makes it safe: the insert itself
    // requires a library_member row, and a non-member is exactly the state a
    // revocation leaves behind.
    assert!(state
        .db
        .create_scan_job_for_user(outsider, library, "manual")
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        state.services.start_library_scan(outsider, library).await,
        Err(ServiceError::NotFound)
    ));
    // Scoped to the owner's library: the outsider legitimately has jobs against
    // their own.
    let intruder_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scan_job WHERE requested_by = ? AND library_id = ?",
    )
    .bind(outsider.to_string())
    .bind(library.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(intruder_jobs, 0, "a non-member queued a scan");

    // Membership is not authority. A listener reads the catalogue; a scan
    // walks the owner's files and takes the writer gate, so it is refused --
    // and refused the way everything unentitled is, indistinguishably from a
    // library that does not exist.
    let listener = state
        .db
        .create_account("asym-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .add_library_member(owner, library, listener, LibraryRole::Listener, now_ms())
        .await
        .unwrap();
    // The membership is real: the listener sees the library and its tracks.
    assert_eq!(
        state.db.libraries_for_user(listener).await.unwrap().len(),
        1
    );
    assert!(state
        .db
        .create_scan_job_for_user(listener, library, "manual")
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        state.services.start_library_scan(listener, library).await,
        Err(ServiceError::NotFound)
    ));
    // startScan names no library, so it skips the read-only ones instead of
    // failing: an account whose every library is read-only queues nothing and
    // succeeds, exactly like one that reaches no library at all.
    assert!(state
        .services
        .start_visible_scans(listener)
        .await
        .unwrap()
        .is_empty());
    let listener_jobs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM scan_job WHERE requested_by = ?")
            .bind(listener.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(listener_jobs, 0, "a listener queued a scan");
    // Promotion is all it takes, and it is read from the row rather than
    // cached anywhere.
    state
        .db
        .add_library_member(owner, library, listener, LibraryRole::Manager, now_ms())
        .await
        .unwrap();
    assert!(state
        .db
        .create_scan_job_for_user(listener, library, "manual")
        .await
        .unwrap()
        .is_some());
    state
        .db
        .remove_library_member(owner, library, listener, now_ms())
        .await
        .unwrap();

    // An account that can reach no library has nothing to scan. That is an
    // empty result, not an error: there is no missing resource to report, and
    // every other catalogue-wide method answers such an account the same way.
    let stranger = state
        .db
        .create_account("asym-stranger", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    assert!(state
        .services
        .start_visible_scans(stranger)
        .await
        .unwrap()
        .is_empty());

    // Container-only aliases for browse-by-folder clients. The payload is the
    // ID3 one; only the wrapper name differs, as for getAlbumList.
    let search2 = subsonic_json(&router, "search2", api_key, "&query=One&songCount=10").await;
    assert!(search2["subsonic-response"]["searchResult2"]["song"].is_array());
    assert!(search2["subsonic-response"].get("searchResult3").is_none());
    // Nothing is starred yet, so the container is present but carries no list.
    let starred = subsonic_json(&router, "getStarred", api_key, "").await;
    assert!(starred["subsonic-response"]["starred"].is_object());
    assert!(starred["subsonic-response"]["starred"]
        .get("song")
        .is_none());
    assert!(starred["subsonic-response"].get("starred2").is_none());

    // One favorite of each kind, so the renamed container is exercised with
    // content: an alias that only ever answers empty proves nothing about the
    // JSON array rules its new name needs.
    let snapshot = state.services.catalog_snapshot(owner, &[]).await.unwrap();
    for (entity, id) in [
        ("artist", snapshot.artists[0].artist.id),
        ("album", snapshot.albums[0].id),
        ("track", snapshot.songs[0].id),
    ] {
        state
            .services
            .set_star(owner, entity, id, true)
            .await
            .unwrap();
    }
    let starred = subsonic_json(&router, "getStarred", api_key, "").await;
    let starred = &starred["subsonic-response"]["starred"];
    for field in ["artist", "album", "song"] {
        let entries = starred[field]
            .as_array()
            .unwrap_or_else(|| panic!("starred.{field} is not an array: {starred}"));
        assert_eq!(entries.len(), 1, "{field}");
    }
    // The ID3 method answers the same payload under its own container.
    let starred2 = subsonic_json(&router, "getStarred2", api_key, "").await;
    let starred2 = &starred2["subsonic-response"]["starred2"];
    for field in ["artist", "album", "song"] {
        assert_eq!(starred2[field], starred[field], "{field}");
    }

    // `getTopSongs` used to be in the list below, and is not a surface
    // WaveFlow fails to compute any more: it answers an artist's most played
    // tracks from local history, which is asserted in `subsonic_browse`. What
    // belongs here is the shape of the answer when there is nothing to say —
    // still the empty container, and still `ok`, because a name that names no
    // artist is not an error.
    let unknown = subsonic_json(
        &router,
        "getTopSongs",
        api_key,
        "&artist=nobody-in-this-catalogue&count=5",
    )
    .await;
    assert_eq!(unknown["subsonic-response"]["status"], "ok");
    assert_eq!(
        unknown["subsonic-response"]["topSongs"],
        serde_json::json!({})
    );

    // The two similar-songs methods used to answer the empty container for
    // any id at all. Since #267 the id is resolved first and for its refusal
    // (RFC-011 decision 7), so one naming nothing is error 70; what they
    // answer for a real one, enrichment off and on, is in `tests/enrichment.rs`.
    // `getInternetRadioStations` left the empty-container list with #217.
    for method in ["getSimilarSongs", "getSimilarSongs2"] {
        let response = subsonic_json(
            &router,
            method,
            api_key,
            &format!("&id={}&count=5", uuid::Uuid::new_v4()),
        )
        .await;
        assert_eq!(
            response["subsonic-response"]["error"]["code"], 70,
            "{method}"
        );
    }

    // No avatars are stored, so the data is missing rather than the method.
    let avatar = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAvatar.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&username=asym-owner"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(avatar.status(), StatusCode::OK);
    assert_eq!(
        json_body(avatar).await["subsonic-response"]["error"]["code"],
        70
    );

    // A method that really is unimplemented still says so.
    let unknown = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getPodcasts.view?apiKey={api_key}&v=1.16.1&c=golden&f=json"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(unknown).await["subsonic-response"]["error"]["code"],
        0
    );

    // Starting a scan comes last: it runs a real scan over a library root that
    // holds no files, which marks the fabricated tracks unavailable. Every
    // assertion above reads the catalogue and would race it.
    //
    // The response carries the same shape as getScanStatus, so a client that
    // only calls startScan still learns the state.
    let started = subsonic_json(&router, "startScan", api_key, "").await;
    let started = &started["subsonic-response"]["scanStatus"];
    assert!(started["scanning"].is_boolean());
    assert!(started["count"].is_number());
    // The work is real: a job now exists for the owner's library beyond the one
    // the fixture created.
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scan_job WHERE library_id = ?")
        .bind(library.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(queued >= 2, "startScan queued nothing: {queued} jobs");
    let foreign_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scan_job sj JOIN library l ON l.id = sj.library_id \
         WHERE l.name = 'asym-foreign'",
    )
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        foreign_jobs, 1,
        "startScan reached a library the account cannot see"
    );
}

/// Bookmarks are the last Subsonic mutation that had nowhere to write. They go
/// through the domain services like every other piece of user data, which is
/// what puts them in the sync journal and the bootstrap snapshot rather than
/// only in one client's view.
#[tokio::test]
async fn bookmarks_round_trip_sync_and_isolate_tenants() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_bookmark-key";
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("bookmark-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"bookmark-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let outsider = state
        .db
        .create_account("bookmark-outsider", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();

    let seed = |account: Uuid, name: &'static str, offset: usize| {
        let state = state.clone();
        let root = config.data_dir.join(name);
        async move {
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
            state.db.start_scan_job(scan, 1, false).await.unwrap();
            let mut input = browse_input(
                offset,
                "Long Form",
                "Bookmark Album",
                "Bookmark Artist",
                Some(1),
                Some(1),
            );
            input.relative_path = format!("{name}.flac");
            input.quick_hash = format!("{:064x}", offset + 20_000);
            input.full_hash = format!("{:064x}", offset + 21_000);
            state
                .db
                .apply_catalog_track(library, scan, &input, None, false)
                .await
                .unwrap();
            state.db.finish_scan_job(scan, 0).await.unwrap();
            state
                .services
                .catalog_snapshot(account, &[])
                .await
                .unwrap()
                .songs[0]
                .id
        }
    };
    let track = seed(owner, "bookmark-own", 200).await;
    let foreign_track = seed(outsider, "bookmark-foreign", 201).await;

    let router = waveflow_server::app(&config, state.clone());

    // Nothing set yet: the container is present and empty, as it always was.
    let empty = subsonic_json(&router, "getBookmarks", api_key, "").await;
    assert!(empty["subsonic-response"]["bookmarks"].is_object());
    assert!(empty["subsonic-response"]["bookmarks"]
        .get("bookmark")
        .is_none());

    let created = subsonic_json(
        &router,
        "createBookmark",
        api_key,
        &format!("&id={track}&position=125000&comment=where%20I%20stopped"),
    )
    .await;
    assert_eq!(created["subsonic-response"]["status"], "ok");
    // A mutation with no result answers the bare envelope.
    assert!(created["subsonic-response"].get("createBookmark").is_none());

    let listed = subsonic_json(&router, "getBookmarks", api_key, "").await;
    let bookmark = &listed["subsonic-response"]["bookmarks"]["bookmark"][0];
    assert_eq!(bookmark["position"], 125_000);
    assert_eq!(bookmark["username"], "bookmark-owner");
    assert_eq!(bookmark["comment"], "where I stopped");
    assert!(bookmark["created"].is_string());
    assert!(bookmark["changed"].is_string());
    // The entry is a full media item, carrying the position it is bookmarked at.
    assert_eq!(bookmark["entry"]["id"], track.to_string());
    assert_eq!(bookmark["entry"]["title"], "Long Form");
    assert_eq!(bookmark["entry"]["bookmarkPosition"], 125_000);
    // It goes through the shared projection, so the modern fields are there too.
    assert_eq!(bookmark["entry"]["samplingRate"], 44_100);
    assert!(bookmark["entry"]["artists"].is_array());

    // A bookmark answers "where did I stop in this file", so setting it again
    // moves it rather than adding a second one.
    subsonic_json(
        &router,
        "createBookmark",
        api_key,
        &format!("&id={track}&position=250000"),
    )
    .await;
    let moved = subsonic_json(&router, "getBookmarks", api_key, "").await;
    let moved = &moved["subsonic-response"]["bookmarks"]["bookmark"];
    assert_eq!(moved.as_array().unwrap().len(), 1);
    assert_eq!(moved[0]["position"], 250_000);
    // Omitting the comment clears it rather than keeping the old one.
    assert!(moved[0].get("comment").is_none());

    // It reaches the sync surfaces because it is a domain mutation, not a
    // facade-local one: a desktop client sees it without a second contract.
    let snapshot = state.services.sync_snapshot(owner, 50).await.unwrap();
    assert_eq!(snapshot.bookmarks.len(), 1);
    assert_eq!(snapshot.bookmarks[0].position_ms, 250_000);
    assert_eq!(snapshot.bookmarks[0].song.id, track);
    let changes = state.sync.changes(owner, 0, 100).await.unwrap();
    assert!(
        changes
            .changes
            .iter()
            .any(|change| change.entity_type == "bookmark" && change.entity_id == track),
        "no bookmark event in the journal: {:?}",
        changes
            .changes
            .iter()
            .map(|change| change.entity_type.clone())
            .collect::<Vec<_>>()
    );

    // And the native bootstrap carries them, so a desktop client that only
    // ever calls /sync/snapshot receives bookmarks without a second contract.
    let token = login_token(&router, "bookmark-owner", "correct horse battery staple").await;
    let native = router
        .clone()
        .oneshot(
            Request::get("/api/v2/sync/snapshot")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native.status(), StatusCode::OK);
    let native = json_body(native).await;
    assert_eq!(native["bookmarks"].as_array().unwrap().len(), 1);
    assert_eq!(native["bookmarks"][0]["position_ms"], 250_000);
    assert_eq!(native["bookmarks"][0]["song"]["id"], track.to_string());

    // A position before the start of the file is not a position. The service
    // refuses it, and the facade reports the parameter error rather than an
    // internal one.
    assert!(matches!(
        state.services.set_bookmark(owner, track, -1, None).await,
        Err(ServiceError::Invalid)
    ));
    let negative = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/createBookmark.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={track}&position=-1"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(negative).await["subsonic-response"]["error"]["code"],
        10
    );

    // XML is the default encoding and nests the entry as an element rather
    // than as a JSON object, so it gets its own assertion.
    let xml = router
        .clone()
        .oneshot(
            Request::get(
                "/rest/getBookmarks.view?u=bookmark-owner&p=bookmark-secret&v=1.16.1&c=golden",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(xml.status(), StatusCode::OK);
    let xml = body_text(xml).await;
    assert!(xml.contains("<bookmark "));
    assert!(xml.contains("username=\"bookmark-owner\""));
    assert!(xml.contains("position=\"250000\""));
    assert!(xml.contains("<entry "));
    assert!(xml.contains("bookmarkPosition=\"250000\""));

    // Another account's track is not bookmarkable, and the refusal does not
    // confirm that the track exists.
    let foreign = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/createBookmark.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={foreign_track}&position=1000"
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
    let unknown = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/createBookmark.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={}&position=1000",
                Uuid::nil()
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(unknown).await["subsonic-response"]["error"]["code"],
        70
    );
    assert!(state.services.bookmarks(outsider).await.unwrap().is_empty());

    subsonic_json(&router, "deleteBookmark", api_key, &format!("&id={track}")).await;
    assert!(state.services.bookmarks(owner).await.unwrap().is_empty());
    // Deleting one that is not there is not an error: the caller asked for the
    // track to carry no bookmark, and it does not.
    subsonic_json(&router, "deleteBookmark", api_key, &format!("&id={track}")).await;
}

/// The last empty container in the facade, filled.
///
/// `getInternetRadioStations` answered `{}` from the beginning, and it is the
/// one absence here a client cannot read as an absence: every other missing
/// feature declares a role `false`, while "zero stations" is also what a server
/// without radio says. This asserts the four methods, the two deliberate
/// deviations from the reference server — a per-account list, and the
/// specification's own `homepageUrl` in / `homePageUrl` out asymmetry — and the
/// shape of a container holding exactly one child, which is the only length
/// that can tell a declared array from an object.
#[tokio::test]
async fn internet_radio_stations_are_kept_per_account_and_handed_out_never_fetched() {
    let (_temp, config, state) = test_app().await;
    // Inserted rather than created, and every credential drawn at random: a
    // string literal flowing into a cryptographic call is what
    // `rust/hard-coded-cryptographic-value` reports, whatever the call is for.
    // These accounts never present a password, so nothing is hashed.
    let owner = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    for (id, username) in [(owner, "radio-owner"), (outsider, "radio-outsider")] {
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
    let key_of = |who: Uuid| {
        let state = state.clone();
        async move {
            let api_key = format!("wfsk_{}", Uuid::new_v4());
            let secret = Uuid::new_v4().to_string();
            state
                .db
                .set_subsonic_credential(
                    who,
                    who,
                    &state.secret_box.encrypt(secret.as_bytes()).unwrap(),
                    &security::token_hash(&api_key),
                    now_ms(),
                )
                .await
                .unwrap();
            api_key
        }
    };
    let api_key = key_of(owner).await;
    let other_key = key_of(outsider).await;

    let router = waveflow_server::app(&config, state.clone());

    // Still the empty container before anything is stored — the answer this
    // method used to give always.
    let empty = subsonic_json(&router, "getInternetRadioStations", &api_key, "").await;
    assert!(empty["subsonic-response"]["internetRadioStations"].is_object());
    assert!(omits(
        &empty["subsonic-response"]["internetRadioStations"],
        "internetRadioStation"
    ));

    let created = subsonic_json(
        &router,
        "createInternetRadioStation",
        &api_key,
        "&name=Zed%20Radio&streamUrl=https://radio.example.org/zed.mp3&homepageUrl=https://radio.example.org/zed",
    )
    .await;
    assert_eq!(created["subsonic-response"]["status"], "ok");
    // A write the specification documents as answering nothing answers nothing.
    assert!(omits(
        &created["subsonic-response"],
        "createInternetRadioStation"
    ));

    // One child, which is the length that distinguishes a declared array from
    // an object. Reading it through `as_array().unwrap_or_default()` would
    // yield `[]` here and fail further down, saying nothing about the shape.
    let one = subsonic_json(&router, "getInternetRadioStations", &api_key, "").await;
    let one = &one["subsonic-response"]["internetRadioStations"];
    assert!(
        one["internetRadioStation"].is_array(),
        "internetRadioStations.internetRadioStation is a list: {one}"
    );
    let station = &one["internetRadioStation"][0];
    assert_eq!(station["name"], "Zed Radio");
    // Handed back exactly as typed. The server stores an address and never
    // fetches one, which is why this feature needs no allowlist.
    assert_eq!(station["streamUrl"], "https://radio.example.org/zed.mp3");
    // The specification's spelling: `homepageUrl` in, `homePageUrl` out.
    assert_eq!(station["homePageUrl"], "https://radio.example.org/zed");
    let station_id = station["id"].as_str().unwrap().to_owned();

    // XML carries the same asymmetry, and it is the surface most clients read.
    let xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getInternetRadioStations.view?apiKey={api_key}&v=1.16.1&c=golden"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(xml.status(), StatusCode::OK);
    let xml = body_text(xml).await;
    assert!(
        xml.contains("<internetRadioStation ") && xml.contains("homePageUrl="),
        "{xml}"
    );

    // A second station, with no homepage, and a lowercase name against the
    // first's capital: under SQLite's default BINARY collation `Z` (90) sorts
    // before `a` (97), so this pair comes back in the other order the moment
    // the `COLLATE NOCASE` leaves the query. Two names of the same case would
    // have asserted nothing about the collation.
    subsonic_json(
        &router,
        "createInternetRadioStation",
        &api_key,
        // `homepageUrl=` and not an omitted parameter: a form with the box
        // left blank sends the name with no value, and that means the station
        // has no page rather than one spelled with zero characters. Refusing
        // it would make clearing the field an error (constat de revue, pris).
        "&name=alpha%20radio&streamUrl=http://alpha.example.org/stream&homepageUrl=",
    )
    .await;
    let two = subsonic_json(&router, "getInternetRadioStations", &api_key, "").await;
    let two = two["subsonic-response"]["internetRadioStations"]["internetRadioStation"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(two.len(), 2);
    assert_eq!(two[0]["name"], "alpha radio");
    assert_eq!(two[1]["name"], "Zed Radio");
    // Absent, not null: that station gave no homepage, and it said so with an
    // empty value rather than by omitting the parameter.
    assert!(omits(&two[0], "homePageUrl"));

    // The update carries every field, so omitting the homepage clears it. That
    // is the protocol's shape, and the reason the native route is a `PUT`.
    subsonic_json(
        &router,
        "updateInternetRadioStation",
        &api_key,
        &format!(
            "&id={station_id}&name=Zed%20Radio%20Deux&streamUrl=https://radio.example.org/zed2.mp3"
        ),
    )
    .await;
    let updated = subsonic_json(&router, "getInternetRadioStations", &api_key, "").await;
    let updated = &updated["subsonic-response"]["internetRadioStations"]["internetRadioStation"];
    let zed = updated
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == station_id.as_str())
        .unwrap();
    assert_eq!(zed["name"], "Zed Radio Deux");
    assert_eq!(zed["streamUrl"], "https://radio.example.org/zed2.mp3");
    assert!(omits(zed, "homePageUrl"));

    // Per account, which is the deviation from the reference server: the other
    // account has its own list, and it is empty.
    let foreign_list = subsonic_json(&router, "getInternetRadioStations", &other_key, "").await;
    assert!(omits(
        &foreign_list["subsonic-response"]["internetRadioStations"],
        "internetRadioStation"
    ));
    // And it cannot reach the owner's station by id. Missing and forbidden
    // answer identically.
    for method in ["updateInternetRadioStation", "deleteInternetRadioStation"] {
        let refused = subsonic_json(
            &router,
            method,
            &other_key,
            &format!("&id={station_id}&name=Stolen&streamUrl=https://x.example.org/s"),
        )
        .await;
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 70,
            "{method}"
        );
    }
    let absent = subsonic_json(
        &router,
        "deleteInternetRadioStation",
        &api_key,
        &format!("&id={}", Uuid::nil()),
    )
    .await;
    assert_eq!(absent["subsonic-response"]["error"]["code"], 70);

    // An address the server would have had to dereference to learn anything
    // about is refused. None of these needs escaping: a query string reads
    // `:`, `/`, `;` and `,` as data.
    //
    // `mms://` is the one that proves the scheme restriction: `file:` and
    // `data:` are refused for naming no host, so they would still be refused
    // with any scheme list at all. Only an authority-carrying scheme tells the
    // two rules apart.
    for address in [
        "mms://radio.example.org/stream",
        "file:///etc/passwd",
        "data:audio/mpeg;base64,AAAA",
        "/relative/stream.mp3",
    ] {
        let refused = subsonic_json(
            &router,
            "createInternetRadioStation",
            &api_key,
            &format!("&name=Elsewhere&streamUrl={address}"),
        )
        .await;
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 10,
            "{address}"
        );
    }
    // A blank name is refused too: it would list as a row nothing tells apart.
    let blank = subsonic_json(
        &router,
        "createInternetRadioStation",
        &api_key,
        "&name=%20%20&streamUrl=https://radio.example.org/blank",
    )
    .await;
    assert_eq!(blank["subsonic-response"]["error"]["code"], 10);

    // A required parameter missing is error 10, not a station created with a
    // default nobody asked for.
    for (method, extra) in [
        ("createInternetRadioStation", "&name=No%20Address"),
        (
            "createInternetRadioStation",
            "&streamUrl=https://radio.example.org/anonymous",
        ),
        ("deleteInternetRadioStation", ""),
    ] {
        let incomplete = subsonic_json(&router, method, &api_key, extra).await;
        assert_eq!(
            incomplete["subsonic-response"]["error"]["code"], 10,
            "{method}{extra}"
        );
    }

    // Nothing refused above created anything: two stations, both the owner's.
    assert_eq!(state.services.radio_stations(owner).await.unwrap().len(), 2);
    assert!(state
        .services
        .radio_stations(outsider)
        .await
        .unwrap()
        .is_empty());

    subsonic_json(
        &router,
        "deleteInternetRadioStation",
        &api_key,
        &format!("&id={station_id}"),
    )
    .await;
    assert_eq!(state.services.radio_stations(owner).await.unwrap().len(), 1);
}

/// An account with a Subsonic API key drawn at run time, so no credential is
/// ever written into the source.
async fn subsonic_account(state: &waveflow_server::AppState, username: &str) -> (Uuid, String) {
    let hash = security::hash_password(&Uuid::new_v4().to_string()).unwrap();
    let account = state
        .db
        .create_account(username, &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let api_key = format!("wfsk_{}", Uuid::new_v4().simple());
    state
        .db
        .set_subsonic_credential(
            account,
            account,
            &state
                .secret_box
                .encrypt(Uuid::new_v4().to_string().as_bytes())
                .unwrap(),
            &security::token_hash(&api_key),
            now_ms(),
        )
        .await
        .unwrap();
    (account, api_key)
}

fn entry_ids(playlist: &serde_json::Value) -> Vec<String> {
    playlist["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| entry["id"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// `createPlaylist` and `updatePlaylist` take a track more than once, and
/// `songIndexToRemove` names a position in the list the client was shown
/// (RFC-020 decision 1).
///
/// The second half is what makes the first usable. With duplicates, a
/// position is the only way to say which copy goes — and the positions a
/// client sees skip every track it can no longer read, while the stored list
/// still holds them. Applying the index to the stored list removed the wrong
/// entry, and the one the listener pointed at stayed.
#[tokio::test]
async fn subsonic_playlists_take_duplicates_and_remove_by_the_position_shown() {
    let (_temp, config, state) = test_app().await;
    let (owner, api_key) = subsonic_account(&state, "dup-subsonic").await;
    let titles = ["Gone Soon", "Again", "Beside"];
    let (library, ids) = seed_titles(&state, &config, owner, "dup-sub", 4_000, &titles).await;
    let (gone, a, b) = (ids[0], ids[1], ids[2]);
    let router = waveflow_server::app(&config, state.clone());

    let created = subsonic_json(
        &router,
        "createPlaylist",
        &api_key,
        &format!("&name=Twice&songId={a}&songId={a}"),
    )
    .await;
    let created = &created["subsonic-response"]["playlist"];
    assert_eq!(created["songCount"], 2, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();

    let updated = subsonic_json(
        &router,
        "updatePlaylist",
        &api_key,
        &format!("&playlistId={id}&songIdToAdd={b}&songIdToAdd={a}"),
    )
    .await;
    assert_eq!(updated["subsonic-response"]["status"], "ok", "{updated}");
    let read = subsonic_json(&router, "getPlaylist", &api_key, &format!("&id={id}")).await;
    assert_eq!(
        entry_ids(&read["subsonic-response"]["playlist"]),
        vec![a.to_string(), a.to_string(), b.to_string(), a.to_string()]
    );

    subsonic_json(
        &router,
        "updatePlaylist",
        &api_key,
        &format!("&playlistId={id}&songIndexToRemove=1"),
    )
    .await;
    let read = subsonic_json(&router, "getPlaylist", &api_key, &format!("&id={id}")).await;
    assert_eq!(
        entry_ids(&read["subsonic-response"]["playlist"]),
        vec![a.to_string(), b.to_string(), a.to_string()]
    );

    // A playlist whose first track then goes missing from disk: the client
    // is shown two entries and removes the first of those.
    let hidden_first = subsonic_json(
        &router,
        "createPlaylist",
        &api_key,
        &format!("&name=Hidden%20first&songId={gone}&songId={a}&songId={b}"),
    )
    .await;
    let hidden_first = hidden_first["subsonic-response"]["playlist"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let rescan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(rescan, 2, false).await.unwrap();
    // The rescan sees the other two files and not the first.
    for track in [a, b] {
        state.db.mark_track_seen(track, rescan).await.unwrap();
    }
    assert_eq!(
        state
            .db
            .mark_unseen_unavailable(library, rescan)
            .await
            .unwrap(),
        1
    );
    state.db.finish_scan_job(rescan, 0).await.unwrap();
    let shown = subsonic_json(
        &router,
        "getPlaylist",
        &api_key,
        &format!("&id={hidden_first}"),
    )
    .await;
    assert_eq!(
        entry_ids(&shown["subsonic-response"]["playlist"]),
        vec![a.to_string(), b.to_string()]
    );
    subsonic_json(
        &router,
        "updatePlaylist",
        &api_key,
        &format!("&playlistId={hidden_first}&songIndexToRemove=0"),
    )
    .await;
    let shown = subsonic_json(
        &router,
        "getPlaylist",
        &api_key,
        &format!("&id={hidden_first}"),
    )
    .await;
    assert_eq!(
        entry_ids(&shown["subsonic-response"]["playlist"]),
        vec![b.to_string()],
        "the entry the client pointed at is the one that goes"
    );
    // And the track it could not see is still held, for when it comes back.
    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT track_id FROM playlist_track WHERE playlist_id=? ORDER BY position",
    )
    .bind(&hidden_first)
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(stored, vec![gone.to_string(), b.to_string()]);
}

/// `getPlaylists` and `getPlaylist` answer the public playlists of other
/// accounts, with `owner` and `public` saying whose they are, and every write
/// to one answers error 70 for anyone but its owner (RFC-020 decision 2).
#[tokio::test]
async fn subsonic_lists_public_playlists_of_other_accounts_under_their_owner() {
    let (_temp, config, state) = test_app().await;
    let (owner, owner_key) = subsonic_account(&state, "pub-owner").await;
    let (viewer, viewer_key) = subsonic_account(&state, "pub-viewer").await;
    let (shared, open) = seed_titles(
        &state,
        &config,
        owner,
        "pub-shared",
        4_100,
        &["Heard One", "Heard Two"],
    )
    .await;
    let (_private, kept) =
        seed_titles(&state, &config, owner, "pub-private", 4_200, &["Unheard"]).await;
    state
        .db
        .add_library_member(owner, shared, viewer, LibraryRole::Listener, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let created = subsonic_json(
        &router,
        "createPlaylist",
        &owner_key,
        &format!(
            "&name=Open&songId={}&songId={}&songId={}",
            open[0], kept[0], open[1]
        ),
    )
    .await;
    let id = created["subsonic-response"]["playlist"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let listed = subsonic_json(&router, "getPlaylists", &viewer_key, "").await;
    assert!(
        listed["subsonic-response"]["playlists"]["playlist"]
            .as_array()
            .is_none_or(|playlists| playlists.is_empty()),
        "a private playlist is not listed to another account: {listed}"
    );

    subsonic_json(
        &router,
        "updatePlaylist",
        &owner_key,
        &format!("&playlistId={id}&public=true"),
    )
    .await;

    let listed = subsonic_json(&router, "getPlaylists", &viewer_key, "").await;
    let listed = listed["subsonic-response"]["playlists"]["playlist"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], id.as_str());
    assert_eq!(listed[0]["owner"], "pub-owner");
    assert_eq!(listed[0]["public"], true);
    // Counted over what the viewer can play, and nothing else.
    assert_eq!(listed[0]["songCount"], 2);
    assert_eq!(listed[0]["duration"], 240);

    let read = subsonic_json(&router, "getPlaylist", &viewer_key, &format!("&id={id}")).await;
    let read = &read["subsonic-response"]["playlist"];
    assert_eq!(read["owner"], "pub-owner");
    assert_eq!(
        entry_ids(read),
        vec![open[0].to_string(), open[1].to_string()]
    );
    assert!(!read.to_string().contains(&kept[0].to_string()));

    // The owner's own list says the playlist is theirs.
    let own = subsonic_json(&router, "getPlaylists", &owner_key, "").await;
    assert_eq!(
        own["subsonic-response"]["playlists"]["playlist"][0]["owner"],
        "pub-owner"
    );

    for (method, extra) in [
        ("updatePlaylist", format!("&playlistId={id}&name=Taken")),
        (
            "updatePlaylist",
            format!("&playlistId={id}&songIdToAdd={}", open[0]),
        ),
        (
            "createPlaylist",
            format!("&playlistId={id}&songId={}", open[0]),
        ),
        ("deletePlaylist", format!("&id={id}")),
    ] {
        let refused = subsonic_json(&router, method, &viewer_key, &extra).await;
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 70,
            "{method} by a non-owner: {refused}"
        );
    }
    let intact = subsonic_json(&router, "getPlaylist", &owner_key, &format!("&id={id}")).await;
    let intact = &intact["subsonic-response"]["playlist"];
    assert_eq!(intact["name"], "Open");
    assert_eq!(intact["songCount"], 3);

    subsonic_json(
        &router,
        "updatePlaylist",
        &owner_key,
        &format!("&playlistId={id}&public=false"),
    )
    .await;
    let withdrawn = subsonic_json(&router, "getPlaylist", &viewer_key, &format!("&id={id}")).await;
    assert_eq!(withdrawn["subsonic-response"]["error"]["code"], 70);
}
