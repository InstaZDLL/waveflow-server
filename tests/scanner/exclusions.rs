//! Exclusions, ignore files and symbolic links (RFC-021 decisions 1 to 3).

use super::*;

/// RFC-021 decision 1: the built-in exclusions and `.waveflowignore`, with its
/// gitignore rules, its empty-file rule and its `!`. Received files are kept,
/// and the ignore files themselves are not counted as left out.
#[tokio::test]
async fn built_in_exclusions_and_ignore_files_leave_files_out() {
    let (_temp, state, owner, library, _root) = incremental_library("exclusions", |music| {
        for (index, path) in [
            "Album/One.wav",
            "@eaDir/Thumb.wav",
            ".hidden/Two.wav",
            "._One.wav",
            "Skip/Three.wav",
            "Nested/Four.wav",
            "Keep/Unwanted.wav",
            "Keep/Wanted.wav",
            ".waveflow-managed/Received.wav",
        ]
        .into_iter()
        .enumerate()
        {
            let path = music.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            write_test_wav_of_len(&path, 800 + index * 10);
        }
        std::fs::write(music.join(".waveflowignore"), "# not these\n/Skip/\n").unwrap();
        std::fs::write(music.join("Nested/.waveflowignore"), "").unwrap();
        std::fs::write(music.join("Keep/.waveflowignore"), "*.wav\n!Wanted.wav\n").unwrap();
    })
    .await;
    let scan = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(
        available_paths(&state).await,
        [
            ".waveflow-managed/Received.wav",
            "Album/One.wav",
            "Keep/Wanted.wav"
        ]
    );
    // @eaDir, .hidden, ._One.wav, Skip, Nested, Keep/Unwanted.wav.
    assert_eq!(scan.excluded, 6);
    assert_eq!(scan.errors, 0);
}

