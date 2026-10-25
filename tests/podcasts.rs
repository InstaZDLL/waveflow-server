//! Podcasts (RFC-025 decisions 7 to 14), against a feed host of the suite's
//! own on loopback — never the internet.
//!
//! The hostile feeds are here: one larger than the ceiling and streamed so its
//! length is not declared, one declaring entities, redirects to a private
//! address for a feed and for an enclosure, a redirect loop, and an enclosure
//! that is a page of HTML.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
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
}

impl Host {
    fn count(&self, path: &str) -> usize {
        self.hits.get(path).map_or(0, |hits| *hits)
    }
}

/// The feed, its enclosures carrying `tail` — a private feed's enclosures
/// carry its subscriber's token too.
fn feed(base: &str) -> String {
    private_feed(base, "")
}

fn private_feed(base: &str, tail: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
  <channel>
    <title>Night Shift</title>
    <description>Talk after dark</description>
    <link>{base}/</link>
    <itunes:image href="{base}/cover.jpg"/>
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
            private_feed(&host.base, &format!("?auth={token}")),
        )
            .into_response();
    }
    match path.as_str() {
        "/feed.xml" if host.failing.load(Ordering::SeqCst) => {
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        "/feed.xml" => (
            [(header::CONTENT_TYPE, "application/rss+xml")],
            feed(&host.base),
        )
            .into_response(),
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
