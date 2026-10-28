//! `Config::for_data_dir`: the configuration the test suite boots under.

use std::{path::PathBuf, time::Duration};

use super::pid::default_pid_specs;
use super::scrobbling::test_destinations;
use super::{
    CanvasLimits, Config, LibraryEventRetention, LyricsSettings, MetadataSettings, PodcastSettings,
    RemoteArtworkSettings, ScrobbleLimits, UploadLimits,
};

impl Config {
    pub fn for_data_dir(data_dir: PathBuf) -> Self {
        let transcode_cache_dir = data_dir.join("transcode-cache");
        let canvas_dir_for_tests = data_dir.join("canvas");
        let podcast_dir_for_tests = data_dir.join("podcasts");
        Self {
            bind_addr: "127.0.0.1:0".parse().expect("literal socket address"),
            public_url: Some("http://waveflow.test".to_owned()),
            request_timeout: Duration::from_secs(30),
            database_path: data_dir.join("waveflow.db"),
            instance_key_path: data_dir.join("instance.key"),
            artwork_dir: data_dir.join("artwork"),
            data_dir,
            db_max_connections: 4,
            // Generous where production is tight, and for a reason production
            // does not have: a test target boots a whole server per test, so
            // several cold databases are created at once on a machine already
            // compiling. Three targets — uploads, media, scrobbling — have
            // each failed this acquire on the Windows runner at different
            // times, naming whichever test happened to be unlucky. Raising
            // the budget removes a lottery; it hides no wrong answer, because
            // the only thing this timeout can produce is a refusal to start.
            db_acquire_timeout: Duration::from_secs(30),
            sqlite_busy_timeout: Duration::from_secs(5),
            access_token_ttl: Duration::from_secs(15 * 60),
            stream_ticket_ttl: Duration::from_secs(60 * 60),
            refresh_token_ttl: Duration::from_secs(30 * 24 * 60 * 60),
            // A year, as production has it: a suite that minted tokens with no
            // expiry could not tell a bounded one from an unbounded one.
            api_token_ttl: Some(Duration::from_secs(365 * 24 * 60 * 60)),
            scan_interval: None,
            scan_parallelism: 2,
            // Off, like the periodic scan: a test that writes a file and then
            // scans must not find a second scan racing its own. The watcher's
            // tests start one themselves.
            watch: false,
            watch_delay: Duration::from_millis(200),
            unavailable_retention: None,
            // Production's default: the suite purges the way a server does.
            purge_grace: Duration::from_secs(168 * 60 * 60),
            link_roots: Vec::new(),
            subsonic_replay_gain: crate::media::replay_gain::ReplayGainMode::Off,
            // Production's default, so the suite answers DSub as a deployment
            // does and every other client in full.
            subsonic_legacy_clients: super::from_env::default_legacy_clients(),
            ffmpeg_path: PathBuf::from("ffmpeg"),
            ffprobe_path: PathBuf::from("ffprobe"),
            transcode_cache_dir,
            transcode_cache_max_bytes: 128 * 1024 * 1024,
            transcode_global_limit: 2,
            transcode_per_user_limit: 1,
            quality_parallelism: 1,
            // Small enough that a test can reach every bound without writing a
            // gigabyte, and shaped like production rather than unlimited: a
            // suite that never meets a limit never proves one exists.
            uploads: UploadLimits {
                max_file_bytes: 1024 * 1024,
                library_quota_bytes: 4 * 1024 * 1024,
                chunk_bytes: 64 * 1024,
                batch_limit: 8,
                sessions_per_user: 2,
                session_ttl: Duration::from_secs(3600),
            },
            // Small enough that a test can write past the floor without
            // writing ten thousand rows, and shaped like production rather
            // than unlimited.
            library_event_retention: LibraryEventRetention {
                days: 30,
                min_events: 4,
            },
            canvas_dir: canvas_dir_for_tests,
            // Same reasoning as the upload limits above: small enough that a
            // test can reach every bound, shaped like production rather than
            // unlimited.
            canvas: CanvasLimits {
                max_bytes: 256 * 1024,
                max_duration_secs: 15,
                library_quota_bytes: 1024 * 1024,
            },
            // Same reasoning again: small enough that a test can exhaust the
            // attempts without waiting out eight doublings, shaped like
            // production rather than unlimited. The interval is never reached —
            // `spawn_scrobble_drain` is started by `main`, and tests run the
            // pass themselves.
            scrobbling: ScrobbleLimits {
                drain_interval: Duration::from_secs(60),
                // Long enough that no test double ever meets it by accident;
                // the one test that means to meet it shortens this first.
                request_timeout: Duration::from_secs(30),
                max_attempts: 3,
                stale_after: Duration::from_secs(3600),
                batch: 8,
                retention_days: 30,
            },
            // One instance, declared and unreachable.
            //
            // **Declared**, because a member may only link a name the operator
            // published, and a suite with an empty list could link nothing and
            // would exercise none of the queue.
            //
            // **Unreachable**, because the suite must never reach the real
            // ListenBrainz: port 1 on loopback is a port nothing binds. Every
            // test that drives the queue replaces the adapter with a double
            // through `register_scrobble_target`, and the handful that mean to
            // exercise a real request point this at a server they started
            // themselves — over plain HTTP on loopback, which is why the
            // plaintext escape is on here and off in production.
            destinations: test_destinations(),
            // No application, so the suite never reaches the real Last.fm and
            // the destination declared above stays adapterless — which is what
            // two tests here stand on.
            lastfm: None,
            outbound_allow_plaintext: true,
            // Off, as production defaults: the suite never reaches a real
            // provider, and the targets that exercise enrichment build their
            // own engine against a listener of their own.
            metadata: MetadataSettings::off(),
            // Off as well, and for the same reason: the targets that exercise
            // LRCLIB build their own lookup against a listener of their own.
            lyrics: LyricsSettings::off(),
            // Off, as production defaults, and small enough that a test can
            // reach every bound. The podcast targets switch it on and point
            // it at a listener of their own on loopback, which is the one
            // reason the address policy here admits loopback.
            podcasts: PodcastSettings {
                enabled: false,
                refresh_interval: None,
                allow_http: false,
                max_feed_bytes: 256 * 1024,
                max_episode_bytes: 1024 * 1024,
                quota_bytes: 4 * 1024 * 1024,
                feed_timeout: Duration::from_secs(10),
                download_timeout: Duration::from_secs(30),
                dir: podcast_dir_for_tests,
                addresses: crate::remote::portrait::AddressPolicy::AllowLoopback,
            },
            // On, as production defaults, and admitting loopback for the same
            // reason the podcast policy above does: the image hosts the suite
            // serves are listeners of its own. No test reaches this unless it
            // names an image, so nothing leaves the machine.
            remote_artwork: RemoteArtworkSettings {
                enabled: true,
                timeout: Duration::from_secs(10),
                addresses: crate::remote::portrait::AddressPolicy::AllowLoopback,
            },
            allowed_origins: Vec::new(),
            trusted_proxies: Vec::new(),
            // Drawn at boot and written to the data directory, as production
            // does by default: the setup tests read it back from there.
            setup_token: None,
            migration_backups: 3,
            // The real defaults, so the whole test suite exercises the specs
            // production runs under rather than a simplified stand-in.
            pid: default_pid_specs(),
        }
    }
}
