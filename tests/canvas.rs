//! The loop a member attaches to a track. RFC-009.
//!
//! No routes yet: this drives `DomainServices` directly, which is the whole of
//! what the domain half of the canvas is. What the store does with bytes, what
//! it charges for them, and what it refuses are decided here rather than at the
//! surface, so they are tested here.

use waveflow_server::authentication::now_ms;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::config::CanvasLimits;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

/// An app whose canvas limits this test chose.
///
/// Tuned before `initialize`, never after: `DomainServices` copies these values
/// out of `Config` when it is built, so a test that raises a ceiling on the
/// returned `Config` silently exercises the old one.
async fn canvas_app(
    tune: impl FnOnce(&mut CanvasLimits),
) -> (
    tempfile::TempDir,
    waveflow_server::Config,
    waveflow_server::AppState,
) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    tune(&mut config.canvas);
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
}

struct Fixture {
    owner: uuid::Uuid,
    library: uuid::Uuid,
    tracks: Vec<uuid::Uuid>,
}

/// An owner, an open library, and `count` scanned tracks in it.
async fn fixture(
    config: &waveflow_server::Config,
    state: &waveflow_server::AppState,
    name: &str,
    count: usize,
) -> Fixture {
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account(name, &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join(name);
    std::fs::create_dir_all(&music).unwrap();
    for index in 0..count {
        write_test_wav(&music.join(format!("Track {index}.wav")));
    }
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(owner, name, &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    run_scan(
        state,
        owner,
        LibraryRecord {
            id: library,
            name: name.into(),
            root_path: root,
        },
    )
    .await;
    // A canvas spends the operator's disk, so it follows the deposit rules: a
    // server that was merely upgraded must not have become one.
    state
        .db
        .set_library_accepts_uploads(owner, library, true, now_ms())
        .await
        .unwrap();
    let mut tracks: Vec<uuid::Uuid> = state
        .db
        .list_tracks_for_user(owner, library)
        .await
        .unwrap()
        .into_iter()
        .map(|track| track.id)
        .collect();
    tracks.sort();
    assert_eq!(tracks.len(), count);
    Fixture {
        owner,
        library,
        tracks,
    }
}

/// A real container, because the server reads the bytes rather than the name.
///
/// `mpeg4` rather than `libx264` or `libvpx`: it is built into FFmpeg itself,
/// so this fixture does not depend on which encoders a machine happens to ship.
fn canvas_bytes(dir: &std::path::Path, name: &str, seconds: &str, colour: &str) -> Vec<u8> {
    let path = dir.join(name);
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={colour}:s=64x64:r=10:d={seconds}"),
            "-c:v",
            "mpeg4",
        ])
        .arg(&path)
        .output()
        .expect("ffmpeg must be on PATH");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::read(&path).unwrap()
}

/// An mp4 carrying sound and no picture. A canvas without a video stream is not
/// a canvas, whatever else it is.
fn soundtrack_only(dir: &std::path::Path) -> Vec<u8> {
    let path = dir.join("audio-only.mp4");
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:a",
            "aac",
        ])
        .arg(&path)
        .output()
        .expect("ffmpeg must be on PATH");
    assert!(output.status.success());
    std::fs::read(&path).unwrap()
}

