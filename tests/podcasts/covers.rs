//! The images of channels and radio stations: fetched, refused, served and
//! swept.

use super::*;

/// Subscribes, waits for the cover the subscription fetches in the background,
/// and answers the channel as it then reads.
async fn subscribe_settled(
    router: &Router,
    state: &waveflow_server::AppState,
    token: &str,
    url: &str,
) -> (StatusCode, serde_json::Value) {
    let (status, channel) = subscribe(router, token, url).await;
    if status != StatusCode::CREATED {
        return (status, channel);
    }
    state.services.podcast_covers_settled().await;
    let read = send_json(
        router,
        "GET",
        &format!("/api/v2/podcasts/{}", uuid_of(&channel)),
        token,
        None,
    )
    .await;
    (status, json_body(read).await)
}

/// The hash the store files an image under.
fn hash_of(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// The bytes `/api/v2/artwork/{hash}` answers, or its status when it does not.
async fn artwork(router: &Router, token: &str, hash: &str) -> Result<Vec<u8>, StatusCode> {
    let response = send_json(
        router,
        "GET",
        &format!("/api/v2/artwork/{hash}"),
        token,
        None,
    )
    .await;
    if response.status() == StatusCode::OK {
        Ok(body_bytes(response).await)
    } else {
        Err(response.status())
    }
}

async fn artwork_rows(state: &waveflow_server::AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM artwork")
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

/// RFC-025, amended: a feed's cover is fetched into the artwork store and
/// handed out by its hash — natively, and as Subsonic `coverArt` on channels
/// and episodes — to every account, while the address it came from is shown
/// to a manager only. An unchanged address is not fetched again; a new one
/// is, once, through a redirect, and the cover it replaces is released.
#[tokio::test]
async fn a_feeds_cover_is_fetched_once_and_served_by_its_hash() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (_, admin_password, admin_key) = account(&state, "cover-admin", AccountRole::Admin).await;
    let (reader, reader_password, reader_key) =
        account(&state, "cover-reader", AccountRole::User).await;
    let router = waveflow_server::app(&config, state.clone());
    let admin_token = login_token(&router, "cover-admin", &admin_password).await;
    let reader_token = login_token(&router, "cover-reader", &reader_password).await;

    let (status, channel) = subscribe_settled(
        &router,
        &state,
        &admin_token,
        &format!("{}/feed.xml", host.base),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    let hash = hash_of(&jpeg(1));
    assert_eq!(channel["artwork_hash"], hash.as_str(), "{channel}");
    assert_eq!(host.count("/cover.jpg"), 1);
    assert_eq!(
        channel["image_url"],
        format!("{}/cover.jpg", host.base).as_str(),
        "a manager is shown the address the feed names"
    );
    let channel_id = uuid_of(&channel);

    // A reader is given the hash, never the address.
    let read = json_body(
        send_json(
            &router,
            "GET",
            &format!("/api/v2/podcasts/{channel_id}"),
            &reader_token,
            None,
        )
        .await,
    )
    .await;
    assert!(read["image_url"].is_null(), "{read}");
    assert_eq!(read["artwork_hash"], hash.as_str());
    assert_eq!(
        episode(&read, "Through a redirect")["artwork_hash"],
        hash.as_str(),
        "an episode shows its channel's cover"
    );
    assert_eq!(artwork(&router, &reader_token, &hash).await, Ok(jpeg(1)));

    // Subsonic: `coverArt` on the channel and its episodes, and served by
    // `getCoverArt`; `originalImageUrl` to a manager only.
    let listed = subsonic_json(&router, "getPodcasts", &reader_key, "").await;
    let listed = &listed["subsonic-response"]["podcasts"]["channel"][0];
    assert_eq!(listed["coverArt"], hash.as_str(), "{listed}");
    assert!(omits(listed, "originalImageUrl"), "{listed}");
    assert_eq!(listed["episode"][0]["coverArt"], hash.as_str());
    let newest = subsonic_json(&router, "getNewestPodcasts", &reader_key, "").await;
    assert_eq!(
        newest["subsonic-response"]["newestPodcasts"]["episode"][0]["coverArt"],
        hash.as_str()
    );
    let managed = subsonic_json(&router, "getPodcasts", &admin_key, "").await;
    assert_eq!(
        managed["subsonic-response"]["podcasts"]["channel"][0]["originalImageUrl"],
        format!("{}/cover.jpg", host.base).as_str()
    );
    let cover = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getCoverArt.view?apiKey={reader_key}&v=1.16.1&c=golden&id={hash}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cover.status(), StatusCode::OK);
    assert_eq!(body_bytes(cover).await, jpeg(1));

    // Refreshed with the same address: the feed is read, the cover is not.
    state.services.refresh_all_podcasts().await;
    assert_eq!(host.count("/feed.xml"), 2);
    assert_eq!(
        host.count("/cover.jpg"),
        1,
        "an unchanged cover is not fetched again"
    );

    // The feed names another cover, behind a redirect: fetched once.
    *host.cover.lock().unwrap() = "/redirect-cover".to_owned();
    state.services.refresh_all_podcasts().await;
    state.services.refresh_all_podcasts().await;
    assert_eq!(host.count("/cover-2.jpg"), 1);
    let moved = state
        .services
        .podcast_channel(reader, channel_id)
        .await
        .unwrap();
    assert_eq!(moved.artwork_hash, Some(hash_of(&jpeg(2))));
    // The first cover is named by nothing now, and the sweep forgets it.
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.rows_removed, 1);
    assert_eq!(
        artwork(&router, &reader_token, &hash).await,
        Err(StatusCode::NOT_FOUND)
    );
}