/// RFC-021 decision 2: an excluded file counts as not discovered. Its track
/// becomes unavailable like a deleted one, with the time it went, and comes
/// back under the same identifier when the rule is lifted.
#[tokio::test]
async fn an_excluded_track_becomes_unavailable_like_a_deleted_one() {
    let (_temp, state, owner, library, root) = incremental_library("excluded-later", |music| {
        std::fs::create_dir_all(music.join("Live")).unwrap();
        write_test_wav(&music.join("Live/One.wav"));
        write_test_wav_of_len(&music.join("Studio.wav"), 900);
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    let id = track_id(&state, "Live/One.wav").await;

    std::fs::write(root.join(".waveflowignore"), "Live/\n").unwrap();
    let excluded = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(
        excluded.unavailable, 1,
        "an excluded file must count as missing, not as unseen"
    );
    let (available, since): (i64, Option<i64>) =
        sqlx::query_as("SELECT is_available, unavailable_since FROM track WHERE id = ?")
            .bind(&id)
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(available, 0);
    assert!(since.is_some(), "the time it went missing is recorded");

    std::fs::remove_file(root.join(".waveflowignore")).unwrap();
    scan_once(&state, owner, library).await;
    let (available, since): (i64, Option<i64>) =
        sqlx::query_as("SELECT is_available, unavailable_since FROM track WHERE id = ?")
            .bind(&id)
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!((available, since), (1, None));
}

/// RFC-021 decision 1: a folder whose ignore file cannot be used is an
/// unreadable folder — reported, and its tracks left as they are — while the
/// same failure at the root fails the scan.
#[tokio::test]
async fn an_ignore_file_that_cannot_be_used_keeps_its_tracks() {
    let (_temp, state, owner, library, root) = incremental_library("ignore-unusable", |music| {
        std::fs::create_dir_all(music.join("Sub")).unwrap();
        write_test_wav(&music.join("Sub/One.wav"));
        write_test_wav_of_len(&music.join("Two.wav"), 900);
    })
    .await;
    scan_once(&state, owner, library.clone()).await;

    std::fs::write(root.join("Sub/.waveflowignore"), "[z-a]\n").unwrap();
    let scan = scan_once(&state, owner, library.clone()).await;
    let record = job(&state, owner, scan).await;
    assert_eq!(record.errors, 1);
    assert_eq!(
        record.unavailable, 0,
        "a track under a folder whose rules are unknown was marked missing"
    );
    assert_eq!(available_paths(&state).await, ["Sub/One.wav", "Two.wav"]);
    let message: String = sqlx::query_scalar("SELECT message FROM scan_error WHERE scan_id = ?")
        .bind(scan.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(message.contains(".waveflowignore"), "{message}");

    std::fs::write(root.join(".waveflowignore"), "[z-a]\n").unwrap();
    let failing = state
        .scanner
        .trigger(library, Some(owner), "manual")
        .await
        .unwrap();
    assert_eq!(
        settled(&state, owner, failing).await.status,
        "failed",
        "a root whose ignore file cannot be used must fail the scan"
    );
    assert_eq!(available_paths(&state).await, ["Sub/One.wav", "Two.wav"]);
}

/// A symbolic link to `target` at `link`, or `false` where this system will
/// not let the test make one (Windows without the privilege).
fn make_link(target: &std::path::Path, link: &std::path::Path, directory: bool) -> bool {
    #[cfg(unix)]
    {
        let _ = directory;
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        if directory {
            std::os::windows::fs::symlink_dir(target, link).is_ok()
        } else {
            std::os::windows::fs::symlink_file(target, link).is_ok()
        }
    }
}

/// RFC-021 decision 3: links are skipped and counted unless the library
/// follows them, and then only those whose target stays inside the root; a
/// followed file is one track under its canonical path, however many links
/// name it.
#[tokio::test]
async fn links_are_followed_only_inside_the_library() {
    let outside = tempfile::tempdir().unwrap();
    write_test_wav_of_len(&outside.path().join("Elsewhere.wav"), 950);
    let mut linked = true;
    let (_temp, state, owner, library, _root) = incremental_library("links", |music| {
        std::fs::create_dir_all(music.join("store")).unwrap();
        std::fs::create_dir_all(music.join("Album")).unwrap();
        write_test_wav(&music.join("store/a.wav"));
        // The store is reached through the links only.
        std::fs::write(music.join(".waveflowignore"), "/store/\n").unwrap();
        linked &= make_link(
            &music.join("store/a.wav"),
            &music.join("Album/a.wav"),
            false,
        );
        linked &= make_link(&music.join("store"), &music.join("Mirror"), true);
        linked &= make_link(
            &outside.path().join("Elsewhere.wav"),
            &music.join("Outside.wav"),
            false,
        );
        linked &= make_link(&music.join("missing.wav"), &music.join("Broken.wav"), false);
        linked &= make_link(music, &music.join("Loop"), true);
    })
    .await;
    if !linked {
        eprintln!("skipped: this system will not let the test make a symbolic link");
        return;
    }

    let unfollowed = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(unfollowed.added, 0);
    assert_eq!(
        unfollowed.skipped_links, 5,
        "every link must be counted, not skipped in silence"
    );

    let updated = state
        .services
        .update_library(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            waveflow_server::services::LibraryUpdate {
                follow_links: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let scan = updated
        .scan_id
        .expect("changing the links setting queues a scan");
    let followed = settled(&state, owner, scan).await;
    assert_eq!(followed.status, "completed", "{:?}", followed.message);
    assert_eq!(
        available_paths(&state).await,
        ["store/a.wav"],
        "one file, under its canonical path, whatever links name it"
    );
    // Outside the root, broken, and a loop back to the root.
    assert_eq!(followed.skipped_links, 3);
}

/// RFC-018 decision 3 under RFC-021's links: a folder reached only through a
/// followed link, whose rules cannot be used, spares its tracks under the name
/// the catalogue gives them — the target's — and not under the link's, where
/// they would be marked missing anyway.
#[tokio::test]
async fn a_folder_reached_through_a_link_spares_its_tracks_when_it_cannot_be_used() {
    let mut linked = true;
    let (_temp, state, owner, library, root) = incremental_library("alias-unusable", |music| {
        std::fs::create_dir_all(music.join(".hidden/Sub")).unwrap();
        write_test_wav(&music.join(".hidden/Sub/One.wav"));
        // The hidden folder is reached through the link only.
        linked &= make_link(&music.join(".hidden"), &music.join("Alias"), true);
    })
    .await;
    if !linked {
        eprintln!("skipped: this system will not let the test make a symbolic link");
        return;
    }
    sqlx::query("UPDATE library SET follow_links = 1 WHERE id = ?")
        .bind(library.id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    scan_once(&state, owner, library.clone()).await;
    assert_eq!(available_paths(&state).await, [".hidden/Sub/One.wav"]);

    std::fs::write(root.join(".hidden/Sub/.waveflowignore"), "[z-a]\n").unwrap();
    let scan = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(scan.errors, 1);
    assert_eq!(
        scan.unavailable, 0,
        "a track under a folder the walk could not use was marked missing"
    );
    assert_eq!(available_paths(&state).await, [".hidden/Sub/One.wav"]);
}

/// RFC-021, what decision 3 left open: with `WAVEFLOW_LINK_ROOTS`, a library
/// that follows links reads a link farm whose files live in an allowed folder
/// outside its root — once per file, under the path the walk reached it by,
/// and streamable — while a link into another library, or anywhere else, is
/// still skipped and counted, and a loop is still caught.
#[tokio::test]
async fn links_reach_into_an_allowed_folder_and_nowhere_else() {
    let temp = tempfile::tempdir().unwrap();
    let farm = temp.path().join("farm");
    let elsewhere = temp.path().join("elsewhere");
    let music = temp.path().join("music");
    let neighbour = temp.path().join("farm/neighbour");
    for folder in [&farm, &elsewhere, &music, &neighbour] {
        std::fs::create_dir_all(folder).unwrap();
    }
    write_test_wav_of_len(&farm.join("Farmed.wav"), 1_111);
    write_test_wav_of_len(&elsewhere.join("Secret.wav"), 2_222);
    write_test_wav_of_len(&neighbour.join("Theirs.wav"), 3_333);
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.link_roots = vec![farm.clone()];
    let state = waveflow_server::initialize(&config).await.unwrap();
    let owner = insert_account(&state, "farm-owner", "admin").await;
    // Another library lives inside the allowed folder: no link may show its
    // files to the members of this one.
    state
        .db
        .create_library(
            owner,
            "neighbour",
            &std::fs::canonicalize(&neighbour).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let linked = make_link(&farm.join("Farmed.wav"), &music.join("Single.wav"), false)
        && make_link(&farm, &music.join("Farm"), true)
        && make_link(
            &elsewhere.join("Secret.wav"),
            &music.join("Secret.wav"),
            false,
        )
        && make_link(
            &neighbour.join("Theirs.wav"),
            &music.join("Theirs.wav"),
            false,
        )
        && make_link(&music, &farm.join("Back"), true);
    if !linked {
        eprintln!("skipped: this system will not let the test make a symbolic link");
        return;
    }
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "farmed", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "farmed".into(),
        root_path: root,
    };
    let updated = state
        .services
        .update_library(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            waveflow_server::services::LibraryUpdate {
                follow_links: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let scanned = settled(&state, owner, updated.scan_id.unwrap()).await;
    assert_eq!(scanned.status, "completed", "{:?}", scanned.message);
    let paths: Vec<String> = sqlx::query_scalar(
        "SELECT relative_path FROM track WHERE library_id = ? AND is_available = 1 \
         ORDER BY relative_path",
    )
    .bind(library.id.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        paths,
        ["Farm/Farmed.wav"],
        "one file, once, under the first path that reaches it"
    );
    // The secret elsewhere, the neighbour's file, the neighbour's folder seen
    // through the farm link, and the loop back from the farm to the library.
    assert_eq!(scanned.skipped_links, 4, "{scanned:?}");

    let track_id: String = sqlx::query_scalar("SELECT id FROM track WHERE library_id = ?")
        .bind(library.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    let stream = || async {
        let track = state
            .db
            .stream_track_for_user(owner, track_id.parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        state
            .media
            .serve(
                owner,
                track,
                waveflow_server::media::StreamQuery {
                    format: waveflow_server::media::OutputFormat::Raw,
                    bitrate: None,
                    offset_ms: 0,
                },
                None,
            )
            .await
    };
    assert_eq!(stream().await.unwrap().status(), axum::http::StatusCode::OK);

    // The farm's file becomes a link into the neighbouring library. The path
    // this library stored now resolves into another library's root, which no
    // link may reach — whoever asks, since access was checked on this library
    // alone.
    let farmed = farm.join("Farmed.wav");
    let kept = std::fs::read(&farmed).unwrap();
    std::fs::remove_file(&farmed).unwrap();
    assert!(make_link(&neighbour.join("Theirs.wav"), &farmed, false));
    assert!(
        matches!(
            stream().await,
            Err(waveflow_server::media::MediaError::NotFound)
        ),
        "a link retargeted into another library streamed its file"
    );
    std::fs::remove_file(&farmed).unwrap();
    std::fs::write(&farmed, kept).unwrap();
    assert!(stream().await.is_ok());

    // A library that stopped following links reads no link, allowlist or not.
    sqlx::query("UPDATE library SET follow_links = 0 WHERE id = ?")
        .bind(library.id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    assert!(
        matches!(
            stream().await,
            Err(waveflow_server::media::MediaError::NotFound)
        ),
        "a library that does not follow links streamed through one"
    );
}