fn stored_files(config: &waveflow_server::Config) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(&config.canvas_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_canvas_is_stored_once_shared_by_reference_and_erased_with_the_last_link() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-store", 2).await;
    let bytes = canvas_bytes(temp.path(), "loop.mp4", "1", "black");

    let placed = state
        .services
        .place_canvas(fixture.owner, fixture.tracks[0], &bytes)
        .await
        .unwrap();
    assert_eq!(placed.format, "mp4");
    assert_eq!(placed.byte_size, bytes.len() as i64);
    assert_eq!(stored_files(&config), vec![placed.file_name()]);

    // The alias resolves the link, and the fingerprint resolves the content.
    // Both answer the same blob to somebody entitled to the track.
    assert_eq!(
        state
            .services
            .canvas_for_track(fixture.owner, fixture.tracks[0])
            .await
            .unwrap(),
        Some(placed.clone())
    );
    assert_eq!(
        state
            .services
            .canvas_for_user(fixture.owner, &placed.hash)
            .await
            .unwrap(),
        Some(placed.clone())
    );

    // The twelve tracks of an album that share one loop share the bytes. Here
    // there are two, and one file.
    let second = state
        .services
        .place_canvas(fixture.owner, fixture.tracks[1], &bytes)
        .await
        .unwrap();
    assert_eq!(second, placed);
    assert_eq!(stored_files(&config), vec![placed.file_name()]);

    // Removing one link removes a row, not a blob: the other track still names
    // it, and a file erased here would be a dead link for somebody else.
    state
        .services
        .remove_canvas(fixture.owner, fixture.tracks[0])
        .await
        .unwrap();
    assert_eq!(
        state
            .services
            .canvas_for_track(fixture.owner, fixture.tracks[0])
            .await
            .unwrap(),
        None
    );
    assert_eq!(stored_files(&config), vec![placed.file_name()]);
    assert!(state
        .services
        .canvas_for_user(fixture.owner, &placed.hash)
        .await
        .unwrap()
        .is_some());

    // The last one takes the bytes with it.
    state
        .services
        .remove_canvas(fixture.owner, fixture.tracks[1])
        .await
        .unwrap();
    assert!(stored_files(&config).is_empty());
    assert_eq!(
        state
            .services
            .canvas_for_user(fixture.owner, &placed.hash)
            .await
            .unwrap(),
        None
    );

    // Removing what is not there is not a silent success: there was nothing to
    // take away, and the caller asked to take something away.
    assert!(matches!(
        state
            .services
            .remove_canvas(fixture.owner, fixture.tracks[1])
            .await,
        Err(waveflow_server::services::ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn replacing_a_canvas_releases_the_one_it_replaced() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-replace", 2).await;
    let first = canvas_bytes(temp.path(), "first.mp4", "1", "black");
    let second = canvas_bytes(temp.path(), "second.mp4", "1", "white");

    let first = state
        .services
        .place_canvas(fixture.owner, fixture.tracks[0], &first)
        .await
        .unwrap();
    let second = state
        .services
        .place_canvas(fixture.owner, fixture.tracks[0], &second)
        .await
        .unwrap();
    assert_ne!(first.hash, second.hash);
    // A replacement is a removal too. The loop the track used to carry lost its
    // last reference, so its row and its bytes are gone.
    assert_eq!(stored_files(&config), vec![second.file_name()]);
    assert_eq!(
        state
            .services
            .canvas_for_user(fixture.owner, &first.hash)
            .await
            .unwrap(),
        None
    );

    // Unless another track still holds it. Both tracks take the first loop,
    // then the first track is given the second one back: the blob survives the
    // replacement because track two is still naming it.
    let first_bytes = std::fs::read(temp.path().join("first.mp4")).unwrap();
    let second_bytes = std::fs::read(temp.path().join("second.mp4")).unwrap();
    for track in &fixture.tracks {
        assert_eq!(
            state
                .services
                .place_canvas(fixture.owner, *track, &first_bytes)
                .await
                .unwrap()
                .hash,
            first.hash
        );
    }
    assert_eq!(stored_files(&config), vec![first.file_name()]);

    state
        .services
        .place_canvas(fixture.owner, fixture.tracks[0], &second_bytes)
        .await
        .unwrap();
    let mut expected = vec![first.file_name(), second.file_name()];
    expected.sort();
    assert_eq!(stored_files(&config), expected);
}

#[tokio::test]
async fn a_canvas_belongs_to_the_library_and_not_to_whoever_knows_its_name() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let owner = fixture(&config, &state, "canvas-owner", 1).await;
    let stranger = fixture(&config, &state, "canvas-stranger", 1).await;
    let bytes = canvas_bytes(temp.path(), "loop.mp4", "1", "black");

    let placed = state
        .services
        .place_canvas(owner.owner, owner.tracks[0], &bytes)
        .await
        .unwrap();

    // A hash identifies content and proves nothing. Two accounts that are
    // strangers to each other can hold the same loop, so knowing the
    // fingerprint establishes no more than knowing a track id does.
    assert_eq!(
        state
            .services
            .canvas_for_user(stranger.owner, &placed.hash)
            .await
            .unwrap(),
        None
    );
    // A track of another library is missing and belongs to someone else, and
    // both answer the same thing.
    assert_eq!(
        state
            .services
            .canvas_for_track(stranger.owner, owner.tracks[0])
            .await
            .unwrap(),
        None
    );
    assert!(matches!(
        state
            .services
            .place_canvas(stranger.owner, owner.tracks[0], &bytes)
            .await,
        Err(waveflow_server::services::ServiceError::NotFound)
    ));
    assert!(matches!(
        state
            .services
            .remove_canvas(stranger.owner, owner.tracks[0])
            .await,
        Err(waveflow_server::services::ServiceError::NotFound)
    ));

    // And the stranger placing on their own track shares the blob without
    // learning anything about the neighbour who already held it.
    let theirs = state
        .services
        .place_canvas(stranger.owner, stranger.tracks[0], &bytes)
        .await
        .unwrap();
    assert_eq!(theirs.hash, placed.hash);
    assert_eq!(stored_files(&config), vec![placed.file_name()]);
}

#[tokio::test]
async fn a_closed_library_takes_no_canvas_but_still_gives_one_back() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-closed", 1).await;
    let bytes = canvas_bytes(temp.path(), "loop.mp4", "1", "black");
    state
        .services
        .place_canvas(fixture.owner, fixture.tracks[0], &bytes)
        .await
        .unwrap();

    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, false, now_ms())
        .await
        .unwrap();

    // The flag answers "may a member spend the operator's disk", so it closes
    // the door that spends it.
    assert!(matches!(
        state
            .services
            .place_canvas(fixture.owner, fixture.tracks[0], &bytes)
            .await,
        Err(waveflow_server::services::ServiceError::Forbidden)
    ));
    // Taking something away never spends it, and closing a library must not
    // strand what it already holds.
    state
        .services
        .remove_canvas(fixture.owner, fixture.tracks[0])
        .await
        .unwrap();
    assert!(stored_files(&config).is_empty());
}

