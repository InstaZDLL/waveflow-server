//! Podcasts (RFC-025 decisions 7 to 14), against a feed host of the suite's
//! own on loopback — never the internet.
//!
//! The hostile feeds are here: one larger than the ceiling and streamed so its
//! length is not declared, one declaring entities, redirects to a private
//! address for a feed and for an enclosure, a redirect loop, and an enclosure
//! that is a page of HTML.
//!
//! So are the images fetched from another host (RFC-025, amended): a feed's
//! cover and a radio station's image go through one fetcher, so they are
//! proved against the same host — an image over the ceiling, a page of HTML
//! labelled as an image, a redirect to a private address, an unchanged address
//! not fetched twice, and the sweep keeping what a channel or a station names
//! until it is deleted.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::authentication::now_ms;
use waveflow_server::database::AccountRole;
use waveflow_server::security;

#[allow(dead_code)]
mod support;
use support::*;

/// The episode the fake host serves: an ID3 header, then bytes.
fn audio() -> Vec<u8> {
    let mut bytes = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
    bytes.extend((0..64 * 1024).map(|index| (index % 251) as u8));
    bytes
}

#[derive(Clone)]
struct Host {
    base: Arc<String>,
    /// Every request the host answered, by path.
    hits: Arc<dashmap::DashMap<String, usize>>,
    /// The feed answers 500 while this is set.
    failing: Arc<AtomicBool>,
    /// `/cover.jpg` answers 500 while this is set.
    cover_failing: Arc<AtomicBool>,
    /// The cover `/feed.xml` names, as a path on this host.
    cover: Arc<Mutex<String>>,
}

impl Host {
    fn count(&self, path: &str) -> usize {
        self.hits.get(path).map_or(0, |hits| *hits)
    }
}

/// A JPEG by its first bytes, which is all the server checks: the start of
/// image and an APP0 marker, `seed` to tell two covers apart, padding.
fn jpeg(seed: u8) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0, seed];
    bytes.extend((0..4096).map(|index| (index % 251) as u8));
    bytes
}

/// A PNG by its signature.
fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend((0..4096).map(|index| (index % 241) as u8));
    bytes
}

/// The feed, its enclosures carrying `tail` — a private feed's enclosures
/// carry its subscriber's token too.
fn feed(base: &str, cover: &str) -> String {
    private_feed(base, "", cover)
}

/// A feed naming `cover` as its image: a path on the host, with the private
/// feed's token on it as `cover=` rather than `auth=`, so a test can tell the
/// cover's address from an enclosure's.
fn private_feed(base: &str, tail: &str, cover: &str) -> String {
    let cover_tail = tail.replacen("?auth=", "?cover=", 1);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
  <channel>
    <title>Night Shift</title>
    <description>Talk after dark</description>
    <link>{base}/</link>
    <itunes:image href="{base}{cover}{cover_tail}"/>
    <item><title>Through a redirect</title><guid>ep-1</guid>
      <pubDate>Fri, 05 Jan 2024 10:00:00 +0000</pubDate><itunes:duration>12:00</itunes:duration>
      <enclosure url="{base}/redirect-audio{tail}" type="audio/mpeg"/></item>
    <item><title>A page, not audio</title><guid>ep-2</guid>
      <pubDate>Thu, 04 Jan 2024 10:00:00 +0000</pubDate>
      <enclosure url="{base}/page.html{tail}" type="audio/mpeg"/></item>
    <item><title>Behind a private address</title><guid>ep-3</guid>
      <pubDate>Wed, 03 Jan 2024 10:00:00 +0000</pubDate>
      <enclosure url="{base}/to-private{tail}" type="audio/mpeg"/></item>
    <item><title>Too large</title><guid>ep-4</guid>
      <pubDate>Tue, 02 Jan 2024 10:00:00 +0000</pubDate>
      <enclosure url="{base}/huge.mp3{tail}" type="audio/mpeg"/></item>
  </channel>
</rss>"#
    )
}

