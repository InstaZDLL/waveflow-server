//! What the routes say about the analysis: the verdicts, the spectrogram, the
//! attention list and the summary, and who may read or ask for each.

use super::*;

async fn send(
    router: &axum::Router,
    token: &str,
    request: Request<Body>,
) -> axum::response::Response {
    let (mut parts, body) = request.into_parts();
    parts
        .headers
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    router
        .clone()
        .oneshot(Request::from_parts(parts, body))
        .await
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn members_read_the_quality_and_only_curators_ask_for_it_again() {
    let shelf = shelf("routes").await;
    let scratch = shelf.config.data_dir.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    genuine(&shelf.music.join("genuine.flac"), 3);
    from_mp3(&shelf.music.join("from-mp3.flac"), &scratch);
    shelf.scan().await;
    shelf.analyze().await;
    let listener = account(&shelf.state, "listener").await;
    account(&shelf.state, "stranger").await;
    shelf.join(listener, LibraryRole::Listener).await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let owning = shelf.token("routes").await;
    let listening = shelf.token("listener").await;
    let outside = shelf.token("stranger").await;
    let library = shelf.library.id;
    let fake = shelf.track("from-mp3.flac").await;

    let summary = json_body(
        send(
            &router,
            &listening,
            get(&format!("/api/v2/libraries/{library}/quality")),
        )
        .await,
    )
    .await;
    assert_eq!(summary["enabled"], true);
    assert_eq!(summary["lossless"], 2);
    assert_eq!(summary["measured"], 2);
    assert_eq!(summary["verdicts"][0]["verdict"], "lossy_source");
    assert_eq!(summary["verdicts"][0]["count"], 1);

    let list = json_body(
        send(
            &router,
            &listening,
            get(&format!("/api/v2/libraries/{library}/quality/tracks")),
        )
        .await,
    )
    .await;
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["id"], fake.to_string(), "the worst first");
    assert_eq!(list[0]["quality"]["verdict"], "lossy_source");
    assert_eq!(list[0]["quality"]["nyquist_hz"], 22_050.0);
    assert_eq!(list[1]["quality"]["verdict"], "consistent");

    let filtered = json_body(
        send(
            &router,
            &owning,
            get(&format!(
                "/api/v2/libraries/{library}/quality/tracks?verdict=consistent"
            )),
        )
        .await,
    )
    .await;
    assert_eq!(filtered.as_array().unwrap().len(), 1);
    assert_eq!(
        send(
            &router,
            &owning,
            get(&format!(
                "/api/v2/libraries/{library}/quality/tracks?verdict=fine"
            )),
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let detail = json_body(
        send(
            &router,
            &listening,
            get(&format!("/api/v2/tracks/{fake}/quality")),
        )
        .await,
    )
    .await;
    assert_eq!(detail["verdict"], "lossy_source");
    assert_eq!(detail["spectrum"].as_array().unwrap().len(), 256);

    let analysis = || {
        Request::post(format!("/api/v2/libraries/{library}/quality/analysis"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(&router, &listening, analysis()).await.status(),
        StatusCode::NOT_FOUND,
        "a listener cannot spend the operator's CPU"
    );
    assert_eq!(shelf.rows().await, 2);
    assert_eq!(
        send(&router, &owning, analysis()).await.status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        shelf.rows().await,
        0,
        "the library's measurements are dropped"
    );
    assert_eq!(shelf.analyze().await, 2);

    for uri in [
        format!("/api/v2/libraries/{library}/quality"),
        format!("/api/v2/libraries/{library}/quality/tracks"),
        format!("/api/v2/tracks/{fake}/quality"),
        format!("/api/v2/tracks/{fake}/spectrogram"),
    ] {
        assert_eq!(
            send(&router, &outside, get(&uri)).await.status(),
            StatusCode::NOT_FOUND,
            "a stranger is told nothing at {uri}"
        );
    }
    assert_eq!(
        send(&router, &outside, analysis()).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_spectrogram_is_revalidated_by_whoever_asks() {
    let shelf = shelf("spectrogram").await;
    genuine(&shelf.music.join("a.flac"), 3);
    shelf.scan().await;
    let listener = account(&shelf.state, "listener").await;
    shelf.join(listener, LibraryRole::Listener).await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let listening = shelf.token("listener").await;
    let track = shelf.track("a.flac").await;
    let uri = format!("/api/v2/tracks/{track}/spectrogram");

    let response = send(&router, &listening, get(&uri)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        content_type == "image/webp" || content_type == "image/jpeg",
        "{content_type}"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-cache",
        "kept, but asked about before every use"
    );
    let etag = response.headers()[header::ETAG]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let magic: &[u8] = if content_type == "image/webp" {
        b"RIFF"
    } else {
        &[0xFF, 0xD8]
    };
    assert!(bytes.starts_with(magic), "a picture of the type it names");

    let revalidate = || {
        Request::get(&uri)
            .header(header::IF_NONE_MATCH, &etag)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(&router, &listening, revalidate()).await.status(),
        StatusCode::NOT_MODIFIED
    );

    shelf
        .state
        .db
        .remove_library_member(shelf.owner, shelf.library.id, listener, now_ms())
        .await
        .unwrap();
    assert_eq!(
        send(&router, &listening, revalidate()).await.status(),
        StatusCode::NOT_FOUND,
        "a browser's copy does not outlive the membership"
    );
}

#[tokio::test]
async fn one_spectrogram_per_account_at_a_time() {
    let shelf = shelf("bound").await;
    // Long enough that the first is still being drawn when the second asks.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 240, 3),
        "-ac",
        "2",
        "-sample_fmt",
        "s16",
        "-c:a",
        "flac",
        shelf.music.join("long.flac").to_str().unwrap(),
    ]);
    shelf.scan().await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let token = shelf.token("bound").await;
    let uri = format!(
        "/api/v2/tracks/{}/spectrogram",
        shelf.track("long.flac").await
    );

    let (first, second) = tokio::join!(
        send(&router, &token, get(&uri)),
        send(&router, &token, get(&uri))
    );
    let mut statuses = [first.status(), second.status()];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::TOO_MANY_REQUESTS]);
    let busy = if first.status() == StatusCode::TOO_MANY_REQUESTS {
        first
    } else {
        second
    };
    assert!(busy.headers().contains_key(header::RETRY_AFTER));
}

#[tokio::test]
async fn needs_attention_lists_the_fake_lossless_files() {
    let shelf = shelf("attention").await;
    let scratch = shelf.config.data_dir.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    genuine(&shelf.music.join("genuine.flac"), 3);
    from_mp3(&shelf.music.join("from-mp3.flac"), &scratch);
    padded(&shelf.music.join("padded.flac"), &scratch);
    shelf.scan().await;

    let count = || async {
        shelf
            .state
            .services
            .attention_summary(shelf.owner, shelf.library.id)
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.category == AttentionCategory::ProbableFakeLossless)
            .unwrap()
            .count
    };
    assert_eq!(count().await, 0, "nothing before it is measured");
    shelf.analyze().await;
    assert_eq!(count().await, 1, "the fake, and not the padded file");
    let tracks = shelf
        .state
        .services
        .attention_tracks(
            shelf.owner,
            shelf.library.id,
            AttentionCategory::ProbableFakeLossless,
            BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].song.id, shelf.track("from-mp3.flac").await);
}

