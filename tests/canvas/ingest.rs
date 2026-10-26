//! Taking a canvas in: stored once and shared by reference, owned by the
//! library, refused when it is not a short loop or crosses the quota, and
//! stripped of any sound.

use super::*;

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

#[tokio::test]
async fn a_canvas_is_stored_once_shared_by_reference_and_erased_with_the_last_link() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-store", 2).await;
    let bytes = canvas_bytes(temp.path(), "loop.mp4", "1", "black");

    let placed = state
        .services
        .place_canvas(fixture.owner, fixture.tracks[0], &bytes, None)
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
        .place_canvas(fixture.owner, fixture.tracks[1], &bytes, None)
        .await
        .unwrap();
    assert_eq!(second, placed);
    assert_eq!(stored_files(&config), vec![placed.file_name()]);

    // Removing one link removes a row, not a blob: the other track still names
    // it, and a file erased here would be a dead link for somebody else.
    state
        .services
        .remove_canvas(fixture.owner, fixture.tracks[0], None)
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
        .remove_canvas(fixture.owner, fixture.tracks[1], None)
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
            .remove_canvas(fixture.owner, fixture.tracks[1], None)
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
        .place_canvas(fixture.owner, fixture.tracks[0], &first, None)
        .await
        .unwrap();
    let second = state
        .services
        .place_canvas(fixture.owner, fixture.tracks[0], &second, None)
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
                .place_canvas(fixture.owner, *track, &first_bytes, None)
                .await
                .unwrap()
                .hash,
            first.hash
        );
    }
    assert_eq!(stored_files(&config), vec![first.file_name()]);

    state
        .services
        .place_canvas(fixture.owner, fixture.tracks[0], &second_bytes, None)
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
        .place_canvas(owner.owner, owner.tracks[0], &bytes, None)
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
            .place_canvas(stranger.owner, owner.tracks[0], &bytes, None)
            .await,
        Err(waveflow_server::services::ServiceError::NotFound)
    ));
    assert!(matches!(
        state
            .services
            .remove_canvas(stranger.owner, owner.tracks[0], None)
            .await,
        Err(waveflow_server::services::ServiceError::NotFound)
    ));

    // And the stranger placing on their own track shares the blob without
    // learning anything about the neighbour who already held it.
    let theirs = state
        .services
        .place_canvas(stranger.owner, stranger.tracks[0], &bytes, None)
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
        .place_canvas(fixture.owner, fixture.tracks[0], &bytes, None)
        .await
        .unwrap();

    state
        .db
        .set_library_accepts_canvas(fixture.owner, fixture.library, false, now_ms())
        .await
        .unwrap();

    // The flag answers "may a member spend the operator's disk", so it closes
    // the door that spends it.
    assert!(matches!(
        state
            .services
            .place_canvas(fixture.owner, fixture.tracks[0], &bytes, None)
            .await,
        Err(waveflow_server::services::ServiceError::Forbidden)
    ));
    // Taking something away never spends it, and closing a library must not
    // strand what it already holds.
    state
        .services
        .remove_canvas(fixture.owner, fixture.tracks[0], None)
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
            .place_canvas(
                library.owner,
                library.tracks[0],
                b"this is not a container",
                None
            )
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
                &soundtrack_only(temp.path()),
                None
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
            .place_canvas(library.owner, library.tracks[0], &long, None)
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
            .place_canvas(small.owner, small.tracks[0], &ordinary, None)
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
        .place_canvas(fixture.owner, fixture.tracks[0], &first, None)
        .await
        .unwrap();

    // The same blob on another track is free: an album's shared loop is billed
    // once however many tracks name it, which is the deduplication showing up
    // in the price.
    state
        .services
        .place_canvas(fixture.owner, fixture.tracks[1], &first, None)
        .await
        .unwrap();
    assert_eq!(stored_files(&config), vec![placed.file_name()]);

    // A different blob is not.
    assert!(matches!(
        state
            .services
            .place_canvas(fixture.owner, fixture.tracks[2], &second, None)
            .await,
        Err(waveflow_server::services::ServiceError::Conflict)
    ));
    // And the refusal left no bytes: the file was written before the row, so a
    // transaction that says no has to take them back.
    assert_eq!(stored_files(&config), vec![placed.file_name()]);
}

