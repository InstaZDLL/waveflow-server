//! Public shares: visits, caps, transcodes and covers seen by an anonymous
//! visitor.

use super::*;

/// One real file, scanned, and a share of it created through the native API.
///
/// The temporary directory comes back with the rest because the library lives
/// in it: dropping it would delete the file the share streams.
struct SharedFile {
    _temp: tempfile::TempDir,
    router: axum::Router,
    token: String,
    track: uuid::Uuid,
    share_id: String,
    share_url: String,
}

async fn shared_file(name: &str, share: serde_json::Value) -> SharedFile {
    shared_file_with(name, share, false).await
}

/// `shared_file`, with a cover beside the track when `cover` says so.
async fn shared_file_with(name: &str, share: serde_json::Value, cover: bool) -> SharedFile {
    let (temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account(name, &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join(format!("{name}-music"));
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Shared.wav"));
    if cover {
        write_test_png(&music.join("cover.png"));
    }
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(owner, name, &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library,
            name: name.into(),
            root_path: root,
        },
    )
    .await;
    let track = state
        .db
        .list_tracks_for_user(owner, library)
        .await
        .unwrap()
        .pop()
        .unwrap()
        .id;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, name, &password).await;
    let mut body = share;
    body["track_ids"] = serde_json::json!([track]);
    let created = send_json(&router, "POST", "/api/v2/shares", &token, Some(body)).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    SharedFile {
        _temp: temp,
        router,
        token,
        track,
        share_id: created["id"].as_str().unwrap().to_owned(),
        share_url: created["url"].as_str().unwrap().to_owned(),
    }
}

async fn anonymous(
    router: &axum::Router,
    uri: &str,
    range: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::get(uri);
    if let Some(range) = range {
        request = request.header("range", range);
    }
    router
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Opens the share as its JSON consumers do, and answers the body.
async fn open_share(router: &axum::Router, url: &str) -> serde_json::Value {
    let opened = anonymous(router, url, None).await;
    assert_eq!(opened.status(), StatusCode::OK);
    json_body(opened).await
}

fn with_query(url: &str, pair: &str) -> String {
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}{pair}")
}

async fn listed_share(shared: &SharedFile) -> serde_json::Value {
    let listed =
        json_body(send_json(&shared.router, "GET", "/api/v2/shares", &shared.token, None).await)
            .await;
    listed
        .as_array()
        .unwrap()
        .iter()
        .find(|share| share["id"] == shared.share_id.as_str())
        .unwrap()
        .clone()
}

/// A visit is an opening of the share, not a request for its bytes (RFC-020
/// decision 4).
///
/// Every stream request used to count one, under the process-wide writer gate:
/// a single listen with a few seeks — each one a range request — climbed the
/// counter by as many, and every one of them queued behind a scan.
#[tokio::test]
async fn a_share_counts_one_visit_per_opening_and_none_per_stream_request() {
    let shared = shared_file("visit-owner", serde_json::json!({})).await;
    let opened = open_share(&shared.router, &shared.share_url).await;
    assert_eq!(opened["visitCount"], 1);
    let stream = opened["tracks"][0]["streamUrl"]
        .as_str()
        .unwrap()
        .to_owned();
    for range in ["bytes=0-3", "bytes=4-7", "bytes=0-"] {
        let played = anonymous(&shared.router, &stream, Some(range)).await;
        assert_eq!(played.status(), StatusCode::PARTIAL_CONTENT, "{range}");
        // The stream is a share response like the others (RFC-019).
        assert_eq!(played.headers()["referrer-policy"], "no-referrer");
        assert_eq!(played.headers()["x-frame-options"], "DENY");
        assert_eq!(
            played.headers()["content-security-policy"],
            "frame-ancestors 'none'"
        );
    }
    assert_eq!(listed_share(&shared).await["visit_count"], 1);

    open_share(&shared.router, &shared.share_url).await;
    assert_eq!(listed_share(&shared).await["visit_count"], 2);

    // The stream address is the one the opening handed out. Built by hand,
    // without the visit it belongs to, it opens nothing.
    let bare = format!("{}/tracks/{}/stream", shared.share_url, shared.track);
    assert_eq!(
        anonymous(&shared.router, &bare, None).await.status(),
        StatusCode::NOT_FOUND
    );
}