/// With the analysis off, the server draws one spectrogram at a time; a
/// stranger who asks while that one is drawn is told nothing exists, not that
/// the server is busy with it.
#[tokio::test]
async fn a_stranger_is_told_nothing_even_at_the_bound() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.quality_parallelism = 0;
    let state = waveflow_server::initialize(&config).await.unwrap();
    let owner = account(&state, "off").await;
    let music = config.data_dir.join("off");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let id = state
        .db
        .create_library(owner, "off", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let shelf = Shelf {
        state,
        config,
        owner,
        library: LibraryRecord {
            id,
            name: "off".into(),
            root_path: root,
        },
        music,
        _temp: temp,
    };
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 240, 3),
        "-ac",
        "2",
        "-sample_fmt",
        "s16",
        "-c:a",
        "flac",
        shelf.music.join("long.flac").to_str().unwrap(),
    ]);
    shelf.scan().await;
    account(&shelf.state, "stranger").await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let owning = shelf.token("off").await;
    let outside = shelf.token("stranger").await;
    let track = shelf.track("long.flac").await;
    let uri = format!("/api/v2/tracks/{track}/spectrogram");

    assert_eq!(shelf.analyze().await, 0, "the operator turned it off");
    let summary = json_body(
        send(
            &router,
            &owning,
            get(&format!("/api/v2/libraries/{}/quality", shelf.library.id)),
        )
        .await,
    )
    .await;
    assert_eq!(summary["enabled"], false);
    assert_eq!(summary["measured"], 0);

    let (drawn, refused) = tokio::join!(send(&router, &owning, get(&uri)), async {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        send(&router, &outside, get(&uri)).await
    });
    assert_eq!(drawn.status(), StatusCode::OK);
    assert_eq!(
        refused.status(),
        StatusCode::NOT_FOUND,
        "access is checked before the bound"
    );
}