/// RFC-009 decision 9. A canvas plays over a track that is already playing and
/// the desktop's `<video>` carries a hard-coded `muted`, so there is no client
/// that could choose to hear this. Keeping it would spend the ceiling on bytes
/// nobody can hear.
#[tokio::test]
async fn a_loop_that_carries_sound_is_taken_without_it() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let library = fixture(&config, &state, "canvas-sound", 2).await;
    let noisy = noisy_canvas_bytes(temp.path());

    // Accepted rather than refused: an otherwise perfectly good mp4 must not be
    // rejected over a stream the sender did not necessarily choose to include.
    let placed = state
        .services
        .place_canvas(library.owner, library.tracks[0], &noisy, None)
        .await
        .unwrap();
    assert_eq!(placed.format, "mp4");
    assert_eq!(stored_files(&config), vec![placed.file_name()]);

    // And stored without it. This is the assertion the previous shape of this
    // test was missing: accepting the file was true before the strip existed.
    let stored = config.canvas_dir.join(placed.file_name());
    assert!(
        !has_audio_stream(&stored),
        "the soundtrack must not be in the store"
    );
    assert_eq!(
        placed.byte_size,
        std::fs::metadata(&stored).unwrap().len() as i64,
        "the quota is charged what the store holds"
    );
    assert!(
        placed.byte_size < noisy.len() as i64,
        "and what it holds is smaller than what arrived"
    );

    // The strip has to be deterministic or the deduplication of decision 1
    // quietly stops working: the same loop offered twice would land as two
    // blobs and be charged twice.
    let again = state
        .services
        .place_canvas(library.owner, library.tracks[1], &noisy, None)
        .await
        .unwrap();
    assert_eq!(again, placed, "the same source must give the same blob");
    assert_eq!(stored_files(&config), vec![placed.file_name()]);

    // And the quota charges what the store holds, not what arrived. Asserting
    // `byte_size` above says what the row carries; this says what the quota
    // spends, which is the number that can silently go wrong.
    //
    // It cannot be shown by squeezing the quota to the stripped size, because
    // `validate_canvas` refuses a per-canvas ceiling above the library quota
    // and the ceiling has to admit the *arriving* bytes. So the room is filled
    // instead: a first canvas takes all but exactly the stripped size, and the
    // noisy one fits only if it is charged what it became.
    // A test pattern rather than a flat colour: flat black compresses to about
    // a kilobyte a second, far too little to fill a quota against an 11 KB
    // arrival, and a filler long enough to get there would trip the duration
    // ceiling instead.
    let filler = detailed_canvas_bytes(temp.path());
    let (_temp2, config2, state2) = canvas_app(|canvas| {
        // The ceiling has to admit both arrivals — the filler and the noisy
        // one — while staying under the quota, which is what makes this a
        // configuration the server would actually start with.
        canvas.max_bytes = filler.len().max(noisy.len()) as i64;
        canvas.library_quota_bytes = filler.len() as i64 + placed.byte_size;
    })
    .await;
    assert!(
        filler.len() as i64 + placed.byte_size >= filler.len().max(noisy.len()) as i64,
        "the ceiling must fit under the quota, or this config would not validate"
    );
    let tight = fixture(&config2, &state2, "canvas-charged", 2).await;
    state2
        .services
        .place_canvas(tight.owner, tight.tracks[0], &filler, None)
        .await
        .unwrap();
    // Exactly the stripped size is left. Charged `noisy.len()` this is a
    // `Conflict`; charged what the store holds it fits to the byte.
    let squeezed = state2
        .services
        .place_canvas(tight.owner, tight.tracks[1], &noisy, None)
        .await
        .unwrap();
    assert_eq!(squeezed.byte_size, placed.byte_size);
}

/// An mp4 big enough to fill a quota without running long enough to trip the
/// duration ceiling: detail costs bytes where duration costs seconds.
fn detailed_canvas_bytes(dir: &std::path::Path) -> Vec<u8> {
    let path = dir.join("filler.mp4");
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=160x120:r=10:d=2",
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

/// An mp4 carrying both a picture and a soundtrack.
fn noisy_canvas_bytes(dir: &std::path::Path) -> Vec<u8> {
    let path = dir.join("noisy.mp4");
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:r=10:d=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(&path)
        .output()
        .expect("ffmpeg must be on PATH");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(&path).unwrap();
    assert!(has_audio_stream(&path), "the fixture must carry sound");
    bytes
}

fn has_audio_stream(path: &std::path::Path) -> bool {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("ffprobe must be on PATH");
    assert!(output.status.success());
    !String::from_utf8_lossy(&output.stdout).trim().is_empty()
}
