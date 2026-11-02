//! Transcoding profiles on the native stream routes (RFC-027): a profile fills
//! a silent request on the bearer route, and touches nothing else.

use super::*;

const WAV_SECOND: usize = 8_000;

struct Listener {
    _temp: tempfile::TempDir,
    config: Config,
    state: waveflow_server::AppState,
    router: axum::Router,
    owner: uuid::Uuid,
    token: String,
    /// One second, shortest first; the last is ten minutes.
    tracks: Vec<uuid::Uuid>,
}

impl Listener {
    async fn new() -> Self {
        let (temp, config, state) = test_app().await;
        // Inserted rather than created, so no literal reaches a cryptographic
        // call: this account authenticates by API token.
        let owner = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
             VALUES (?, 'profile-listener', 'this-account-never-authenticates', 'admin', 0, ?, ?)",
        )
        .bind(owner.to_string())
        .bind(now_ms())
        .bind(now_ms())
        .execute(state.db.pool())
        .await
        .unwrap();
        let music = config.data_dir.join("profile-music");
        std::fs::create_dir_all(&music).unwrap();
        write_test_wav_of_len(&music.join("a-short.wav"), WAV_SECOND);
        // Long enough that its encode outgrows the channel and the pipe behind
        // it, and sits holding its slots until the response is read or dropped.
        write_test_wav_of_len(&music.join("b-long.wav"), WAV_SECOND * 600);
        let root = std::fs::canonicalize(&music).unwrap();
        let library_id = state
            .db
            .create_library(
                owner,
                "Profiles",
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
                name: "Profiles".into(),
                root_path: root,
            },
        )
        .await;
        let mut tracks = state
            .db
            .list_tracks_for_user(owner, library_id)
            .await
            .unwrap();
        tracks.sort_by_key(|track| track.duration_ms);
        assert_eq!(tracks.len(), 2);
        let (_record, token) = state
            .services
            .create_api_token(owner, &[], "profile-listener", "profiles", &[])
            .await
            .unwrap();
        let router = waveflow_server::app(&config, state.clone());
        Self {
            _temp: temp,
            config,
            state,
            router,
            owner,
            token,
            tracks: tracks.into_iter().map(|track| track.id).collect(),
        }
    }

    async fn set_default(&self, format: &str, bitrate: u32) {
        self.state
            .services
            .set_transcoding_profile(self.owner, None, format, Some(bitrate))
            .await
            .unwrap();
    }

    async fn get(&self, uri: &str, range: Option<&str>) -> axum::response::Response {
        let mut request =
            Request::get(uri).header("authorization", format!("Bearer {}", self.token));
        if let Some(range) = range {
            request = request.header("range", range);
        }
        self.router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    /// The short track's stream route, with `query` after it.
    fn stream(&self, query: &str) -> String {
        format!("/api/v2/tracks/{}/stream{query}", self.tracks[0])
    }

    async fn ticket(&self) -> String {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::post(format!("/api/v2/tracks/{}/stream-ticket", self.tracks[0]))
                    .header("authorization", format!("Bearer {}", self.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await["url"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

/// The media type a response names, and the bytes it starts with.
async fn sound(response: axum::response::Response) -> (String, Vec<u8>) {
    assert!(response.status().is_success(), "{}", response.status());
    let kind = response.headers()["content-type"]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (kind, bytes[..4].to_vec())
}

fn is_original((kind, start): &(String, Vec<u8>)) -> bool {
    kind == "audio/wav" && start == b"RIFF"
}

fn is_opus((kind, start): &(String, Vec<u8>)) -> bool {
    kind == "audio/ogg" && start == b"OggS"
}

/// An account with no profile takes the path it always took: the file, by byte
/// range, and the refusals a request without a format has always had.
#[tokio::test]
async fn with_no_profile_a_silent_stream_is_the_original() {
    let listener = Listener::new().await;

    let whole = listener.get(&listener.stream(""), None).await;
    assert_eq!(whole.headers()["accept-ranges"], "bytes");
    assert!(is_original(&sound(whole).await));

    let part = listener.get(&listener.stream(""), Some("bytes=0-3")).await;
    assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
    assert!(is_original(&sound(part).await));

    for query in ["?bitrate=64", "?offset_ms=500"] {
        let refused = listener.get(&listener.stream(query), None).await;
        assert_eq!(
            refused.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{query}"
        );
    }
}

/// A silent request is encoded as the profile says, and its encode is the one
/// an explicit request for the same pair is answered from: a profile adds no
/// second copy to the cache.
#[tokio::test]
async fn a_profile_fills_a_silent_stream_and_shares_the_cache() {
    let listener = Listener::new().await;
    listener.set_default("opus", 64).await;

    assert!(is_opus(
        &sound(listener.get(&listener.stream(""), None).await).await
    ));
    wait_for_cache_file(&listener.config.transcode_cache_dir, "opus").await;

    let cached = listener
        .get(
            &listener.stream("?format=opus&bitrate=64"),
            Some("bytes=0-3"),
        )
        .await;
    assert_eq!(cached.status(), StatusCode::PARTIAL_CONTENT);
    assert!(is_opus(&sound(cached).await));
}

/// A seek names no format, so a seek inside a profile's transcode stays under
/// the profile, where the same request without one is refused.
#[tokio::test]
async fn a_seek_stays_under_the_profile() {
    let listener = Listener::new().await;
    listener.set_default("opus", 64).await;

    let seek = listener.get(&listener.stream("?offset_ms=500"), None).await;
    assert!(is_opus(&sound(seek).await));
}

/// The frozen half of the contract, on this surface: what a request names, it
/// gets, and the profile lends it nothing.
#[tokio::test]
async fn an_explicit_request_beats_a_profile() {
    let listener = Listener::new().await;
    listener.set_default("opus", 64).await;

    let raw = listener.get(&listener.stream("?format=raw"), None).await;
    assert!(is_original(&sound(raw).await));

    let mp3 = listener.get(&listener.stream("?format=mp3"), None).await;
    let (kind, _) = sound(mp3).await;
    assert_eq!(kind, "audio/mpeg");

    // A bit rate alone has always been refused, because it asks for a transcode
    // without saying of what. The profile does not get to answer that.
    let bitrate = listener.get(&listener.stream("?bitrate=64"), None).await;
    assert_eq!(bitrate.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// A ceiling: the fixture is 128 kbit/s, 16 bits at 8 kHz, so a profile of 128 is
/// already met and one of 127 is not.
#[tokio::test]
async fn a_source_within_the_ceiling_stays_the_original() {
    let listener = Listener::new().await;
    listener.set_default("opus", 128).await;
    assert!(is_original(
        &sound(listener.get(&listener.stream(""), None).await).await
    ));

    listener.set_default("opus", 127).await;
    assert!(is_opus(
        &sound(listener.get(&listener.stream(""), None).await).await
    ));
}

/// The native API has no client name, so an override is not its to read.
#[tokio::test]
async fn an_override_alone_does_not_reach_the_native_route() {
    let listener = Listener::new().await;
    listener
        .state
        .services
        .set_transcoding_profile(listener.owner, Some("Symfonium"), "opus", Some(64))
        .await
        .unwrap();

    assert!(is_original(
        &sound(listener.get(&listener.stream(""), None).await).await
    ));
}

/// A ticket's URL is played by a media element, which seeks by byte range; a
/// download is the file. Neither is a profile's to change.
#[tokio::test]
async fn tickets_and_downloads_keep_the_original() {
    let listener = Listener::new().await;
    listener.set_default("opus", 64).await;
    let ticket = listener.ticket().await;

    let played = listener.get(&ticket, None).await;
    assert_eq!(played.headers()["accept-ranges"], "bytes");
    assert!(is_original(&sound(played).await));

    let seeked = listener.get(&ticket, Some("bytes=4-7")).await;
    assert_eq!(seeked.status(), StatusCode::PARTIAL_CONTENT);

    let saved = listener.get(&format!("{ticket}?download=1"), None).await;
    assert!(saved.headers().contains_key("content-disposition"));
    assert!(is_original(&sound(saved).await));
}

/// A profile's transcode takes the slots an asked-for one takes, and past them
/// is refused rather than sent the original: that would replace a bounded cost
/// in CPU with an unbounded one over the link the profile was set for.
#[tokio::test]
async fn a_profiles_transcode_is_refused_past_the_ceilings() {
    let listener = Listener::new().await;
    listener.set_default("mp3", 64).await;
    assert_eq!(listener.config.transcode_per_user_limit, 1);

    // Held unread, so its encode keeps the account's one slot.
    let holding = listener
        .get(
            &format!("/api/v2/tracks/{}/stream", listener.tracks[1]),
            None,
        )
        .await;
    assert_eq!(holding.status(), StatusCode::OK);
    assert_eq!(holding.headers()["content-type"], "audio/mpeg");

    let refused = listener.get(&listener.stream(""), None).await;
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(refused.headers().contains_key("retry-after"));
    drop(holding);
}