/// What a cover fetch refuses, it refuses before anything is kept: an image
/// over the 10 MiB ceiling, declared or not; a page of HTML, labelled as an
/// image or as what it is; a redirect to a private address. The channel is
/// subscribed all the same, without a cover. A fetch that fails is tried again
/// on the next refresh, and an image labelled as nothing in particular is
/// recognised by its bytes.
#[tokio::test]
async fn a_cover_that_is_not_a_safe_image_stores_nothing() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (admin, password, _) = account(&state, "cover-guard", AccountRole::Admin).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "cover-guard", &password).await;

    use waveflow_server::remote::portrait::Refusal;
    let cases = [
        ("/huge.jpg", Refusal::TooLarge, "declared over the ceiling"),
        (
            "/endless.jpg",
            Refusal::TooLarge,
            "over the ceiling, undeclared",
        ),
        (
            "/page-as-image.jpg",
            Refusal::NotAnImage,
            "HTML labelled as an image",
        ),
        ("/page.htm", Refusal::NotAnImage, "HTML labelled as HTML"),
        (
            "/cover-to-private",
            Refusal::Address,
            "a redirect to a private address",
        ),
        (
            "/cover-to-mapped-private",
            Refusal::Address,
            "a redirect to a mapped private address",
        ),
    ];
    for (cover, _, why) in cases {
        let (status, channel) = subscribe_settled(
            &router,
            &state,
            &token,
            &format!("{}/covered{cover}", host.base),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{why}: {channel}");
        assert!(channel["artwork_hash"].is_null(), "{why}: {channel}");
        assert_eq!(host.count(cover), 1, "{why}");
    }
    // And each for its own reason, asked of the fetcher itself: a refusal and
    // an unreachable host store the same nothing, and only the reason tells
    // a checked hop from one that was attempted and happened to fail.
    let policy = waveflow_server::remote::podcast::FetchPolicy {
        addresses: waveflow_server::remote::portrait::AddressPolicy::AllowLoopback,
        allow_http: false,
        timeout: std::time::Duration::from_secs(10),
    };
    for (cover, refusal, why) in cases {
        let url = url::Url::parse(&format!("{}{cover}", host.base)).unwrap();
        let fetched = waveflow_server::remote::portrait::fetch_followed(&url, policy).await;
        assert_eq!(fetched.err(), Some(refusal), "{why}");
    }
    assert_eq!(artwork_rows(&state).await, 0);
    let written = std::fs::read_dir(&config.artwork_dir)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(written, 0, "a refused cover leaves nothing in the store");

    // Labelled as nothing in particular, and a JPEG by its bytes.
    let (_, channel) = subscribe_settled(
        &router,
        &state,
        &token,
        &format!("{}/covered/unlabelled.jpg", host.base),
    )
    .await;
    assert_eq!(channel["artwork_hash"], hash_of(&jpeg(3)).as_str());

    // A host that fails stores nothing, and the next refresh tries again.
    host.cover_failing.store(true, Ordering::SeqCst);
    let (_, channel) =
        subscribe_settled(&router, &state, &token, &format!("{}/feed.xml", host.base)).await;
    assert!(channel["artwork_hash"].is_null(), "{channel}");
    host.cover_failing.store(false, Ordering::SeqCst);
    state.services.refresh_all_podcasts().await;
    let retried = state
        .services
        .podcast_channel(admin, uuid_of(&channel))
        .await
        .unwrap();
    assert_eq!(retried.artwork_hash, Some(hash_of(&jpeg(1))));
    assert_eq!(host.count("/cover.jpg"), 2);
}