/// An anonymous visitor plays the original or one fixed MP3, and chooses
/// nothing else (RFC-020 decision 5).
///
/// The stream route took the whole stream query, so anyone holding the link
/// could ask for any format at any bitrate, or for a live encode from any
/// offset, each one an FFmpeg process run on the owner's account.
#[tokio::test]
async fn an_anonymous_share_visitor_cannot_choose_a_transcode() {
    let shared = shared_file("transcode-owner", serde_json::json!({})).await;
    let opened = open_share(&shared.router, &shared.share_url).await;
    let stream = opened["tracks"][0]["streamUrl"]
        .as_str()
        .unwrap()
        .to_owned();
    for refused in [
        "format=opus",
        "format=mp3&bitrate=96",
        "format=mp3&offset_ms=50",
        "bitrate=320",
    ] {
        assert_eq!(
            anonymous(&shared.router, &with_query(&stream, refused), None)
                .await
                .status(),
            StatusCode::BAD_REQUEST,
            "{refused}"
        );
    }
    let mp3 = anonymous(&shared.router, &with_query(&stream, "format=mp3"), None).await;
    assert_eq!(mp3.status(), StatusCode::OK);
    assert_eq!(mp3.headers()["content-type"], "audio/mpeg");
    let original = anonymous(&shared.router, &stream, None).await;
    assert_eq!(original.status(), StatusCode::OK);
    let bytes = original.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..4], b"RIFF");
}