/// A minute of noise: long enough that a pass over a few of them can be
/// watched while it runs.
fn minute(path: &Path, seed: u32) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 60, seed),
        "-ac",
        "2",
        "-sample_fmt",
        "s16",
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

#[tokio::test]
async fn the_summary_says_where_the_analysis_stands() {
    let shelf = shelf("status").await;
    genuine(&shelf.music.join("a.flac"), 3);
    shelf.scan().await;

    let before = analysis(&shelf).await;
    assert_eq!(
        before.state,
        AnalysisState::Queued,
        "files wait, and no pass is on them"
    );
    assert_eq!(before.in_flight, 0);
    assert_eq!(before.last_measured_at, None);

    let scan = shelf.state.scanner.begin_scan(shelf.library.id);
    assert_eq!(
        analysis(&shelf).await.state,
        AnalysisState::WaitingForScan,
        "its own library is being scanned"
    );
    drop(scan);

    shelf.analyze().await;
    let after = analysis(&shelf).await;
    assert_eq!(after.state, AnalysisState::Idle);
    assert_eq!(after.in_flight, 0);
    assert_eq!(after.pass_started_at, None);
    assert!(after.last_measured_at.is_some());
}

/// A pass over three files is watched while it runs: it says it is running,
/// names the file it is decoding, and the count moves one file at a time
/// rather than all at once at the end.
#[tokio::test]
async fn a_running_pass_moves_one_file_at_a_time() {
    let shelf = shelf("moving").await;
    for (index, seed) in [3, 5, 7].into_iter().enumerate() {
        minute(&shelf.music.join(format!("{index}.flac")), seed);
    }
    shelf.scan().await;

    let services = shelf.state.services.clone();
    let pass = tokio::spawn(async move { services.analyze_pending_quality().await.unwrap() });
    let mut seen_running = false;
    let mut seen_partial = false;
    for _ in 0..2000 {
        if pass.is_finished() {
            break;
        }
        let now = analysis(&shelf).await;
        if now.state == AnalysisState::Running && now.in_flight == 1 {
            assert!(now.pass_started_at.is_some());
            seen_running = true;
        }
        let rows = shelf.rows().await;
        if (1..3).contains(&rows) && !pass.is_finished() {
            seen_partial = true;
        }
        if seen_running && seen_partial {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        seen_running,
        "a running pass says so, with its file in flight"
    );
    assert!(
        seen_partial,
        "a measurement is written as it finishes, not at the end of the pass"
    );
    assert_eq!(pass.await.unwrap(), 3);
    assert_eq!(analysis(&shelf).await.state, AnalysisState::Idle);
}

/// A scan of somebody else's library holds every measurement back, but it
/// changes nothing its outsiders are shown: the pass reads as running and its
/// file as in flight, exactly as with no scan at all.
#[tokio::test]
async fn a_scan_the_caller_cannot_see_is_not_named() {
    let shelf = shelf("mine").await;
    genuine(&shelf.music.join("a.flac"), 3);
    shelf.scan().await;
    let stranger = account(&shelf.state, "stranger").await;
    let theirs_root = shelf.config.data_dir.join("theirs");
    std::fs::create_dir_all(&theirs_root).unwrap();
    let theirs = shelf
        .state
        .db
        .create_library(
            stranger,
            "theirs",
            &std::fs::canonicalize(&theirs_root).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    let scan = shelf.state.scanner.begin_scan(theirs);
    let services = shelf.state.services.clone();
    let pass = tokio::spawn(async move { services.analyze_pending_quality().await.unwrap() });
    // Long enough for the pass to have reached the scan and stopped there.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(!pass.is_finished(), "the pass waits for the scan");
    let held = analysis(&shelf).await;
    assert_eq!(
        (held.state, held.in_flight),
        (AnalysisState::Running, 1),
        "nothing says another library is being scanned"
    );
    assert!(held.pass_started_at.is_some());

    drop(scan);
    assert_eq!(pass.await.unwrap(), 1);
    assert_eq!(analysis(&shelf).await.state, AnalysisState::Idle);
}

#[tokio::test]
async fn the_route_carries_the_analysis() {
    let shelf = shelf("route-status").await;
    genuine(&shelf.music.join("a.flac"), 3);
    shelf.scan().await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let token = shelf.token("route-status").await;
    let summary = json_body(
        send(
            &router,
            &token,
            get(&format!("/api/v2/libraries/{}/quality", shelf.library.id)),
        )
        .await,
    )
    .await;
    assert_eq!(summary["analysis"]["state"], "queued");
    assert_eq!(summary["analysis"]["in_flight"], 0);
    assert!(summary["analysis"]["last_measured_at"].is_null());
}

/// The header and the list are one answer: for every verdict, the summary
/// counts exactly the files the list carries under it, including when some
/// files are measured, some wait, and a measured one has gone unavailable.
#[tokio::test]
async fn the_summary_counts_what_the_list_lists() {
    let shelf = shelf("agree").await;
    let scratch = shelf.config.data_dir.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    genuine(&shelf.music.join("genuine.flac"), 3);
    from_mp3(&shelf.music.join("from-mp3.flac"), &scratch);
    padded(&shelf.music.join("padded.flac"), &scratch);
    genuine(&shelf.music.join("gone.flac"), 9);
    std::fs::copy(scratch.join("from-mp3.mp3"), shelf.music.join("lossy.mp3")).unwrap();
    shelf.scan().await;
    shelf.analyze().await;
    // Measured, then gone: its row stays, and neither side may count it.
    std::fs::remove_file(shelf.music.join("gone.flac")).unwrap();
    // Added after the pass: pending on both sides.
    genuine(&shelf.music.join("late.flac"), 11);
    shelf.scan().await;

    let summary = shelf
        .state
        .services
        .quality_summary(shelf.owner, shelf.library.id)
        .await
        .unwrap();
    let listed = shelf
        .state
        .services
        .quality_tracks(
            shelf.owner,
            shelf.library.id,
            None,
            None,
            BrowsePage::default(),
        )
        .await
        .unwrap();
    for entry in &summary.verdicts {
        let in_list = listed
            .iter()
            .filter(|track| track.quality.verdict == entry.verdict)
            .count() as i64;
        assert_eq!(entry.count, in_list, "{:?}", entry.verdict);
    }
    assert_eq!(listed.len(), 5);
    assert_eq!(summary.lossless, 4);
    assert_eq!(summary.measured, 3);
}
