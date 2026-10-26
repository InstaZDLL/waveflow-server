//! Stream tickets for browser playback, and what a disabled account stops
//! playing.

use super::*;

#[tokio::test]
async fn stream_tickets_authorise_browser_playback_without_a_bearer() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("ticket-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("ticket-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("ticket-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Ticket.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Tickets",
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
            id: library_id,
            name: "Tickets".into(),
            root_path: root,
        },
    )
    .await;

    // Kept before `state` moves into the router, to mint an expired ticket below.
    let secret_box = std::sync::Arc::clone(&state.secret_box);
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "ticket-owner", password).await;
    let intruder_token = login_token(&router, "ticket-intruder", password).await;

    let tracks = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/libraries/{library_id}/tracks"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let tracks = json_body(tracks).await;
    let track_id = tracks[0]["id"].as_str().unwrap().to_owned();

    let mint = |token: Option<String>| {
        let router = router.clone();
        let track_id = track_id.clone();
        async move {
            let mut request = Request::post(format!("/api/v2/tracks/{track_id}/stream-ticket"));
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            router
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };

    // Minting requires a bearer; redeeming must not.
    assert_eq!(mint(None).await.status(), StatusCode::UNAUTHORIZED);

    let issued = mint(Some(owner_token.clone())).await;
    assert_eq!(issued.status(), StatusCode::OK);
    let issued = json_body(issued).await;
    let url = issued["url"].as_str().unwrap().to_owned();
    assert!(url.starts_with("/api/v2/stream/"));
    assert!(issued["expires_at"].as_i64().unwrap() > now_ms());

    // The ticket URL stays relative even when a public URL is configured, and
    // that asymmetry with createShare is deliberate: a share link is made to
    // leave the application, a ticket is not. An absolute ticket would let the
    // server point playback at a host the user never authenticated against, so
    // clients are right to reject absolute or protocol-relative values — this
    // pins the guarantee they rely on.
    let mut public_config = config.clone();
    public_config.public_url = Some("https://waveflow.example".to_owned());
    let public_router = waveflow_server::app(&public_config, state.clone());
    let public_ticket = public_router
        .oneshot(
            Request::post(format!("/api/v2/tracks/{track_id}/stream-ticket"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_ticket.status(), StatusCode::OK);
    let public_url = json_body(public_ticket).await["url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        public_url.starts_with("/api/v2/stream/"),
        "ticket URL must stay relative, got {public_url}"
    );

    // The ticket URL plays with no Authorization header at all.
    let played = router
        .clone()
        .oneshot(Request::get(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(played.status(), StatusCode::OK);
    assert_eq!(played.headers()["accept-ranges"], "bytes");

    // Range requests work, which is what a browser seek relies on.
    let ranged = router
        .clone()
        .oneshot(
            Request::get(&url)
                .header("range", "bytes=0-15")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);

    // Playing sends no attachment header; that is a download's alone.
    assert!(played.headers().get("content-disposition").is_none());
    let played_length = played.headers()["content-length"].clone();

    // `?download=1` is the same file, saved under the file's own name
    // (RFC-012 decision 5) — no other route, no other read.
    let basename = tracks[0]["relative_path"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned();
    let downloaded = router
        .clone()
        .oneshot(
            Request::get(format!("{url}?download=1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(downloaded.status(), StatusCode::OK);
    assert_eq!(downloaded.headers()["content-length"], played_length);
    let disposition = downloaded.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        disposition.starts_with("attachment; filename=\"")
            && disposition.contains(&format!("filename*=UTF-8''{basename}")),
        "{disposition}"
    );

    // A download is the file: asked for with a transcode, an offset or a bit
    // rate, it is refused rather than handed over under the file's name.
    for refused in [
        format!("{url}?download=1&format=mp3"),
        format!("{url}?download=1&offset_ms=1000"),
        format!("{url}?download=1&bitrate=128"),
    ] {
        let response = router
            .clone()
            .oneshot(Request::get(&refused).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{refused}"
        );
    }

    // An expired ticket is refused even though it is otherwise well formed.
    let expired = waveflow_server::stream_ticket::mint(
        &secret_box,
        waveflow_server::stream_ticket::TicketKind::Audio,
        owner,
        uuid::Uuid::parse_str(&track_id).unwrap(),
        now_ms() - 1,
    )
    .unwrap();
    let expired = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/stream/{expired}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::NOT_FOUND);

    // A ticket of another kind does not open audio, even minted for this very
    // track by this very account under the instance key, and unexpired. The
    // kind is the only thing separating it from the ticket that works two
    // assertions above — which is the whole reason the byte exists.
    let wrong_kind = waveflow_server::stream_ticket::mint(
        &secret_box,
        waveflow_server::stream_ticket::TicketKind::Canvas,
        owner,
        uuid::Uuid::parse_str(&track_id).unwrap(),
        now_ms() + 60_000,
    )
    .unwrap();
    let wrong_kind = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/stream/{wrong_kind}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_kind.status(), StatusCode::NOT_FOUND);

    // A tampered ticket is indistinguishable from an unknown track. The flipped
    // character sits mid-ticket: trailing base64url characters carry spare bits,
    // so changing one there can decode to the same bytes.
    let prefix_len = "/api/v2/stream/".len();
    let mut tampered: Vec<char> = url.chars().collect();
    let middle = prefix_len + (tampered.len() - prefix_len) / 2;
    tampered[middle] = if tampered[middle] == 'A' { 'B' } else { 'A' };
    let tampered: String = tampered.into_iter().collect();
    let forged = router
        .clone()
        .oneshot(Request::get(&tampered).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::NOT_FOUND);

    // A tenant without access cannot mint a ticket in the first place.
    assert_eq!(
        mint(Some(intruder_token)).await.status(),
        StatusCode::NOT_FOUND
    );
}

/// RFC-023 decision 4: a disabled account plays nothing it handed out — not a
/// stream ticket it minted, and not a share it created, by its page, its JSON
/// or its stream.
///
/// The bearer routes already refused a disabled account; these did not. A
/// ticket is sealed rather than looked up and lives an hour, and a share is
/// opened by nobody at all, so the account was never asked. Each now answers
/// 404, exactly as for a ticket or a share that does not exist, and opening the
/// share counts no visit. Re-enabling the account at the end proves the cause.
#[tokio::test]
async fn a_disabled_account_plays_nothing_it_handed_out() {
    let (_temp, config, state) = test_app().await;
    let (owner, api_token) = admin_with_api_token(&state, "disabled-sharer").await;
    let music = config.data_dir.join("disabled-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Handed.wav"));
    // A cover, so the share's own cover route (RFC-020) is held to the same
    // rule as its stream.
    write_test_png(&music.join("cover.png"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Handed out",
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
            id: library_id,
            name: "Handed out".into(),
            root_path: root,
        },
    )
    .await;
    let track: String = sqlx::query_scalar("SELECT id FROM track WHERE library_id=?")
        .bind(library_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    let track_id: uuid::Uuid = track.parse().unwrap();
    let share = state
        .services
        .create_share(owner, &[track_id], Some("Handed out"), None)
        .await
        .unwrap();
    let share_url = format!("/share/{}", share.url_token.clone().unwrap());
    let router = waveflow_server::app(&config, state.clone());
    let get = |uri: String, accept: Option<&'static str>| {
        let router = router.clone();
        async move {
            let mut request = Request::get(uri);
            if let Some(accept) = accept {
                request = request.header("accept", accept);
            }
            router
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };
    let navigation = Some("text/html,application/xhtml+xml,*/*;q=0.8");
    let visits = || async { state.services.shares(owner).await.unwrap()[0].visit_count };

    // Before: the ticket plays, and the share opens and plays.
    let minted = send_json(
        &router,
        "POST",
        &format!("/api/v2/tracks/{track_id}/stream-ticket"),
        &api_token,
        None,
    )
    .await;
    assert_eq!(minted.status(), StatusCode::OK);
    let ticket_url = json_body(minted).await["url"].as_str().unwrap().to_owned();
    assert_eq!(get(ticket_url.clone(), None).await.status(), StatusCode::OK);
    assert_eq!(
        get(share_url.clone(), navigation).await.status(),
        StatusCode::OK
    );
    let opened = get(share_url.clone(), None).await;
    assert_eq!(opened.status(), StatusCode::OK);
    let opened = json_body(opened).await;
    let stream_url = opened["tracks"][0]["streamUrl"]
        .as_str()
        .unwrap()
        .to_owned();
    let cover_url = opened["tracks"][0]["coverUrl"]
        .as_str()
        .expect("the track has a cover")
        .to_owned();
    assert_eq!(get(stream_url.clone(), None).await.status(), StatusCode::OK);
    assert_eq!(get(cover_url.clone(), None).await.status(), StatusCode::OK);
    assert_eq!(visits().await, 1);

    set_disabled(&state, owner, true).await;
    assert_eq!(
        get(ticket_url.clone(), None).await.status(),
        StatusCode::NOT_FOUND,
        "a ticket minted before the account was disabled must stop playing"
    );
    assert_eq!(
        get(share_url.clone(), navigation).await.status(),
        StatusCode::NOT_FOUND,
        "the share page"
    );
    assert_eq!(
        get(share_url.clone(), None).await.status(),
        StatusCode::NOT_FOUND,
        "the share document"
    );
    assert_eq!(
        get(stream_url.clone(), None).await.status(),
        StatusCode::NOT_FOUND,
        "a pass handed out before the account was disabled"
    );
    assert_eq!(
        get(cover_url.clone(), None).await.status(),
        StatusCode::NOT_FOUND,
        "the share's covers, under the same pass"
    );
    assert_eq!(visits().await, 1, "a refused opening is not a visit");

    set_disabled(&state, owner, false).await;
    assert_eq!(get(ticket_url, None).await.status(), StatusCode::OK);
    assert_eq!(get(share_url, None).await.status(), StatusCode::OK);
    assert_eq!(get(stream_url, None).await.status(), StatusCode::OK);
    assert_eq!(get(cover_url, None).await.status(), StatusCode::OK);
}