#[tokio::test]
async fn what_is_not_a_short_loop_is_refused_and_leaves_nothing_behind() {
    let (temp, config, state) = canvas_app(|canvas| canvas.max_duration_secs = 2).await;
    let library = fixture(&config, &state, "canvas-refused", 1).await;

    // Bytes ffprobe cannot read at all. A verdict on the offer, not an outage.
    assert!(matches!(
        state
            .services
            .place_canvas(library.owner, library.tracks[0], b"this is not a container")
            .await,
        Err(waveflow_server::services::ServiceError::Invalid)
    ));

    // A container on the list, carrying sound and no picture.
    assert!(matches!(
        state
            .services
            .place_canvas(
                library.owner,
                library.tracks[0],
                &soundtrack_only(temp.path())
            )
            .await,
        Err(waveflow_server::services::ServiceError::Invalid)
    ));

    // A loop that is not short. Without this bound, "a short loop" becomes
    // video hosting, which is a different product with different costs.
    let long = canvas_bytes(temp.path(), "long.mp4", "5", "black");
    assert!(matches!(
        state
            .services
            .place_canvas(library.owner, library.tracks[0], &long)
            .await,
        Err(waveflow_server::services::ServiceError::Invalid)
    ));

    // Larger than the ceiling, refused before anything is written.
    let (temp2, config2, state2) = canvas_app(|canvas| {
        canvas.max_bytes = 1024;
        canvas.library_quota_bytes = 1024 * 1024;
    })
    .await;
    let small = fixture(&config2, &state2, "canvas-ceiling", 1).await;
    let ordinary = canvas_bytes(temp2.path(), "loop.mp4", "1", "black");
    assert!(ordinary.len() > 1024, "the fixture must exceed the ceiling");
    assert!(matches!(
        state2
            .services
            .place_canvas(small.owner, small.tracks[0], &ordinary)
            .await,
        Err(waveflow_server::services::ServiceError::Invalid)
    ));

    // Nothing refused leaves anything behind — no staged file, no blob, no row.
    assert!(stored_files(&config).is_empty());
    assert!(stored_files(&config2).is_empty());
    assert_eq!(
        state
            .services
            .canvas_for_track(library.owner, library.tracks[0])
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn the_quota_counts_distinct_blobs_and_refuses_the_one_that_would_cross_it() {
    let first_size = {
        let probe = tempfile::tempdir().unwrap();
        canvas_bytes(probe.path(), "probe.mp4", "1", "black").len() as i64
    };
    // Room for one canvas of that size and not two. The second placement is the
    // one that has to be refused, and a quota nothing ever meets proves nothing.
    let (temp, config, state) = canvas_app(|canvas| {
        canvas.max_bytes = first_size * 2;
        canvas.library_quota_bytes = first_size + first_size / 2;
    })
    .await;
    let fixture = fixture(&config, &state, "canvas-quota", 3).await;
    let first = canvas_bytes(temp.path(), "first.mp4", "1", "black");
    let second = canvas_bytes(temp.path(), "second.mp4", "1", "white");
    // The boundary this test is about, stated rather than assumed, and stated
    // against the limits the app actually runs under rather than against the
    // arithmetic that chose them. `first_size` came from a separate ffmpeg run,
    // so it sized the quota but it is not what the quota will charge: that is
    // `first.len()`.
    let first_used = first.len() as i64;
    let second_used = second.len() as i64;
    assert!(
        first_used <= config.canvas.library_quota_bytes,
        "the first canvas has to fit, or the refusal below proves nothing"
    );
    assert!(
        second_used <= config.canvas.max_bytes,
        "the second has to pass the per-canvas ceiling, so what stops it is the quota"
    );
    assert!(
        first_used + second_used > config.canvas.library_quota_bytes,
        "and it has to be the one that crosses"
    );

    let placed = state
        .services
        .place_canvas(fixture.owner, fixture.tracks[0], &first)
        .await
        .unwrap();

    // The same blob on another track is free: an album's shared loop is billed
    // once however many tracks name it, which is the deduplication showing up
    // in the price.
    state
        .services
        .place_canvas(fixture.owner, fixture.tracks[1], &first)
        .await
        .unwrap();
    assert_eq!(stored_files(&config), vec![placed.file_name()]);

    // A different blob is not.
    assert!(matches!(
        state
            .services
            .place_canvas(fixture.owner, fixture.tracks[2], &second)
            .await,
        Err(waveflow_server::services::ServiceError::Conflict)
    ));
    // And the refusal left no bytes: the file was written before the row, so a
    // transaction that says no has to take them back.
    assert_eq!(stored_files(&config), vec![placed.file_name()]);
}
