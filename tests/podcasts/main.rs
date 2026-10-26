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
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::storage::database::AccountRole;

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod channels;
mod covers;

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
    /// `/gated-cover.jpg` holds its answer until this is set.
    gate_open: Arc<AtomicBool>,
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
        // The gated cover below, for its first request only; every later one
        // answers at once with other bytes, as if the image behind the
        // address changed.
        "/gated-once.jpg" => {
            if host.count("/gated-once.jpg") > 1 {
                return image("image/jpeg", jpeg(8));
            }
            for _ in 0..2_000 {
                if host.gate_open.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            image("image/jpeg", jpeg(7))
        }
        // A cover host that answers only once the test opens the gate.
        "/gated-cover.jpg" => {
            for _ in 0..2_000 {
                if host.gate_open.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            image("image/jpeg", jpeg(7))
        }
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
        gate_open: Arc::default(),
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