async fn serve(State(host): State<Host>, request: Request<Body>) -> Response {
    let path = request.uri().path().to_owned();
    *host.hits.entry(path.clone()).or_insert(0) += 1;
    let redirect =
        |to: &str| (StatusCode::FOUND, [(header::LOCATION, to.to_owned())], "").into_response();
    // A private feed, as Supercast and Patreon serve them: the subscriber's
    // token in the path, and on every enclosure.
    if let Some(token) = path
        .strip_prefix("/private/")
        .and_then(|rest| rest.strip_suffix("/podcast.xml"))
    {
        return (
            [(header::CONTENT_TYPE, "application/rss+xml")],
            private_feed(&host.base, &format!("?auth={token}"), "/cover.jpg"),
        )
            .into_response();
    }
    // A feed naming whatever follows the prefix as its cover, for the
    // hostile covers below.
    if let Some(cover) = path.strip_prefix("/covered") {
        return (
            [(header::CONTENT_TYPE, "application/rss+xml")],
            feed(&host.base, cover),
        )
            .into_response();
    }
    let image = |mime: &'static str, bytes: Vec<u8>| {
        ([(header::CONTENT_TYPE, mime)], bytes).into_response()
    };
    match path.as_str() {
        "/feed.xml" if host.failing.load(Ordering::SeqCst) => {
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        "/feed.xml" => {
            let cover = host.cover.lock().unwrap().clone();
            (
                [(header::CONTENT_TYPE, "application/rss+xml")],
                feed(&host.base, &cover),
            )
                .into_response()
        }
        "/cover.jpg" if host.cover_failing.load(Ordering::SeqCst) => {
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        "/cover.jpg" => image("image/jpeg", jpeg(1)),
        "/cover-2.jpg" => image("image/jpeg", jpeg(2)),
        // A cover host that answers, eventually: longer than any request
        // should be held open for it.
        "/slow-cover.jpg" => {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            image("image/jpeg", jpeg(6))
        }
        // Object storage labels everything so; the bytes decide.
        "/unlabelled.jpg" => image("application/octet-stream", jpeg(3)),
        "/station.png" => image("image/png", png()),
        "/redirect-cover" => redirect("/cover-2.jpg"),
        // Over the 10 MiB ceiling, declared.
        "/huge.jpg" => {
            let mut bytes = jpeg(4);
            bytes.resize(11 * 1024 * 1024, 0);
            image("image/jpeg", bytes)
        }
        // Over the ceiling and never saying so: met while reading.
        "/endless.jpg" => {
            let head = futures_util::stream::once(async { Ok::<_, std::io::Error>(jpeg(5)) });
            let tail = futures_util::stream::iter(
                (0..200).map(|_| Ok::<_, std::io::Error>(vec![0u8; 64 * 1024])),
            );
            (
                [(header::CONTENT_TYPE, "image/jpeg")],
                Body::from_stream(futures_util::StreamExt::chain(head, tail)),
            )
                .into_response()
        }
        // A page of HTML, labelled as an image and as what it is.
        "/page-as-image.jpg" => image(
            "image/jpeg",
            b"<html><body>not a cover</body></html>".to_vec(),
        ),
        "/page.htm" => image(
            "text/html",
            b"<html><body>not a cover</body></html>".to_vec(),
        ),
        // Over https, so that the address check is the only rule that refuses
        // the hop: a plain-http hop would be refused for its scheme first.
        "/cover-to-private" => redirect("https://10.255.255.1/cover.jpg"),
        "/cover-to-mapped-private" => redirect("https://[::ffff:192.168.1.1]/cover.jpg"),
        // Streamed, so no length is declared and the ceiling has to be met
        // while reading.
        "/big.xml" => {
            let chunks = (0..64).map(|_| Ok::<_, std::io::Error>(vec![b' '; 16 * 1024]));
            Body::from_stream(futures_util::stream::iter(chunks)).into_response()
        }
        "/laughs.xml" => concat!(
            r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol">"#,
            r#"<!ENTITY lol1 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">"#,
            r#"<!ENTITY lol2 "&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;">]>"#,
            r#"<rss><channel><title>&lol2;</title></channel></rss>"#
        )
        .into_response(),
        "/moved.xml" => redirect("http://10.255.255.1/feed.xml"),
        "/loop.xml" => redirect("/loop.xml"),
        "/redirect-audio" => redirect("/audio.mp3"),
        "/to-private" => redirect("http://[::ffff:192.168.1.1]/audio.mp3"),
        "/audio.mp3" => ([(header::CONTENT_TYPE, "audio/mpeg")], audio()).into_response(),
        "/page.html" => (
            [(header::CONTENT_TYPE, "audio/mpeg")],
            "<html><body>not an episode</body></html>",
        )
            .into_response(),
        "/huge.mp3" => {
            let mut bytes = b"ID3\x04".to_vec();
            bytes.resize(2 * 1024 * 1024, 0);
            bytes.into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn fake_host() -> Host {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let host = Host {
        base: Arc::new(format!("http://127.0.0.1:{port}")),
        hits: Arc::default(),
        failing: Arc::default(),
        cover_failing: Arc::default(),
        cover: Arc::new(Mutex::new("/cover.jpg".to_owned())),
    };
    let router = Router::new().fallback(serve).with_state(host.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    host
}

/// A server with podcasts switched on, as an operator would.
async fn podcast_app(
    adjust: impl FnOnce(&mut waveflow_server::Config),
) -> (
    tempfile::TempDir,
    waveflow_server::Config,
    waveflow_server::AppState,
) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.podcasts.enabled = true;
    adjust(&mut config);
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
}

/// An account with a password and a Subsonic key drawn at run time.
async fn account(
    state: &waveflow_server::AppState,
    username: &str,
    role: AccountRole,
) -> (Uuid, String, String) {
    let password = Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let id = state
        .db
        .create_account(username, &hash, role, now_ms())
        .await
        .unwrap();
    let api_key = format!("wfsk_{}", Uuid::new_v4().simple());
    state
        .db
        .set_subsonic_credential(
            id,
            id,
            &state
                .secret_box
                .encrypt(Uuid::new_v4().to_string().as_bytes())
                .unwrap(),
            &security::token_hash(&api_key),
            now_ms(),
        )
        .await
        .unwrap();
    (id, password, api_key)
}

async fn subscribe(router: &Router, token: &str, url: &str) -> (StatusCode, serde_json::Value) {
    let response = send_json(
        router,
        "POST",
        "/api/v2/podcasts",
        token,
        Some(json!({ "url": url })),
    )
    .await;
    let status = response.status();
    (status, json_body(response).await)
}

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

fn episode<'a>(channel: &'a serde_json::Value, title: &str) -> &'a serde_json::Value {
    channel["episodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|episode| episode["title"] == title)
        .unwrap_or_else(|| panic!("no episode called {title}: {channel}"))
}

/// Waits for an episode's download to end, answering its final state.
async fn settled(state: &waveflow_server::AppState, user: Uuid, id: Uuid) -> serde_json::Value {
    for _ in 0..500 {
        let episode = state.services.podcast_episode(user, id).await.unwrap();
        if episode.status != "downloading" {
            return serde_json::to_value(episode).unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the download never ended");
}

fn uuid_of(value: &serde_json::Value) -> Uuid {
    value["id"].as_str().unwrap().parse().unwrap()
}

/// Decisions 7 to 13 end to end: an administrator subscribes, the feed is read
/// through a redirect, an episode is downloaded under a derived name, played
/// natively and through Subsonic, bookmarked through the bookmark routes, and
/// deleted for good.
#[tokio::test]
async fn a_channel_is_subscribed_downloaded_played_bookmarked_and_deleted() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (admin, admin_password, admin_key) = account(&state, "pod-admin", AccountRole::Admin).await;
    let (listener, listener_password, listener_key) =
        account(&state, "pod-listener", AccountRole::User).await;
    let router = waveflow_server::app(&config, state.clone());
    let admin_token = login_token(&router, "pod-admin", &admin_password).await;
    let listener_token = login_token(&router, "pod-listener", &listener_password).await;

    // A listener may not subscribe: the 404 every refusal here answers.
    let (status, _) = subscribe(&router, &listener_token, &format!("{}/feed.xml", host.base)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let feed_url = format!("{}/feed.xml?token={}", host.base, Uuid::new_v4().simple());
    let (status, channel) = subscribe(&router, &admin_token, &feed_url).await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    assert_eq!(channel["title"], "Night Shift");
    assert_eq!(channel["url"], feed_url.as_str());
    assert_eq!(channel["episode_count"], 4);
    let channel_id = uuid_of(&channel);
    // Twice is a conflict, and fetches nothing more.
    let (status, _) = subscribe(&router, &admin_token, &feed_url).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(host.count("/feed.xml"), 1);

    // The overview says who may manage, and shows a reader no feed address.
    let overview =
        json_body(send_json(&router, "GET", "/api/v2/podcasts", &listener_token, None).await).await;
    assert_eq!(overview["enabled"], true);
    assert_eq!(overview["can_manage"], false);
    assert!(overview["channels"][0]["url"].is_null());

    let first = uuid_of(episode(&channel, "Through a redirect"));
    let refused = send_json(
        &router,
        "POST",
        &format!("/api/v2/podcast-episodes/{first}/download"),
        &listener_token,
        None,
    )
    .await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    let queued = send_json(
        &router,
        "POST",
        &format!("/api/v2/podcast-episodes/{first}/download"),
        &admin_token,
        None,
    )
    .await;
    assert_eq!(queued.status(), StatusCode::ACCEPTED);
    let stored = settled(&state, admin, first).await;
    assert_eq!(stored["status"], "completed", "{stored}");
    assert_eq!(stored["suffix"], "mp3");
    assert_eq!(stored["size"], audio().len());

    // Decision 11: named by its id and its bytes, inside the podcast directory,
    // and nothing half written left behind.
    let names: Vec<String> = std::fs::read_dir(&config.podcasts.dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec![format!("{}.mp3", first.simple())]);

    // Played by any account, natively through a ticket and through Subsonic.
    let ticket = json_body(
        send_json(
            &router,
            "POST",
            &format!("/api/v2/tracks/{first}/stream-ticket"),
            &listener_token,
            None,
        )
        .await,
    )
    .await;
    let played = router
        .clone()
        .oneshot(
            Request::get(ticket["url"].as_str().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(played.status(), StatusCode::OK);
    assert_eq!(body_bytes(played).await, audio());
    let streamed = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/stream.view?apiKey={listener_key}&v=1.16.1&c=golden&id={first}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(streamed.status(), StatusCode::OK);
    assert_eq!(body_bytes(streamed).await, audio());

    // Decision 13: the bookmark routes take an episode id.
    let bookmarked = send_json(
        &router,
        "PUT",
        &format!("/api/v2/bookmarks/{first}"),
        &listener_token,
        Some(json!({ "position_ms": 90_000 })),
    )
    .await;
    assert_eq!(bookmarked.status(), StatusCode::NO_CONTENT);
    let read = json_body(
        send_json(
            &router,
            "GET",
            &format!("/api/v2/podcasts/{channel_id}"),
            &listener_token,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(episode(&read, "Through a redirect")["position_ms"], 90_000);
    let others = json_body(
        send_json(
            &router,
            "GET",
            &format!("/api/v2/podcasts/{channel_id}"),
            &admin_token,
            None,
        )
        .await,
    )
    .await;
    assert!(episode(&others, "Through a redirect")["position_ms"].is_null());
    let bookmarks = subsonic_json(&router, "getBookmarks", &listener_key, "").await;
    let entries = bookmarks["subsonic-response"]["bookmarks"]["bookmark"]
        .as_array()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["position"], 90_000);
    assert_eq!(entries[0]["entry"]["id"], first.to_string().as_str());
    assert_eq!(entries[0]["entry"]["type"], "podcast");
    subsonic_json(
        &router,
        "deleteBookmark",
        &listener_key,
        &format!("&id={first}"),
    )
    .await;
    let read = json_body(
        send_json(
            &router,
            "GET",
            &format!("/api/v2/podcasts/{channel_id}"),
            &listener_token,
            None,
        )
        .await,
    )
    .await;
    assert!(episode(&read, "Through a redirect")["position_ms"].is_null());

    // Deleted: the file goes, and a refresh does not bring the episode back.
    let deleted = send_json(
        &router,
        "DELETE",
        &format!("/api/v2/podcast-episodes/{first}"),
        &admin_token,
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(std::fs::read_dir(&config.podcasts.dir).unwrap().count(), 0);
    state.services.refresh_all_podcasts().await;
    assert_eq!(host.count("/feed.xml"), 2);
    let read = json_body(
        send_json(
            &router,
            "GET",
            &format!("/api/v2/podcasts/{channel_id}"),
            &admin_token,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(read["episodes"].as_array().unwrap().len(), 3);
    assert_eq!(
        state
            .services
            .podcast_episode(admin, first)
            .await
            .unwrap()
            .status,
        "deleted"
    );
    let gone = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/stream.view?apiKey={admin_key}&v=1.16.1&c=golden&id={first}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let gone = json_or_xml_code(gone).await;
    assert!(gone.contains("code=\"70\""), "{gone}");

    // A refresh that fails is recorded on the channel as a word, not a page.
    host.failing.store(true, Ordering::SeqCst);
    state.services.refresh_all_podcasts().await;
    let read = state
        .services
        .podcast_channel(admin, channel_id)
        .await
        .unwrap();
    assert_eq!(read.status, "error");
    assert_eq!(read.error.as_deref(), Some("http_500"));

    let unsubscribed = send_json(
        &router,
        "DELETE",
        &format!("/api/v2/podcasts/{channel_id}"),
        &admin_token,
        None,
    )
    .await;
    assert_eq!(unsubscribed.status(), StatusCode::NO_CONTENT);
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM podcast_episode")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(left, 0);
    let _ = listener;
}

/// Decisions 8 to 11 against a hostile host. Each refusal is a word from the
/// fixed list, nothing is stored for a feed that was refused, and nothing is
/// written to disk for an episode that was.
#[tokio::test]
async fn hostile_feeds_and_enclosures_are_refused_before_anything_is_kept() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (admin, password, _) = account(&state, "pod-guard", AccountRole::Admin).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "pod-guard", &password).await;

    for (path, reason) in [
        // Larger than the 256 KiB the fixture allows, with no length declared.
        ("/big.xml", "too_large"),
        // Entities declared in a DTD: refused before anything expands.
        ("/laughs.xml", "invalid_feed"),
        // A redirect to a private address is checked before it is followed.
        ("/moved.xml", "address_refused"),
        ("/loop.xml", "too_many_redirects"),
        ("/nothing-here.xml", "http_404"),
    ] {
        let (status, body) = subscribe(&router, &token, &format!("{}{path}", host.base)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path}: {body}");
        assert_eq!(body["code"], "feed_rejected");
        assert_eq!(body["reason"], reason, "{path}");
    }
    // The loop was followed five times past the first request, and no more.
    assert_eq!(host.count("/loop.xml"), 6);
    // Addresses refused before anything is fetched.
    for url in [
        "file:///etc/passwd",
        "https://user:secret@example.org/feed.xml",
        "http://10.0.0.1/feed.xml",
        "ftp://example.org/feed.xml",
    ] {
        let (status, _) = subscribe(&router, &token, url).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{url}");
    }
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM podcast_channel")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(stored, 0);

    let (status, channel) = subscribe(&router, &token, &format!("{}/feed.xml", host.base)).await;
    assert_eq!(status, StatusCode::CREATED);
    for (title, reason) in [
        ("A page, not audio", "not_audio"),
        ("Behind a private address", "address_refused"),
        ("Too large", "too_large"),
    ] {
        let id = uuid_of(episode(&channel, title));
        state
            .services
            .download_podcast_episode(admin, id)
            .await
            .unwrap();
        let ended = settled(&state, admin, id).await;
        assert_eq!(ended["status"], "error", "{title}");
        assert_eq!(ended["error"], reason, "{title}");
    }
    let written = std::fs::read_dir(&config.podcasts.dir)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(written, 0, "a refused episode leaves nothing on disk");
}

/// Decision 11: the downloads share a quota, and the one that would pass it
/// is refused as such.
#[tokio::test]
async fn an_episode_that_would_pass_the_quota_is_refused() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|config| {
        config.podcasts.quota_bytes = 32 * 1024;
    })
    .await;
    let (admin, password, _) = account(&state, "pod-quota", AccountRole::Admin).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "pod-quota", &password).await;
    let (_, channel) = subscribe(&router, &token, &format!("{}/feed.xml", host.base)).await;
    let id = uuid_of(episode(&channel, "Through a redirect"));
    state
        .services
        .download_podcast_episode(admin, id)
        .await
        .unwrap();
    let ended = settled(&state, admin, id).await;
    assert_eq!(ended["error"], "quota_exceeded");
}

/// Decisions 7 and 12: off by default. Nothing can be subscribed to, every
/// podcast write is error 50, `podcastRole` is false for everyone — and the
/// reads answer the empty containers, arrays in JSON.
#[tokio::test]
async fn podcasts_are_off_by_default_and_the_role_says_so() {
    let host = fake_host().await;
    let (_temp, config, state) = test_app().await;
    let (_, password, api_key) = account(&state, "pod-off", AccountRole::Admin).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "pod-off", &password).await;
    let (status, _) = subscribe(&router, &token, &format!("{}/feed.xml", host.base)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(host.count("/feed.xml"), 0, "nothing was fetched");
    let overview =
        json_body(send_json(&router, "GET", "/api/v2/podcasts", &token, None).await).await;
    assert_eq!(overview["enabled"], false);
    assert_eq!(overview["can_manage"], false);

    let user = subsonic_json(&router, "getUser", &api_key, "&username=pod-off").await;
    assert_eq!(user["subsonic-response"]["user"]["podcastRole"], false);
    let created = subsonic_json(
        &router,
        "createPodcastChannel",
        &api_key,
        &format!("&url={}/feed.xml", host.base),
    )
    .await;
    assert_eq!(created["subsonic-response"]["error"]["code"], 50);
    let podcasts = subsonic_json(&router, "getPodcasts", &api_key, "").await;
    assert_eq!(podcasts["subsonic-response"]["status"], "ok");
    assert!(podcasts["subsonic-response"]["podcasts"].is_object());
}

/// Decision 12: the Subsonic methods, their shapes, and `podcastRole` true for
/// the administrator once the switch is on.
#[tokio::test]
async fn the_subsonic_podcast_methods_answer_the_schema_shapes() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (admin, _, admin_key) = account(&state, "pod-sub-admin", AccountRole::Admin).await;
    let (_, _, listener_key) = account(&state, "pod-sub-listener", AccountRole::User).await;
    let router = waveflow_server::app(&config, state.clone());

    let user = subsonic_json(&router, "getUser", &admin_key, "&username=pod-sub-admin").await;
    assert_eq!(user["subsonic-response"]["user"]["podcastRole"], true);
    let users = subsonic_json(&router, "getUsers", &admin_key, "").await;
    let listed = users["subsonic-response"]["users"]["user"]
        .as_array()
        .unwrap();
    let listener = listed
        .iter()
        .find(|user| user["username"] == "pod-sub-listener")
        .unwrap();
    assert_eq!(listener["podcastRole"], false);

    let refused = subsonic_json(
        &router,
        "createPodcastChannel",
        &listener_key,
        &format!("&url={}/feed.xml", host.base),
    )
    .await;
    assert_eq!(refused["subsonic-response"]["error"]["code"], 50);
    let created = subsonic_json(
        &router,
        "createPodcastChannel",
        &admin_key,
        &format!("&url={}/feed.xml", host.base),
    )
    .await;
    assert_eq!(created["subsonic-response"]["status"], "ok", "{created}");
    assert!(
        created["subsonic-response"]
            .get("createPodcastChannel")
            .is_none(),
        "a write answers the bare envelope"
    );
    let hostile = subsonic_json(
        &router,
        "createPodcastChannel",
        &admin_key,
        &format!("&url={}/laughs.xml", host.base),
    )
    .await;
    assert_eq!(hostile["subsonic-response"]["status"], "failed");

    let podcasts = subsonic_json(&router, "getPodcasts", &listener_key, "").await;
    let channels = podcasts["subsonic-response"]["podcasts"]["channel"]
        .as_array()
        .expect("channels are an array");
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0]["title"], "Night Shift");
    assert_eq!(channels[0]["status"], "completed");
    let episodes = channels[0]["episode"]
        .as_array()
        .expect("episodes are an array");
    assert_eq!(episodes.len(), 4);
    assert_eq!(episodes[0]["title"], "Through a redirect");
    assert_eq!(episodes[0]["status"], "new");
    assert!(
        episodes[0].get("streamId").is_none(),
        "nothing to stream yet"
    );
    let first = episodes[0]["id"].as_str().unwrap().to_owned();

    let bare = subsonic_json(
        &router,
        "getPodcasts",
        &listener_key,
        "&includeEpisodes=false",
    )
    .await;
    assert!(bare["subsonic-response"]["podcasts"]["channel"][0]
        .get("episode")
        .is_none());

    let downloaded = subsonic_json(
        &router,
        "downloadPodcastEpisode",
        &admin_key,
        &format!("&id={first}"),
    )
    .await;
    assert_eq!(downloaded["subsonic-response"]["status"], "ok");
    settled(&state, admin, first.parse().unwrap()).await;
    let one = subsonic_json(
        &router,
        "getPodcastEpisode",
        &listener_key,
        &format!("&id={first}"),
    )
    .await;
    let one = &one["subsonic-response"]["podcastEpisode"];
    assert_eq!(one["status"], "completed");
    assert_eq!(one["streamId"], first.as_str());
    assert_eq!(one["suffix"], "mp3");
    assert_eq!(one["duration"], 720);

    let newest = subsonic_json(&router, "getNewestPodcasts", &listener_key, "&count=2").await;
    let newest = newest["subsonic-response"]["newestPodcasts"]["episode"]
        .as_array()
        .expect("newest episodes are an array");
    assert_eq!(newest.len(), 2);
    assert_eq!(newest[0]["id"], first.as_str());

    let refreshed = subsonic_json(&router, "refreshPodcasts", &admin_key, "").await;
    assert_eq!(refreshed["subsonic-response"]["status"], "ok");
    let deleted = subsonic_json(
        &router,
        "deletePodcastEpisode",
        &admin_key,
        &format!("&id={first}"),
    )
    .await;
    assert_eq!(deleted["subsonic-response"]["status"], "ok");
    let channel_id = channels[0]["id"].as_str().unwrap().to_owned();
    let gone = subsonic_json(
        &router,
        "deletePodcastChannel",
        &admin_key,
        &format!("&id={channel_id}"),
    )
    .await;
    assert_eq!(gone["subsonic-response"]["status"], "ok");
    let empty = subsonic_json(&router, "getPodcasts", &listener_key, "").await;
    assert!(empty["subsonic-response"]["podcasts"]
        .get("channel")
        .is_none_or(|channels| channels.as_array().is_some_and(Vec::is_empty)));
}

/// Decision 8: a private feed's token — in its path, in its query and on every
/// enclosure — reaches the account that may manage podcasts and nobody else,
/// on any surface. A reader is shown the feed's scheme and host in the
/// Subsonic `url` the schema requires, and no address at all natively.
#[tokio::test]
async fn a_private_feeds_token_reaches_only_the_accounts_that_manage_it() {
    let host = fake_host().await;
    let (_temp, config, state) = podcast_app(|_| {}).await;
    let (admin, admin_password, admin_key) =
        account(&state, "pod-private-admin", AccountRole::Admin).await;
    let (_, reader_password, reader_key) =
        account(&state, "pod-private-reader", AccountRole::User).await;
    let router = waveflow_server::app(&config, state.clone());
    let admin_token = login_token(&router, "pod-private-admin", &admin_password).await;
    let reader_token = login_token(&router, "pod-private-reader", &reader_password).await;

    let path_token = format!("p{}", Uuid::new_v4().simple());
    let query_token = format!("q{}", Uuid::new_v4().simple());
    let feed_url = format!(
        "{}/private/{path_token}/podcast.xml?key={query_token}",
        host.base
    );
    let (status, channel) = subscribe(&router, &admin_token, &feed_url).await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    let channel_id = uuid_of(&channel);
    let first = uuid_of(episode(&channel, "Through a redirect"));
    // Downloaded through an enclosure that carries the token as well.
    state
        .services
        .download_podcast_episode(admin, first)
        .await
        .unwrap();
    assert_eq!(settled(&state, admin, first).await["status"], "completed");
    let bookmarked = send_json(
        &router,
        "PUT",
        &format!("/api/v2/bookmarks/{first}"),
        &reader_token,
        Some(json!({ "position_ms": 1_000 })),
    )
    .await;
    assert_eq!(bookmarked.status(), StatusCode::NO_CONTENT);

    for answer in every_podcast_read(&router, &reader_token, &reader_key, channel_id, first).await {
        assert!(
            !answer.contains(&path_token),
            "the path token leaked: {answer}"
        );
        assert!(
            !answer.contains(&query_token),
            "the query token leaked: {answer}"
        );
        assert!(
            !answer.contains("/private/"),
            "the feed path leaked: {answer}"
        );
    }
    let listed = subsonic_json(
        &router,
        "getPodcasts",
        &reader_key,
        "&includeEpisodes=false",
    )
    .await;
    assert_eq!(
        listed["subsonic-response"]["podcasts"]["channel"][0]["url"],
        format!("{}/", host.base).as_str(),
        "a reader is shown the scheme and host, and nothing after"
    );
    let overview =
        json_body(send_json(&router, "GET", "/api/v2/podcasts", &reader_token, None).await).await;
    assert!(overview["channels"][0]["url"].is_null());

    // The manager is shown the address in full, natively and through Subsonic.
    let managed = every_podcast_read(&router, &admin_token, &admin_key, channel_id, first).await;
    assert!(managed[0].contains(&path_token) && managed[0].contains(&query_token));
    assert!(managed[4].contains(&path_token) && managed[4].contains(&query_token));
    // An enclosure address is shown to nobody: an episode is played from the
    // server, so no surface needs it.
    for answer in managed {
        assert!(!answer.contains(&format!("auth={path_token}")), "{answer}");
    }
}

/// Every podcast read an account can make, natively and through Subsonic, as
/// the text it was answered with.
async fn every_podcast_read(
    router: &Router,
    token: &str,
    key: &str,
    channel: Uuid,
    episode: Uuid,
) -> Vec<String> {
    let mut answers = Vec::new();
    for path in [
        "/api/v2/podcasts".to_owned(),
        format!("/api/v2/podcasts/{channel}"),
        "/api/v2/podcast-episodes/newest".to_owned(),
        format!("/api/v2/podcast-episodes/{episode}"),
    ] {
        answers.push(
            json_body(send_json(router, "GET", &path, token, None).await)
                .await
                .to_string(),
        );
    }
    for (method, extra) in [
        ("getPodcasts", String::new()),
        ("getNewestPodcasts", String::new()),
        ("getPodcastEpisode", format!("&id={episode}")),
        ("getBookmarks", String::new()),
    ] {
        answers.push(subsonic_json(router, method, key, &extra).await.to_string());
    }
    answers
}

async fn body_bytes(response: Response) -> Vec<u8> {
    use http_body_util::BodyExt;
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

async fn json_or_xml_code(response: Response) -> String {
    String::from_utf8(body_bytes(response).await).unwrap()
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

async fn station(
    router: &Router,
    method: &str,
    uri: &str,
    token: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = send_json(router, method, uri, token, Some(body)).await;
    let status = response.status();
    (status, json_body(response).await)
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

    use waveflow_server::portrait::Refusal;
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
    let policy = waveflow_server::podcast::FetchPolicy {
        addresses: waveflow_server::portrait::AddressPolicy::AllowLoopback,
        allow_http: false,
        timeout: std::time::Duration::from_secs(10),
    };
    for (cover, refusal, why) in cases {
        let url = url::Url::parse(&format!("{}{cover}", host.base)).unwrap();
        let fetched = waveflow_server::portrait::fetch_followed(&url, policy).await;
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
