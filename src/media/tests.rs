//! Unit tests of the media service: byte ranges, media paths, the transcode
//! cache, and what FFmpeg may open.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use dashmap::DashMap;
use tokio::sync::{Mutex, Semaphore};

use super::{
    attachment, secure_track_path,
    stream::{parse_range, starts_at_the_first_byte},
    transcode::{cache_key, prune_cache},
    MediaInner, MediaService, OutputFormat,
};

/// A download's name in both forms, whatever it holds.
///
/// The fallback is what an old browser builds a path from, so a quote, a
/// backslash, a separator or anything outside ASCII must not survive in
/// it; `filename*` must carry the real name, accents and all.
#[test]
fn a_download_name_is_safe_in_both_forms() {
    let header = attachment("Björk - Hyperballad \"live\" 1/2\\3.flac");
    let header = header.to_str().unwrap();
    assert_eq!(
        header,
        "attachment; filename=\"Bj_rk - Hyperballad _live_ 1_2_3.flac\"; \
         filename*=UTF-8''Bj%C3%B6rk%20-%20Hyperballad%20%22live%22%201_2_3.flac"
    );
    // A control character is no part of a name.
    let header = attachment("a\nb.flac");
    assert!(
        header
            .to_str()
            .unwrap()
            .starts_with("attachment; filename=\"a_b.flac\""),
        "{header:?}"
    );
}

#[test]
fn ranges_are_bounded_and_validated() {
    assert_eq!(parse_range("bytes=2-5", 10), Some((2, 5)));
    assert_eq!(parse_range("bytes=8-", 10), Some((8, 9)));
    assert_eq!(parse_range("bytes=-3", 10), Some((7, 9)));
    assert_eq!(parse_range("bytes=10-", 10), None);
    assert_eq!(parse_range("bytes=5-2", 10), None);
    assert_eq!(parse_range("bytes=0-1,4-5", 10), None);
    // Whitespace is tolerated on both bounds, so that a range the cold
    // transcode path accepts is still satisfiable from the cache.
    assert_eq!(parse_range("bytes= 0- ", 10), Some((0, 9)));
    assert_eq!(parse_range("bytes= 2 - 5 ", 10), Some((2, 5)));
}

#[test]
fn a_range_from_the_first_byte_opens_a_resource() {
    // How a browser opens an audio element, and how iOS probes one.
    assert!(starts_at_the_first_byte("bytes=0-"));
    assert!(starts_at_the_first_byte("bytes=0-1"));
    assert!(starts_at_the_first_byte("bytes=0-1023"));
    assert!(starts_at_the_first_byte("bytes= 0- "));
    // Starting anywhere else is a seek, and has no meaning yet.
    assert!(!starts_at_the_first_byte("bytes=1-"));
    assert!(!starts_at_the_first_byte("bytes=19924-429882"));
    assert!(!starts_at_the_first_byte("bytes=-3"));
    // One body cannot answer several ranges.
    assert!(!starts_at_the_first_byte("bytes=0-1,4-5"));
    assert!(!starts_at_the_first_byte("items=0-"));
    assert!(!starts_at_the_first_byte("bytes=00x-1"));
    // A malformed end is not an open either.
    assert!(!starts_at_the_first_byte("bytes=0-1-2"));
    assert!(!starts_at_the_first_byte("bytes=0-invalid"));
}

#[tokio::test]
async fn media_paths_reject_parent_components_and_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("safe.wav"), b"audio").unwrap();
    let none = crate::link_roots::LinkRoots::default();
    let policy = crate::link_roots::LinkPolicy::default();
    assert!(secure_track_path(&root, "safe.wav", &none, &policy)
        .await
        .is_ok());
    assert!(secure_track_path(&root, "../safe.wav", &none, &policy)
        .await
        .is_err());

    let link = root.join("linked.wav");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(root.join("safe.wav"), &link).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(root.join("safe.wav"), &link).is_ok();
    if linked {
        assert!(secure_track_path(&root, "linked.wav", &none, &policy)
            .await
            .is_err());
    }
}

