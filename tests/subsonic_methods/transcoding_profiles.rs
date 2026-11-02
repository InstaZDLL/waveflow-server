//! Transcoding profiles through the façade (RFC-027). The contract is frozen,
//! so most of this is what a profile must not do: a request that names a format
//! or a cap is answered exactly as if the account had none.

use super::*;

struct Listener {
    _temp: tempfile::TempDir,
    state: waveflow_server::AppState,
    router: axum::Router,
    owner: Uuid,
    api_key: String,
    song: Uuid,
}

impl Listener {
    async fn new() -> Self {
        let (temp, config, state) = test_app().await;
        let (owner, api_key) = subsonic_account(&state, "profile-listener").await;
        let music = config.data_dir.join("profile-music");
        std::fs::create_dir_all(&music).unwrap();
        write_test_wav_of_len(&music.join("Profile.wav"), 8_000 * 3);
        let root = std::fs::canonicalize(&music).unwrap();
        let library = state
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
            waveflow_server::storage::catalog::LibraryRecord {
                id: library,
                name: "Profiles".into(),
                root_path: root,
            },
        )
        .await;
        let song = state
            .services
            .catalog_snapshot(owner, &[])
            .await
            .unwrap()
            .songs[0]
            .id;
        let router = waveflow_server::app(&config, state.clone());
        Self {
            _temp: temp,
            state,
            router,
            owner,
            api_key,
            song,
        }
    }

    async fn set(&self, client: Option<&str>, format: &str, bitrate: Option<u32>) {
        self.state
            .services
            .set_transcoding_profile(self.owner, client, format, bitrate)
            .await
            .unwrap();
    }

    /// `method` for the track, as client `c`, with `extra` after it: the media
    /// type of the answer and the first bytes of it.
    async fn ask(&self, method: &str, c: &str, extra: &str) -> (String, Vec<u8>) {
        let uri = format!(
            "/rest/{method}.view?apiKey={}&id={}&v=1.16.1&c={c}{extra}",
            self.api_key, self.song
        );
        let response = self
            .router
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{extra}");
        let kind = response.headers()["content-type"]
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        (kind, bytes[..4].to_vec())
    }

    async fn stream(&self, c: &str, extra: &str) -> &'static str {
        let (kind, start) = self.ask("stream", c, extra).await;
        match (kind.as_str(), start.as_slice()) {
            ("audio/wav", b"RIFF") => "original",
            ("audio/ogg", b"OggS") => "opus",
            ("audio/mpeg", _) => "mp3",
            ("audio/aac", _) => "aac",
            _ => panic!("{extra}: {kind} {start:?}"),
        }
    }
}

/// No profile, no change: every shape of request answered what it always was.
#[tokio::test]
async fn with_no_profile_every_request_is_answered_as_before() {
    let listener = Listener::new().await;
    for (extra, expected) in [
        ("", "original"),
        ("&maxBitRate=0", "original"),
        ("&format=", "original"),
        ("&format=raw", "original"),
        ("&format=flac", "original"),
        ("&format=opus", "opus"),
        ("&maxBitRate=64", "mp3"),
        ("&maxBitRate=320", "original"),
    ] {
        assert_eq!(listener.stream("golden", extra).await, expected, "{extra}");
    }
}

/// A silence, in the three spellings a client that does not ask has of it.
#[tokio::test]
async fn a_profile_fills_a_silent_request() {
    let listener = Listener::new().await;
    listener.set(None, "opus", Some(64)).await;
    for extra in ["", "&maxBitRate=0", "&format="] {
        assert_eq!(listener.stream("golden", extra).await, "opus", "{extra}");
    }
    // A seek names neither, and stays inside the profile's transcode.
    assert_eq!(listener.stream("golden", "&timeOffset=1").await, "opus");
}

/// The frozen contract: what the request names is what it gets. `format=flac`
/// included, since it named something this server cannot make, and RFC-022
/// decision 6 already says what that plays.
#[tokio::test]
async fn an_explicit_format_or_cap_beats_a_profile() {
    let listener = Listener::new().await;
    listener.set(None, "opus", Some(64)).await;
    for (extra, expected) in [
        ("&format=raw", "original"),
        ("&format=flac", "original"),
        ("&format=mp3", "mp3"),
        ("&format=aac", "aac"),
        // The cap's own rule, MP3 over it and the original under: not the
        // profile's Opus, and not the profile's ceiling either.
        ("&maxBitRate=64", "mp3"),
        ("&maxBitRate=320", "original"),
    ] {
        assert_eq!(listener.stream("golden", extra).await, expected, "{extra}");
    }
}

/// A download is the file, whatever the account prefers to listen to.
#[tokio::test]
async fn a_download_keeps_the_original() {
    let listener = Listener::new().await;
    listener.set(None, "opus", Some(64)).await;
    let (kind, start) = listener.ask("download", "golden", "").await;
    assert_eq!(kind, "audio/wav");
    assert_eq!(start, b"RIFF");
}

/// `Symfonium → Opus` beside a default of MP3 and `Feishin → original`: the
/// sentence an account-wide profile could not say.
#[tokio::test]
async fn a_client_name_overrides_the_default() {
    let listener = Listener::new().await;
    listener.set(None, "mp3", Some(64)).await;
    listener.set(Some("Symfonium"), "opus", Some(64)).await;
    listener.set(Some("Feishin"), "raw", None).await;

    assert_eq!(listener.stream("Symfonium", "").await, "opus");
    assert_eq!(listener.stream("symfonium", "").await, "opus");
    assert_eq!(listener.stream("Feishin", "").await, "original");
    assert_eq!(listener.stream("golden", "").await, "mp3");
    // An explicit request beats an override as it beats a default.
    assert_eq!(
        listener.stream("Symfonium", "&format=raw").await,
        "original"
    );

    // Without a default, a client with no override of its own has no profile.
    listener
        .state
        .services
        .delete_transcoding_profile(listener.owner, None)
        .await
        .unwrap();
    assert_eq!(listener.stream("golden", "").await, "original");
    assert_eq!(listener.stream("Symfonium", "").await, "opus");
}

/// A ceiling, on the façade too: 128 kbit/s under a profile of 128 is the file.
#[tokio::test]
async fn a_source_within_the_ceiling_stays_the_original() {
    let listener = Listener::new().await;
    listener.set(None, "opus", Some(128)).await;
    assert_eq!(listener.stream("golden", "").await, "original");
    listener.set(None, "opus", Some(127)).await;
    assert_eq!(listener.stream("golden", "").await, "opus");
}

/// The section offers the clients a profile reaches: one that streamed without
/// asking. A client that named a format, or a download, is not one of them.
#[tokio::test]
async fn the_names_offered_are_those_of_silent_streams() {
    let listener = Listener::new().await;
    listener.stream("Quiet", "").await;
    listener.stream("Asking", "&format=mp3").await;
    listener.stream("Capping", "&maxBitRate=64").await;
    listener.ask("download", "Saving", "").await;

    let profiles = listener
        .state
        .services
        .transcoding_profiles(listener.owner)
        .await
        .unwrap();
    assert_eq!(profiles.seen_clients, ["Quiet"]);
}
