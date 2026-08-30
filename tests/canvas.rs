//! The loop a member attaches to a track. RFC-009.
//!
//! No routes yet: this drives `DomainServices` directly, which is the whole of
//! what the domain half of the canvas is. What the store does with bytes, what
//! it charges for them, and what it refuses are decided here rather than at the
//! surface, so they are tested here.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
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
    token: String,
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
    // server that was merely upgraded must not have become one. Its own flag,
    // not the upload one — the fixture leaves `accepts_uploads` off precisely
    // so that every test here runs on a library that takes no audio.
    state
        .db
        .set_library_accepts_canvas(owner, library, true, now_ms())
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
    let token = login_token(
        &waveflow_server::app(config, state.clone()),
        name,
        "correct horse battery staple",
    )
    .await;
    Fixture {
        owner,
        library,
        tracks,
        token,
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

// ---------------------------------------------------------------------------
// The surfaces. RFC-009 decisions 3 and 4.
// ---------------------------------------------------------------------------

fn authorized(request: axum::http::request::Builder, token: &str) -> axum::http::request::Builder {
    request.header("authorization", format!("Bearer {token}"))
}

#[tokio::test]
async fn a_canvas_is_placed_read_back_and_taken_away_over_http() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-routes", 1).await;
    let router = waveflow_server::app(&config, state.clone());
    let bytes = canvas_bytes(temp.path(), "loop.mp4", "1", "black");
    let track = fixture.tracks[0];

    let placed = router
        .clone()
        .oneshot(
            authorized(
                Request::put(format!("/api/v2/tracks/{track}/canvas")),
                &fixture.token,
            )
            .body(Body::from(bytes.clone()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(placed.status(), StatusCode::OK);
    let placed = json_body(placed).await;
    assert_eq!(placed["format"], "mp4");
    assert_eq!(placed["byte_size"], bytes.len() as i64);
    let hash = placed["hash"].as_str().unwrap().to_owned();
    assert_eq!(placed["url"], format!("/api/v2/canvas/{hash}"));

    // Addressed by content: this URL can never answer differently, so the
    // client is told to stop asking.
    let by_hash = router
        .clone()
        .oneshot(
            authorized(
                Request::get(format!("/api/v2/canvas/{hash}")),
                &fixture.token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(by_hash.status(), StatusCode::OK);
    assert_eq!(by_hash.headers()["content-type"], "video/mp4");
    assert_eq!(
        by_hash.headers()["cache-control"],
        "private, max-age=31536000, immutable"
    );
    let etag = by_hash.headers()["etag"].to_str().unwrap().to_owned();
    assert_eq!(etag, format!("\"{hash}\""));

    // Addressed by track: it resolves the link of the moment, which a member
    // can replace, so it stays revalidatable.
    let alias = router
        .clone()
        .oneshot(
            authorized(
                Request::get(format!("/api/v2/tracks/{track}/canvas")),
                &fixture.token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(alias.status(), StatusCode::OK);
    assert_eq!(alias.headers()["cache-control"], "private, no-cache");
    // The validator is the hash either way, which is what makes revalidating
    // the alias a 304 rather than a second transfer of the same bytes.
    assert_eq!(alias.headers()["etag"], etag);

    // Revalidation on both shapes. The alias is the one that has to answer 304
    // — it is the URL a client keeps asking about — but the canonical one must
    // too, or a client that does ask pays for the bytes twice.
    for path in [
        format!("/api/v2/tracks/{track}/canvas"),
        format!("/api/v2/canvas/{hash}"),
    ] {
        let revalidated = router
            .clone()
            .oneshot(
                authorized(Request::get(&path), &fixture.token)
                    .header("if-none-match", &etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revalidated.status(), StatusCode::NOT_MODIFIED, "{path}");
        // The validator comes back with the 304, so the entry stays fresh
        // rather than being evicted for having lost its tag.
        assert_eq!(revalidated.headers()["etag"], etag, "{path}");
    }

    // A <video> element asks for ranges, and it asks the ticket URL. All three
    // ways in serve the same file through the same helper, so all three answer
    // 206 and carry the validator on the partial too.
    let ticket = json_body(
        router
            .clone()
            .oneshot(
                authorized(
                    Request::post(format!("/api/v2/tracks/{track}/canvas-ticket")),
                    &fixture.token,
                )
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await["url"]
        .as_str()
        .unwrap()
        .to_owned();
    for (path, bearer) in [
        (format!("/api/v2/canvas/{hash}"), true),
        (format!("/api/v2/tracks/{track}/canvas"), true),
        // No Authorization header at all: the sealed ticket is the credential.
        (ticket, false),
    ] {
        let request = Request::get(&path);
        let request = if bearer {
            authorized(request, &fixture.token)
        } else {
            request
        };
        let ranged = router
            .clone()
            .oneshot(
                request
                    .header("range", "bytes=0-15")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT, "{path}");
        assert_eq!(ranged.headers()["etag"], etag, "{path}");
        assert_eq!(ranged.headers()["content-length"], "16", "{path}");
    }

    let removed = router
        .clone()
        .oneshot(
            authorized(
                Request::delete(format!("/api/v2/tracks/{track}/canvas")),
                &fixture.token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    for gone in [
        format!("/api/v2/tracks/{track}/canvas"),
        format!("/api/v2/canvas/{hash}"),
    ] {
        let response = router
            .clone()
            .oneshot(
                authorized(Request::get(&gone), &fixture.token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{gone}");
    }
}

#[tokio::test]
async fn a_ticket_opens_the_canvas_it_was_minted_for_and_nothing_else() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-ticket", 1).await;
    let router = waveflow_server::app(&config, state.clone());
    let track = fixture.tracks[0];
    state
        .services
        .place_canvas(
            fixture.owner,
            track,
            &canvas_bytes(temp.path(), "loop.mp4", "1", "black"),
            None,
        )
        .await
        .unwrap();

    let minted = router
        .clone()
        .oneshot(
            authorized(
                Request::post(format!("/api/v2/tracks/{track}/canvas-ticket")),
                &fixture.token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(minted.status(), StatusCode::OK);
    let url = json_body(minted).await["url"].as_str().unwrap().to_owned();
    assert!(url.starts_with("/api/v2/canvas-stream/"));

    // What <video src> does: no Authorization header at all.
    let played = router
        .clone()
        .oneshot(Request::get(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(played.status(), StatusCode::OK);
    assert_eq!(played.headers()["content-type"], "video/mp4");

    // The two kinds do not answer for each other, which is the whole reason the
    // kind is sealed inside the payload. An audio ticket for this very track,
    // minted for this very account and unexpired, does not open the canvas.
    let audio = waveflow_server::stream_ticket::mint(
        &state.secret_box,
        waveflow_server::stream_ticket::TicketKind::Audio,
        fixture.owner,
        track,
        now_ms() + 60_000,
    )
    .unwrap();
    let refused = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/canvas-stream/{audio}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);

    // And the reverse: the canvas ticket does not open the audio route.
    let canvas_ticket = url.rsplit('/').next().unwrap();
    let refused = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/stream/{canvas_ticket}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_route_carries_its_own_ceiling_and_the_neighbour_sees_nothing() {
    let (temp, config, state) = canvas_app(|canvas| {
        canvas.max_bytes = 4096;
        canvas.library_quota_bytes = 1024 * 1024;
    })
    .await;
    let owner = fixture(&config, &state, "canvas-ceiling-http", 1).await;
    let router = waveflow_server::app(&config, state.clone());
    let track = owner.tracks[0];

    // Over the route's own ceiling. The router's global 16 KiB limit would have
    // let this through, so what refuses it is the limit this route carries.
    let oversized = router
        .clone()
        .oneshot(
            authorized(
                Request::put(format!("/api/v2/tracks/{track}/canvas")),
                &owner.token,
            )
            .body(Body::from(vec![0u8; 8192]))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // Under the ceiling and still not a loop: the request was well formed, and
    // what it carried is not something this server will keep.
    let garbage = router
        .clone()
        .oneshot(
            authorized(
                Request::put(format!("/api/v2/tracks/{track}/canvas")),
                &owner.token,
            )
            .body(Body::from(vec![7u8; 2048]))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(garbage.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // A real one, so the neighbour has something to fail to see.
    let (_temp2, config2, state2) = canvas_app(|_| {}).await;
    let roomy = fixture(&config2, &state2, "canvas-roomy", 1).await;
    let router2 = waveflow_server::app(&config2, state2.clone());
    let placed = state2
        .services
        .place_canvas(
            roomy.owner,
            roomy.tracks[0],
            &canvas_bytes(temp.path(), "loop.mp4", "1", "black"),
            None,
        )
        .await
        .unwrap();
    let neighbour = fixture(&config2, &state2, "canvas-neighbour-2", 1).await;

    // Knowing the hash establishes nothing, and neither does knowing the track.
    for path in [
        format!("/api/v2/canvas/{}", placed.hash),
        format!("/api/v2/tracks/{}/canvas", roomy.tracks[0]),
    ] {
        let response = router2
            .clone()
            .oneshot(
                authorized(Request::get(&path), &neighbour.token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
    // And they cannot put one on somebody else's track either.
    let intruding = router2
        .clone()
        .oneshot(
            authorized(
                Request::put(format!("/api/v2/tracks/{}/canvas", roomy.tracks[0])),
                &neighbour.token,
            )
            .body(Body::from(vec![7u8; 512]))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(intruding.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn placing_and_removing_a_canvas_travels_in_the_track_upsert() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-feed", 1).await;
    let router = waveflow_server::app(&config, state.clone());
    let (token, device) =
        login_session(&router, "canvas-feed", "correct horse battery staple").await;
    let track = fixture.tracks[0];
    let library = fixture.library;

    // Where the feed already stood, so what follows is what this test caused
    // rather than what the scan left behind.
    let before = library_events(&router, &token, library).await.len();

    let placed = router
        .clone()
        .oneshot(
            authorized(
                Request::put(format!("/api/v2/tracks/{track}/canvas")),
                &token,
            )
            .header("x-waveflow-device-id", &device)
            .body(Body::from(canvas_bytes(
                temp.path(),
                "loop.mp4",
                "1",
                "black",
            )))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(placed.status(), StatusCode::OK);

    let events = library_events(&router, &token, library).await;
    assert_eq!(events.len(), before + 1, "one event, and only one");
    let event = events.last().unwrap();
    // RFC-009 decision 7: no event for the blob, because the bytes behind a
    // hash never change. Only the link is a change, and the link is part of the
    // track — so it travels in the track's own upsert.
    assert_eq!(event["entity_type"], "track");
    assert_eq!(event["action"], "upsert");
    assert_eq!(event["entity_id"], track.to_string());
    // And the payload does not grow. `full_hash` is there because nothing else
    // carries it; a canvas link is read off the track the client refetches.
    assert!(
        event["payload"]["full_hash"].is_string(),
        "the payload is the one a tag correction sends"
    );
    assert!(
        event["payload"].get("canvas").is_none(),
        "no canvas field: that would make the payload a partial projection"
    );
    // So the client that just placed it does not read it back as a discovery.
    assert_eq!(event["origin_device_id"], device);

    // Taking it away is a change too, and it is the same shape.
    let removed = router
        .clone()
        .oneshot(
            authorized(
                Request::delete(format!("/api/v2/tracks/{track}/canvas")),
                &token,
            )
            .header("x-waveflow-device-id", &device)
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    let events = library_events(&router, &token, library).await;
    assert_eq!(events.len(), before + 2);
    assert_eq!(events.last().unwrap()["action"], "upsert");

    // A removal that found nothing changed nothing, and says nothing.
    let again = router
        .clone()
        .oneshot(
            authorized(
                Request::delete(format!("/api/v2/tracks/{track}/canvas")),
                &token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        library_events(&router, &token, library).await.len(),
        before + 2,
        "nothing happened, so nothing was announced"
    );
}

async fn library_events(
    router: &axum::Router,
    token: &str,
    library: uuid::Uuid,
) -> Vec<serde_json::Value> {
    let feed = router
        .clone()
        .oneshot(
            authorized(
                Request::get(format!(
                    "/api/v2/libraries/{library}/events?after=0&limit=500"
                )),
                token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(feed.status(), StatusCode::OK);
    json_body(feed).await["events"]
        .as_array()
        .expect("the feed answers a list")
        .clone()
}

#[tokio::test]
async fn the_two_doors_are_independent_in_both_directions() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-doors", 1).await;
    let bytes = canvas_bytes(temp.path(), "loop.mp4", "1", "black");
    let track = fixture.tracks[0];

    // The case that made this a separate flag. `fixture` opened the canvas door
    // and left the upload one shut, which is the read-only server the desktop
    // named: it refuses to grow in audio and still takes a loop.
    // Stated rather than inherited from the fixture, so the situation this test
    // is about is visible in the test.
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, false, now_ms())
        .await
        .unwrap();
    state
        .services
        .place_canvas(fixture.owner, track, &bytes, None)
        .await
        .unwrap();

    // And the reverse: opening the library to files says nothing about loops.
    // A flag inherited from the other would be the operator opting into
    // something he never named.
    state
        .services
        .remove_canvas(fixture.owner, track, None)
        .await
        .unwrap();
    state
        .db
        .set_library_accepts_canvas(fixture.owner, fixture.library, false, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();
    assert!(matches!(
        state
            .services
            .place_canvas(fixture.owner, track, &bytes, None)
            .await,
        Err(waveflow_server::services::ServiceError::Forbidden)
    ));
}