/// A share may be opened a set number of times, and the opening that reached
/// the cap still plays (RFC-020 decision 6).
#[tokio::test]
async fn a_share_stops_opening_at_its_visit_cap_and_its_last_visit_still_plays() {
    let shared = shared_file("cap-owner", serde_json::json!({ "max_visits": 2 })).await;
    assert_eq!(listed_share(&shared).await["max_visits"], 2);
    open_share(&shared.router, &shared.share_url).await;
    let last = open_share(&shared.router, &shared.share_url).await;
    let stream = last["tracks"][0]["streamUrl"].as_str().unwrap().to_owned();
    assert_eq!(
        anonymous(&shared.router, &shared.share_url, None)
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "the third opening of a share capped at two"
    );
    assert_eq!(listed_share(&shared).await["visit_count"], 2);
    assert_eq!(
        anonymous(&shared.router, &stream, Some("bytes=0-3"))
            .await
            .status(),
        StatusCode::PARTIAL_CONTENT,
        "the visit that reached the cap keeps its music"
    );

    // A cap is a positive number, and lifting it is a named clear.
    let uri = format!("/api/v2/shares/{}", shared.share_id);
    for invalid in [0, -3] {
        assert_eq!(
            send_json(
                &shared.router,
                "PATCH",
                &uri,
                &shared.token,
                Some(serde_json::json!({ "max_visits": invalid })),
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{invalid}"
        );
    }
    let lifted = send_json(
        &shared.router,
        "PATCH",
        &uri,
        &shared.token,
        Some(serde_json::json!({ "clear": ["max_visits"] })),
    )
    .await;
    assert_eq!(lifted.status(), StatusCode::OK);
    assert!(json_body(lifted).await["max_visits"].is_null());
    assert_eq!(
        open_share(&shared.router, &shared.share_url).await["visitCount"],
        3
    );

    // A visit's address is bound to its share: put on another share's token,
    // it opens nothing, even for the same track.
    let other = json_body(
        send_json(
            &shared.router,
            "POST",
            "/api/v2/shares",
            &shared.token,
            Some(serde_json::json!({ "track_ids": [shared.track] })),
        )
        .await,
    )
    .await;
    let other_url = other["url"].as_str().unwrap();
    let crossed = stream.replacen(&shared.share_url, other_url, 1);
    assert_ne!(crossed, stream);
    assert_eq!(
        anonymous(&shared.router, &crossed, None).await.status(),
        StatusCode::NOT_FOUND
    );

    // And a deleted share stops playing at once, whatever the visit carried.
    assert_eq!(
        send_json(&shared.router, "DELETE", &uri, &shared.token, None)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        anonymous(&shared.router, &stream, None).await.status(),
        StatusCode::NOT_FOUND
    );
}

/// The share page shows each track's cover through a route of the share's
/// own, authorised by the visit's pass like the stream: the session-only
/// artwork route stays closed to a visitor, a pass for another track or none
/// at all opens nothing, and a deleted share stops answering at once. A track
/// with no cover is handed no address for one.
#[tokio::test]
async fn a_share_page_shows_covers_through_the_visit_and_nothing_else() {
    let bare = shared_file("coverless-owner", serde_json::json!({})).await;
    let opened = open_share(&bare.router, &bare.share_url).await;
    assert!(
        opened["tracks"][0].get("coverUrl").is_none(),
        "a track with no cover is handed no address for one: {opened}"
    );

    let shared = shared_file_with("cover-owner", serde_json::json!({}), true).await;
    let opened = open_share(&shared.router, &shared.share_url).await;
    let cover = opened["tracks"][0]["coverUrl"]
        .as_str()
        .expect("a track with a cover is handed its address")
        .to_owned();
    let stream = opened["tracks"][0]["streamUrl"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(cover.contains("visit="), "{cover}");
    let served = anonymous(&shared.router, &cover, None).await;
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(served.headers()["content-type"], "image/png");
    assert_eq!(served.headers()["cache-control"], "private, no-cache");
    // A share response like the others (RFC-019 decision 2).
    assert_eq!(served.headers()["referrer-policy"], "no-referrer");
    let etag = served.headers()["etag"].to_str().unwrap().to_owned();
    let bytes = served.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[1..4], b"PNG");
    let revalidated = shared
        .router
        .clone()
        .oneshot(
            Request::get(cover.as_str())
                .header("if-none-match", etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revalidated.status(), StatusCode::NOT_MODIFIED);
    // Opening the cover counted nothing.
    assert_eq!(listed_share(&shared).await["visit_count"], 1);

    let without_pass = cover.split('?').next().unwrap().to_owned();
    assert_eq!(
        anonymous(&shared.router, &without_pass, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        anonymous(&shared.router, &with_query(&cover, "size=64"), None)
            .await
            .status(),
        StatusCode::BAD_REQUEST,
        "nothing but the pass is accepted"
    );
    // The stream's pass is the same pass, so a pass is bound to its track and
    // its share, not to which of the two routes it is presented on; another
    // track id under it opens nothing.
    let pass = stream.split("visit=").nth(1).unwrap();
    let elsewhere = format!(
        "{}/tracks/{}/cover?visit={pass}",
        shared.share_url,
        uuid::Uuid::new_v4()
    );
    assert_eq!(
        anonymous(&shared.router, &elsewhere, None).await.status(),
        StatusCode::NOT_FOUND
    );
    // The session-only route stays session-only.
    assert_eq!(
        anonymous(
            &shared.router,
            &format!("/api/v2/artwork/{}", shared.track),
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );

    let uri = format!("/api/v2/shares/{}", shared.share_id);
    assert_eq!(
        send_json(&shared.router, "DELETE", &uri, &shared.token, None)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        anonymous(&shared.router, &cover, None).await.status(),
        StatusCode::NOT_FOUND,
        "a deleted share stops answering its covers at once"
    );
}