/// A stored path through a link is accepted only when its library follows
/// links and it resolves, now, into an allowed folder outside the root
/// and into no other library (RFC-021, links outside the root): not into
/// the root itself, not anywhere else, not into another library's root
/// inside the allowed folder, not once the folder is no longer allowed,
/// and not once the library stopped following links.
#[tokio::test]
async fn a_link_is_followed_into_an_allowed_folder_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let farm = temp.path().join("farm");
    let neighbour = farm.join("neighbour");
    let elsewhere = temp.path().join("elsewhere");
    for folder in [&root, &farm, &neighbour, &elsewhere] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(farm.join("kept.wav"), b"audio").unwrap();
    std::fs::write(neighbour.join("theirs.wav"), b"audio").unwrap();
    std::fs::write(elsewhere.join("secret.wav"), b"audio").unwrap();
    std::fs::write(root.join("own.wav"), b"audio").unwrap();
    #[cfg(unix)]
    let make = |target: &std::path::Path, link: &std::path::Path| {
        std::os::unix::fs::symlink(target, link).is_ok()
    };
    #[cfg(windows)]
    let make = |target: &std::path::Path, link: &std::path::Path| {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    };
    if !(make(&farm.join("kept.wav"), &root.join("farmed.wav"))
        && make(&neighbour.join("theirs.wav"), &root.join("crossing.wav"))
        && make(&elsewhere.join("secret.wav"), &root.join("stolen.wav"))
        && make(&root.join("own.wav"), &root.join("mine.wav")))
    {
        eprintln!("skipped: this system will not let the test make a symbolic link");
        return;
    }
    let allowed = crate::link_roots::LinkRoots::new(vec![farm.clone()]);
    let following = crate::link_roots::LinkPolicy {
        follow_links: true,
        other_libraries: vec![neighbour.clone()],
    };
    assert!(secure_track_path(&root, "farmed.wav", &allowed, &following)
        .await
        .is_ok());
    assert!(
        secure_track_path(&root, "crossing.wav", &allowed, &following)
            .await
            .is_err(),
        "another library's file is never reached through a link"
    );
    assert!(secure_track_path(&root, "stolen.wav", &allowed, &following)
        .await
        .is_err());
    assert!(
        secure_track_path(&root, "mine.wav", &allowed, &following)
            .await
            .is_err(),
        "a link inside the root is stored by its target, never by itself"
    );
    let none = crate::link_roots::LinkRoots::default();
    assert!(secure_track_path(&root, "farmed.wav", &none, &following)
        .await
        .is_err());
    let not_following = crate::link_roots::LinkPolicy {
        follow_links: false,
        ..following
    };
    assert!(
        secure_track_path(&root, "farmed.wav", &allowed, &not_following)
            .await
            .is_err(),
        "a library that does not follow links reads no link, allowlist or not"
    );
}

#[tokio::test]
async fn cache_pruning_removes_the_least_recently_used_file() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("old.mp3");
    let recent = temp.path().join("recent.mp3");
    std::fs::write(&old, b"1234").unwrap();
    std::fs::write(&recent, b"5678").unwrap();
    let access = DashMap::new();
    access.insert(old.clone(), SystemTime::UNIX_EPOCH);
    access.insert(recent.clone(), SystemTime::now());
    let inner = MediaInner {
        ffmpeg: PathBuf::from("ffmpeg"),
        ffprobe: PathBuf::from("ffprobe"),
        cache_dir: temp.path().to_path_buf(),
        cache_max_bytes: 4,
        global: Arc::new(Semaphore::new(1)),
        global_limit: 1,
        per_user_limit: 1,
        users: DashMap::new(),
        cache_locks: DashMap::new(),
        cache_access: access,
        pending_fills: dashmap::DashSet::new(),
        active_transcodes: std::sync::atomic::AtomicUsize::new(0),
        transcoding_available: true,
        ffmpeg_digest: String::new(),
        link_roots: crate::link_roots::LinkRoots::default(),
        subsonic_replay_gain: crate::replay_gain::ReplayGainMode::Off,
    };
    prune_cache(&inner).await;
    assert!(!old.exists());
    assert!(recent.exists());
}

/// The fill behind a seek neither waits on the cache lock nor gives up on
/// it. The play a seek abandons still holds that lock for the moment it
/// takes to notice its consumer is gone, and the seek lands inside that
/// moment. No integration test can arrange that instant; a test inside the
/// module can, by holding the lock itself the way the dying play does.
#[tokio::test]
async fn a_seek_fills_the_cache_while_the_abandoned_play_still_holds_its_lock() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::Config::for_data_dir(temp.path().join("data"));
    config.transcode_global_limit = 4;
    let media = MediaService::initialize(&config).await.unwrap();
    let source = temp.path().join("track.wav");
    silent_wav(&source, 2);

    let key = cache_key("abandoned", OutputFormat::Mp3, 96, None);
    let cache_path = media.inner.cache_dir.join(format!("{key}.mp3"));
    let lock = media
        .inner
        .cache_locks
        .entry(key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _held = lock.lock_owned().await;

    media.fill_cache_behind_a_seek(
        source,
        OutputFormat::Mp3,
        96,
        None,
        key,
        cache_path.clone(),
        Duration::from_secs(60),
    );
    assert_eq!(
        media.active_transcodes(),
        1,
        "the fill must start while the lock is held"
    );
    for _ in 0..3_000 {
        if media.active_transcodes() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        media.active_transcodes(),
        0,
        "the fill must not wait for the lock"
    );
    assert!(
        cache_path.exists(),
        "the fill must commit the whole transcode"
    );
}