/// A station's image (RFC-025, amended): fetched when an account sets it,
/// served to that account alone, kept by a write that does not mention it —
/// a Subsonic edit above all — and fetched again only when the address
/// changes or the last fetch failed.
#[tokio::test]
async fn a_station_image_is_fetched_when_set_and_served_to_its_account_only() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (_, owner_password, owner_key) = account(&state, "station-owner", AccountRole::User).await;
    let (_, other_password, _) = account(&state, "station-other", AccountRole::User).await;
    let router = waveflow_server::app(&config, state.clone());
    let owner = login_token(&router, "station-owner", &owner_password).await;
    let other = login_token(&router, "station-other", &other_password).await;
    let image_url = format!("{}/station.png", host.base);
    let stream_url = "https://radio.example.org/stream";

    let (status, created) = station(
        &router,
        "POST",
        "/api/v2/radio-stations",
        &owner,
        json!({ "name": "Night Radio", "stream_url": stream_url, "image_url": image_url }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let hash = hash_of(&png());
    assert_eq!(created["artwork_hash"], hash.as_str(), "{created}");
    assert_eq!(created["image_url"], image_url.as_str());
    assert_eq!(host.count("/station.png"), 1);
    let station_id = uuid_of(&created);
    let uri = format!("/api/v2/radio-stations/{station_id}");

    assert_eq!(artwork(&router, &owner, &hash).await, Ok(png()));
    assert_eq!(
        artwork(&router, &other, &hash).await,
        Err(StatusCode::NOT_FOUND),
        "another account's station image is 404, like the station"
    );

    // Saved with the same address: not fetched again.
    let (status, saved) = station(
        &router,
        "PUT",
        &uri,
        &owner,
        json!({ "name": "Night Radio 2", "stream_url": stream_url, "image_url": image_url }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["artwork_hash"], hash.as_str());
    assert_eq!(
        host.count("/station.png"),
        1,
        "an unchanged image is not fetched again"
    );

    // A write that does not mention the image keeps it, natively and through
    // Subsonic, which has no image parameter at all.
    let (_, kept) = station(
        &router,
        "PUT",
        &uri,
        &owner,
        json!({ "name": "Night Radio 3", "stream_url": stream_url }),
    )
    .await;
    assert_eq!(kept["artwork_hash"], hash.as_str(), "{kept}");
    subsonic_json(
        &router,
        "updateInternetRadioStation",
        &owner_key,
        &format!("&id={station_id}&name=Renamed&streamUrl={stream_url}"),
    )
    .await;
    let renamed = json_body(send_json(&router, "GET", &uri, &owner, None).await).await;
    assert_eq!(renamed["name"], "Renamed");
    assert_eq!(renamed["artwork_hash"], hash.as_str(), "{renamed}");
    assert_eq!(renamed["image_url"], image_url.as_str());
    // The protocol's station shape is unchanged.
    let listed = subsonic_json(&router, "getInternetRadioStations", &owner_key, "").await;
    let listed = &listed["subsonic-response"]["internetRadioStations"]["internetRadioStation"][0];
    assert!(omits(listed, "coverArt"), "{listed}");

    // An image that cannot be fetched keeps its address and shows none; the
    // next save that carries the address tries again.
    host.cover_failing.store(true, Ordering::SeqCst);
    let cover_url = format!("{}/cover.jpg", host.base);
    let (status, failed) = station(
        &router,
        "PUT",
        &uri,
        &owner,
        json!({ "name": "Renamed", "stream_url": stream_url, "image_url": cover_url }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{failed}");
    assert_eq!(failed["image_url"], cover_url.as_str());
    assert!(failed["artwork_hash"].is_null(), "{failed}");
    host.cover_failing.store(false, Ordering::SeqCst);
    let (_, retried) = station(
        &router,
        "PUT",
        &uri,
        &owner,
        json!({ "name": "Renamed", "stream_url": stream_url, "image_url": cover_url }),
    )
    .await;
    assert_eq!(
        retried["artwork_hash"],
        hash_of(&jpeg(1)).as_str(),
        "{retried}"
    );
    assert_eq!(host.count("/cover.jpg"), 2);

    // Cleared by `null`, or by a blank value.
    for cleared in [json!(null), json!("  ")] {
        let (_, saved) = station(
            &router,
            "PUT",
            &uri,
            &owner,
            json!({ "name": "Renamed", "stream_url": stream_url, "image_url": cleared }),
        )
        .await;
        assert!(saved["image_url"].is_null(), "{saved}");
        assert!(saved["artwork_hash"].is_null(), "{saved}");
    }

    // Validated as the homepage is, and refused before anything is fetched.
    let (status, _) = station(
        &router,
        "POST",
        "/api/v2/radio-stations",
        &owner,
        json!({ "name": "Bad", "stream_url": stream_url, "image_url": "file:///etc/passwd" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // A redirect to a private address is refused at the hop; the station is
    // kept, without an image.
    let (status, guarded) = station(
        &router,
        "POST",
        "/api/v2/radio-stations",
        &owner,
        json!({
            "name": "Guarded",
            "stream_url": stream_url,
            "image_url": format!("{}/cover-to-private", host.base),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(guarded["artwork_hash"].is_null(), "{guarded}");
    // Another account cannot make this account's station fetch anything.
    let (status, _) = station(
        &router,
        "PUT",
        &uri,
        &other,
        json!({
            "name": "Taken",
            "stream_url": stream_url,
            "image_url": format!("{}/cover-2.jpg", host.base),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(host.count("/cover-2.jpg"), 0);
}

/// Nothing is fetched for a write that is refused. An account at its station
/// ceiling that names an image is answered 409 with no request to the image's
/// host and nothing in the store; an instance at its channel ceiling refuses a
/// subscription before the feed, and so the cover, is fetched.
#[tokio::test]
async fn a_write_refused_for_its_ceiling_fetches_no_image() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (owner, password, _) = account(&state, "ceiling-admin", AccountRole::Admin).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "ceiling-admin", &password).await;

    // Filled in SQL: five hundred requests would prove nothing more.
    let mut tx = state.db.pool().begin().await.unwrap();
    for index in 0..500 {
        sqlx::query(
            "INSERT INTO radio_station (id, user_id, name, stream_url, created_at, updated_at) \
             VALUES (?, ?, ?, 'https://radio.example.org/s', 1, 1)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(owner.to_string())
        .bind(format!("Station {index}"))
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO podcast_channel (id, feed_url, title, status, created_at, updated_at) \
             VALUES (?, ?, 'Filler', 'completed', 1, 1)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(format!("https://feeds.example.org/{index}.xml"))
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();

    let (status, body) = station(
        &router,
        "POST",
        "/api/v2/radio-stations",
        &token,
        json!({
            "name": "One too many",
            "stream_url": "https://radio.example.org/stream",
            "image_url": format!("{}/station.png", host.base),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        host.count("/station.png"),
        0,
        "the image's host was contacted"
    );

    let (status, body) = subscribe(&router, &token, &format!("{}/feed.xml", host.base)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(host.count("/feed.xml"), 0, "the feed's host was contacted");
    assert_eq!(
        host.count("/cover.jpg"),
        0,
        "the cover's host was contacted"
    );

    assert_eq!(artwork_rows(&state).await, 0);
    let written = std::fs::read_dir(&config.artwork_dir)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(written, 0, "a refused write leaves nothing in the store");
}

/// The sweep counts a channel's cover and a station's image as referenced, so
/// neither is collected while it is named, however old its file; deleting the
/// channel and the station releases both.
#[tokio::test]
async fn covers_named_by_a_channel_or_a_station_survive_the_sweep_until_deleted() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (_, password, _) = account(&state, "sweep-admin", AccountRole::Admin).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "sweep-admin", &password).await;

    let (_, channel) =
        subscribe_settled(&router, &state, &token, &format!("{}/feed.xml", host.base)).await;
    let (_, created) = station(
        &router,
        "POST",
        "/api/v2/radio-stations",
        &token,
        json!({
            "name": "Kept",
            "stream_url": "https://radio.example.org/stream",
            "image_url": format!("{}/station.png", host.base),
        }),
    )
    .await;
    let files = [
        config
            .artwork_dir
            .join(format!("{}.jpg", hash_of(&jpeg(1)))),
        config.artwork_dir.join(format!("{}.png", hash_of(&png()))),
    ];
    // Older than the sweep's grace, so only a reference can keep them.
    for file in &files {
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
        std::fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.rows_removed, 0, "{swept:?}");
    assert_eq!(swept.covers_removed, 0, "{swept:?}");
    assert!(files.iter().all(|file| file.exists()));
    assert_eq!(artwork_rows(&state).await, 2);

    // Named by a fixed word, not by the address: a failure message has no
    // reason to print an id.
    for (what, uri) in [
        ("channel", format!("/api/v2/podcasts/{}", uuid_of(&channel))),
        (
            "station",
            format!("/api/v2/radio-stations/{}", uuid_of(&created)),
        ),
    ] {
        let deleted = send_json(&router, "DELETE", &uri, &token, None).await;
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT, "{what}");
    }
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.rows_removed, 2, "{swept:?}");
    assert_eq!(swept.covers_removed, 2, "{swept:?}");
    assert!(files.iter().all(|file| !file.exists()));
}

/// A subscription answers without waiting for its cover (RFC-025 A2). The
/// cover follows in the background: a cover host as slow as the feed would
/// otherwise hold the answer for the sum of both, and on Subsonic's
/// `createPodcastChannel`, which no `TimeoutLayer` bounds, for as long as the
/// host liked. The fixture's cover deadline is ten seconds, so an answer
/// within five is one that did not wait for it.
#[tokio::test]
async fn a_slow_cover_host_does_not_hold_a_subsonic_subscription() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (admin, _, key) = account(&state, "slow-cover-admin", AccountRole::Admin).await;
    let router = waveflow_server::app(&config, state.clone());

    let feed = format!("{}/covered/slow-cover.jpg", host.base);
    let answered = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        subsonic_json(
            &router,
            "createPodcastChannel",
            &key,
            &format!("&url={feed}"),
        ),
    )
    .await
    .expect("the subscription waited for its cover");
    assert_eq!(answered["subsonic-response"]["status"], "ok", "{answered}");

    // Created, with its cover pending.
    let channels = state
        .services
        .podcast_channels(admin, None, false)
        .await
        .unwrap();
    assert_eq!(channels.len(), 1);
    assert!(channels[0].artwork_hash.is_none());
    // And the cover is being asked for, outside the request.
    for _ in 0..500 {
        if host.count("/slow-cover.jpg") > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(host.count("/slow-cover.jpg"), 1);
}

/// A subscription's cover arrives after a refresh has moved on: the late cover
/// is dropped, and the channel keeps what the refresh left — the new cover, or
/// none — never the stale one. Three ways: the feed names another cover, which
/// the refresh records; it names one the refresh cannot fetch; it names the
/// same address, whose image the refresh records anew. The dropped row is
/// left to the sweep.
#[tokio::test]
async fn a_first_cover_that_lands_after_a_refresh_moved_the_feed_is_dropped() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (admin, password, _) = account(&state, "late-cover-admin", AccountRole::Admin).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "late-cover-admin", &password).await;

    // The subscription's cover is held at the host.
    *host.cover.lock().unwrap() = "/gated-cover.jpg".to_owned();
    let (status, channel) = subscribe(&router, &token, &format!("{}/feed.xml", host.base)).await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    for _ in 0..500 {
        if host.count("/gated-cover.jpg") > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(host.count("/gated-cover.jpg"), 1);

    // Meanwhile the feed names another cover, and a refresh records it.
    *host.cover.lock().unwrap() = "/cover-2.jpg".to_owned();
    state.services.refresh_all_podcasts().await;
    let channel_id = uuid_of(&channel);
    let refreshed = state
        .services
        .podcast_channel(admin, channel_id)
        .await
        .unwrap();
    assert_eq!(refreshed.artwork_hash, Some(hash_of(&jpeg(2))));

    // Then the first cover lands.
    host.gate_open.store(true, Ordering::SeqCst);
    state.services.podcast_covers_settled().await;
    let settled = state
        .services
        .podcast_channel(admin, channel_id)
        .await
        .unwrap();
    assert_eq!(
        settled.artwork_hash,
        Some(hash_of(&jpeg(2))),
        "a stale cover replaced the newer one"
    );
    // Whatever the late fetch recorded names nothing, and the sweep forgets it.
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.rows_removed, 1, "{swept:?}");

    // Again, with a refresh that moves the feed to a cover it cannot fetch:
    // no newer cover is recorded, so only the address tells the late one is
    // stale. The channel ends with none.
    host.gate_open.store(false, Ordering::SeqCst);
    *host.cover.lock().unwrap() = "/gated-cover.jpg".to_owned();
    let (status, second) =
        subscribe(&router, &token, &format!("{}/feed.xml?second", host.base)).await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    for _ in 0..500 {
        if host.count("/gated-cover.jpg") > 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(host.count("/gated-cover.jpg"), 2);
    *host.cover.lock().unwrap() = "/no-such-cover.jpg".to_owned();
    state.services.refresh_all_podcasts().await;
    host.gate_open.store(true, Ordering::SeqCst);
    state.services.podcast_covers_settled().await;
    let second = state
        .services
        .podcast_channel(admin, uuid_of(&second))
        .await
        .unwrap();
    assert_eq!(
        second.artwork_hash, None,
        "a cover the feed no longer names was attached"
    );

    // And with the same address both times: the refresh fetches it again
    // while the first fetch is held, and records what it is now. Only the
    // record tells the late cover is the older one.
    host.gate_open.store(false, Ordering::SeqCst);
    *host.cover.lock().unwrap() = "/gated-once.jpg".to_owned();
    let (status, third) =
        subscribe(&router, &token, &format!("{}/feed.xml?third", host.base)).await;
    assert_eq!(status, StatusCode::CREATED, "{third}");
    for _ in 0..500 {
        if host.count("/gated-once.jpg") > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    state.services.refresh_all_podcasts().await;
    // Every channel names it now, so each refreshes it; only the first
    // request, the subscription's, is held.
    assert!(host.count("/gated-once.jpg") > 1);
    host.gate_open.store(true, Ordering::SeqCst);
    state.services.podcast_covers_settled().await;
    let third = state
        .services
        .podcast_channel(admin, uuid_of(&third))
        .await
        .unwrap();
    assert_eq!(
        third.artwork_hash,
        Some(hash_of(&jpeg(8))),
        "an older fetch of the same address replaced the newer one"
    );
}