/// A fill has no listener who can leave, so nothing else ends an encode
/// that never finishes. Past its deadline FFmpeg is killed, and neither a
/// cache file, nor a staging file, nor the key's claim is left behind —
/// the slot and the key are free for the next seek.
#[tokio::test]
async fn a_cache_fill_past_its_deadline_is_killed_and_leaves_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::Config::for_data_dir(temp.path().join("data"));
    config.transcode_global_limit = 4;
    let media = MediaService::initialize(&config).await.unwrap();
    let source = temp.path().join("track.wav");
    silent_wav(&source, 30);

    let key = cache_key("overdue", OutputFormat::Mp3, 96, None);
    let cache_path = media.inner.cache_dir.join(format!("{key}.mp3"));
    // A millisecond: less than FFmpeg takes to start, let alone to finish.
    media.fill_cache_behind_a_seek(
        source,
        OutputFormat::Mp3,
        96,
        None,
        key,
        cache_path.clone(),
        Duration::from_millis(1),
    );
    assert_eq!(media.active_transcodes(), 1);
    for _ in 0..3_000 {
        if media.active_transcodes() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        media.active_transcodes(),
        0,
        "an overdue fill must give its slot back"
    );
    assert!(!cache_path.exists(), "an overdue fill must not commit");
    let staged = std::fs::read_dir(&media.inner.cache_dir)
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".part-"));
    assert!(!staged, "an overdue fill must remove its staging file");
    assert!(
        media.inner.pending_fills.is_empty(),
        "an overdue fill must free its key for the next seek"
    );
}

/// A listener on loopback that counts the connections it is offered and
/// closes each at once: what "nothing was fetched" is measured with.
pub(crate) async fn counting_listener() -> (
    std::net::SocketAddr,
    Arc<std::sync::atomic::AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&count);
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            drop(socket);
        }
    });
    (address, count, task)
}

fn a_media_service(dir: &std::path::Path) -> MediaService {
    MediaService {
        inner: Arc::new(MediaInner {
            ffmpeg: PathBuf::from("ffmpeg"),
            ffprobe: PathBuf::from("ffprobe"),
            cache_dir: dir.to_path_buf(),
            cache_max_bytes: 0,
            global: Arc::new(Semaphore::new(1)),
            global_limit: 1,
            per_user_limit: 1,
            users: DashMap::new(),
            cache_locks: DashMap::new(),
            cache_access: DashMap::new(),
            pending_fills: dashmap::DashSet::new(),
            active_transcodes: std::sync::atomic::AtomicUsize::new(0),
            transcoding_available: true,
            ffmpeg_digest: String::new(),
            link_roots: crate::link_roots::LinkRoots::default(),
            subsonic_replay_gain: crate::replay_gain::ReplayGainMode::Off,
        }),
    }
}

/// Runs the transcode FFmpeg would be given for `source`, to nowhere, and
/// answers whether it succeeded.
async fn transcode(media: &MediaService, source: &std::path::Path) -> bool {
    let mut command = media
        .transcode_command(source, OutputFormat::Mp3, 128, None, 0)
        .unwrap();
    command
        .arg("pipe:1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(60), command.status())
        .await
        .expect("ffmpeg must not hang on an input it refuses")
        .expect("ffmpeg must be on PATH")
        .success()
}

/// FFmpeg opens files and nothing else, whatever the input names
/// (RFC-023 decision 2).
///
/// Three inputs, one property:
///
/// - **A path that is a URL.** Without the whitelist FFmpeg connects to
///   it; this is the case that tells the whitelist from its absence.
/// - **A playlist that names one.** An HLS playlist pointing at the same
///   listener. FFmpeg 8 already refuses `http` from inside a file opened
///   through `file` (its default nested list is `file,crypto,data`), so
///   this one holds on that build either way: it is here so the property
///   stays tested on a build whose default is wider.
/// - **A `concat:` URL** of two real files, which the default decodes.
///
/// And one control: the same command plays a real file, so the whitelist
/// is proved not to be in the way of the one protocol the server uses —
/// on Windows too, where a path starts with a drive letter and a colon.
#[tokio::test]
async fn ffmpeg_opens_files_and_nothing_else() {
    let temp = tempfile::tempdir().unwrap();
    let media = a_media_service(temp.path());
    let real = temp.path().join("real.wav");
    silent_wav(&real, 1);
    assert!(
        transcode(&media, &real).await,
        "a real file must still play"
    );

    let (address, connections, listener) = counting_listener().await;
    let url = format!("http://{address}/track.mp3");
    assert!(!transcode(&media, std::path::Path::new(&url)).await);

    let playlist = temp.path().join("trap.m3u8");
    std::fs::write(
        &playlist,
        format!(
            "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nhttp://{address}/segment.mp3\n#EXT-X-ENDLIST\n"
        ),
    )
    .unwrap();
    assert!(!transcode(&media, &playlist).await);

    let concat = format!("concat:{}|{}", real.display(), real.display());
    assert!(
        !transcode(&media, std::path::Path::new(&concat)).await,
        "a concat: input must be refused"
    );

    // Connections are counted as they are accepted; give the last one
    // the moment it needs to land before reading the count.
    tokio::time::sleep(Duration::from_millis(200)).await;
    listener.abort();
    assert_eq!(
        connections.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "ffmpeg must not have contacted anything"
    );
}

/// Silence at 8 kHz, as long as asked.
pub(crate) fn silent_wav(path: &std::path::Path, seconds: u32) {
    let generated = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=8000:cl=mono",
            "-t",
        ])
        .arg(seconds.to_string())
        .arg(path)
        .status()
        .expect("ffmpeg must be on PATH");
    assert!(generated.success());
}
