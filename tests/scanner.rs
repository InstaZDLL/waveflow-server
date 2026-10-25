//! What a scan reads off the files, and what it does with it.
//!
//! Split out of `v2_foundations.rs`.

use sqlx::Row;
use waveflow_server::authentication::now_ms;
use waveflow_server::catalog::ApplyOutcome;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

#[tokio::test]
async fn scanner_indexes_moves_and_marks_tracks_unavailable() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("scanner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("scan-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("First Track.wav"));
    std::fs::write(
        music.join("First Track.lrc"),
        "[00:01.25]First line\n[00:02.500]Second line",
    )
    .unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Scanner library",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Scanner library".into(),
        root_path: std::fs::canonicalize(&music).unwrap(),
    };

    run_scan(&state, owner, library.clone()).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "First Track");
    assert!(tracks[0].available);
    let stable_id = tracks[0].id;
    let lyrics = state.services.lyrics(owner, stable_id).await.unwrap();
    assert_eq!(lyrics.structured_lyrics.len(), 1);
    assert!(lyrics.structured_lyrics[0].synced);
    assert_eq!(lyrics.structured_lyrics[0].lines[0].start, Some(1_250));
    assert_eq!(lyrics.structured_lyrics[0].lines[0].value, "First line");
    let found = state
        .db
        .search_tracks_for_user(owner, library_id, "First")
        .await
        .unwrap();
    assert_eq!(found[0].id, stable_id);

    // A sidecar can change while the audio bytes and timestamps stay exactly
    // the same. Its fingerprint must prevent the scanner's unchanged-file fast
    // path from preserving stale lyrics.
    std::fs::write(music.join("First Track.lrc"), "[00:03]Replacement").unwrap();
    run_scan(&state, owner, library.clone()).await;
    let lyrics = state.services.lyrics(owner, stable_id).await.unwrap();
    assert_eq!(lyrics.structured_lyrics[0].lines.len(), 1);
    assert_eq!(lyrics.structured_lyrics[0].lines[0].start, Some(3_000));
    assert_eq!(lyrics.structured_lyrics[0].lines[0].value, "Replacement");

    std::fs::create_dir_all(music.join("Moved")).unwrap();
    std::fs::rename(
        music.join("First Track.wav"),
        music.join("Moved").join("Renamed.wav"),
    )
    .unwrap();
    run_scan(&state, owner, library.clone()).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, stable_id);
    assert_eq!(tracks[0].relative_path, "Moved/Renamed.wav");

    std::fs::remove_file(music.join("Moved").join("Renamed.wav")).unwrap();
    run_scan(&state, owner, library).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert!(!tracks[0].available);
}

/// Sidecar lyrics in GB18030, the way a tool on a Chinese Windows saves them
/// (WaveFlow#910). The scanner read sidecars as strict UTF-8 and dropped this
/// one whole, so the track reached every client with no lyrics.
#[tokio::test]
async fn sidecar_lyrics_that_are_not_utf8_reach_the_lyrics_service() {
    let (_temp, config, state) = test_app().await;
    // Inserted rather than created: a scan needs an owner, not a login, and
    // hashing a literal password is what `rust/hard-coded-cryptographic-value`
    // reports.
    let owner = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(owner.to_string())
    .bind("scanner-encodings")
    .bind(now_ms())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let music = config.data_dir.join("encoded-lyrics");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Song.wav"));
    let (bytes, _, unmappable) = encoding_rs::GB18030
        .encode("[00:01.00]今天的天气很好，我们一起去公园散步吧\n[00:05.50]阳光照在湖面上");
    assert!(!unmappable);
    assert!(
        std::str::from_utf8(&bytes).is_err(),
        "the sample is not UTF-8 by accident"
    );
    std::fs::write(music.join("Song.lrc"), &bytes).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Encoded lyrics",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Encoded lyrics".into(),
        root_path: root,
    };

    run_scan(&state, owner, library).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    let lyrics = state.services.lyrics(owner, tracks[0].id).await.unwrap();
    assert_eq!(
        lyrics.structured_lyrics.len(),
        1,
        "the sidecar was read rather than dropped"
    );
    let lines = &lyrics.structured_lyrics[0].lines;
    assert!(lyrics.structured_lyrics[0].synced);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].start, Some(1_000));
    assert_eq!(lines[0].value, "今天的天气很好，我们一起去公园散步吧");
    assert_eq!(lines[1].value, "阳光照在湖面上");
}

#[tokio::test]
async fn scanner_batches_more_than_one_write_group_without_deduplicating_copies() {
    let (_temp, config, state) = test_app().await;
    let password_hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account(
            "batch-scanner",
            &password_hash,
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("batch-scan");
    std::fs::create_dir_all(&music).unwrap();
    for index in 0..30 {
        write_test_wav(&music.join(format!("Track {index:02}.wav")));
    }
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Batch scanner",
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
            name: "Batch scanner".into(),
            root_path: root,
        },
    )
    .await;

    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 30);
    assert!(tracks.iter().all(|track| track.available));
    assert_eq!(
        state
            .db
            .search_tracks_for_user(owner, library_id, "Track")
            .await
            .unwrap()
            .len(),
        30
    );
}

#[tokio::test]
async fn scanner_indexes_dsd64_and_deduplicates_folder_artwork() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("formats", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("formats");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("One.wav"));
    write_test_wav(&music.join("Two.wav"));
    write_test_dsf(&music.join("Native DSD.dsf"));
    std::fs::write(music.join("Native DSD.lrc"), "[00:01]DSD words").unwrap();
    write_test_png(&music.join("cover.png"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Formats",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Formats".into(),
        root_path: root,
    };
    run_scan(&state, owner, library.clone()).await;

    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 3);
    let dsd = tracks
        .iter()
        .find(|track| track.title == "Native DSD")
        .unwrap();
    assert_eq!(dsd.codec.as_deref(), Some("DSD64"));
    let dsd_lyrics = state.services.lyrics(owner, dsd.id).await.unwrap();
    assert_eq!(dsd_lyrics.structured_lyrics[0].lines[0].start, Some(1_000));
    assert_eq!(dsd_lyrics.structured_lyrics[0].lines[0].value, "DSD words");
    let dsd_depth: i64 = sqlx::query_scalar("SELECT bit_depth FROM track WHERE id = ?")
        .bind(dsd.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(dsd_depth, 1);

    let artwork_hashes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT artwork_hash FROM track WHERE library_id = ? AND artwork_hash IS NOT NULL",
    )
    .bind(library_id.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(artwork_hashes.len(), 1);
    let artwork_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(artwork_rows, 1);

    std::fs::write(music.join("Native DSD.lrc"), "[00:02]Updated DSD words").unwrap();
    run_scan(&state, owner, library).await;
    let dsd_lyrics = state.services.lyrics(owner, dsd.id).await.unwrap();
    assert_eq!(dsd_lyrics.structured_lyrics[0].lines[0].start, Some(2_000));
    assert_eq!(
        dsd_lyrics.structured_lyrics[0].lines[0].value,
        "Updated DSD words"
    );
}

#[tokio::test]
async fn compilation_and_multi_artist_materialization_is_deterministic() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("metadata", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("metadata");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Metadata",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 2, false).await.unwrap();

    for (index, artist) in ["Alpha; Beta", "Gamma"].into_iter().enumerate() {
        let outcome = state
            .db
            .apply_catalog_track(
                library_id,
                scan_id,
                &catalog_input(index, artist),
                None,
                false,
            )
            .await
            .unwrap();
        assert_eq!(outcome, ApplyOutcome::Added);
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let albums: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM album WHERE library_id = ? AND title = 'Shared compilation'",
    )
    .bind(library_id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(albums, 1);
    let album_row =
        sqlx::query("SELECT album_artist_name, is_compilation FROM album WHERE library_id = ?")
            .bind(library_id.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(
        album_row.get::<String, _>("album_artist_name"),
        "Various Artists"
    );
    assert_eq!(album_row.get::<i64, _>("is_compilation"), 1);

    let first_track_artists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM track_participant tp JOIN track t ON t.id = tp.track_id \
         WHERE t.library_id = ? AND tp.role = 'artist' \
           AND t.relative_path = 'track-0.flac'",
    )
    .bind(library_id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(first_track_artists, 2);
    let genres: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM genre WHERE library_id = ?")
        .bind(library_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(genres, 2);
    assert_eq!(
        state
            .db
            .search_tracks_for_user(owner, library_id, "Beta")
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn ffmpeg_generated_catalog_format_matrix_is_indexed() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("matrix", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("format-matrix");
    std::fs::create_dir_all(&music).unwrap();
    for (extension, codec) in [
        ("mp3", "libmp3lame"),
        ("flac", "flac"),
        ("m4a", "aac"),
        ("ogg", "libvorbis"),
        ("wav", "pcm_s16le"),
        // AIFF arrives with the core bump: the desktop reads it through
        // symphonia's RIFF crate now, and this server reads the same list.
        ("aiff", "pcm_s16be"),
    ] {
        generate_audio_fixture(&music.join(format!("matrix.{extension}")), codec, extension);
    }
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Matrix", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Matrix".into(),
            root_path: root,
        },
    )
    .await;

    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 6);

    // AIFF is asserted apart, and on less. FFmpeg's AIFF muxer writes the
    // title and stops — `ffprobe` reports no album and no artist on the file
    // it just produced — so demanding them here would test the fixture
    // generator rather than the scanner. What the bump actually changed is
    // that the extension is admitted at all, and that is what this checks.
    let aiff = tracks
        .iter()
        .find(|track| track.relative_path == "matrix.aiff")
        .expect("aiff is admitted by the extension list");
    assert_eq!(aiff.title, "Matrix aiff");
    assert!(aiff.duration_ms > 0);
    let aiff_bytes = std::fs::read(music.join("matrix.aiff")).unwrap();
    assert_eq!(
        aiff.full_hash,
        blake3::hash(&aiff_bytes).to_hex().to_string()
    );

    for extension in ["mp3", "flac", "m4a", "ogg", "wav"] {
        let track = tracks
            .iter()
            .find(|track| track.relative_path == format!("matrix.{extension}"))
            .unwrap_or_else(|| panic!("missing {extension} from matrix"));
        assert_eq!(track.title, format!("Matrix {extension}"));
        assert_eq!(track.album.as_deref(), Some("WaveFlow format matrix"));
        assert_eq!(track.artist.as_deref(), Some("Alpha; Beta"));
        assert!(track.duration_ms > 0);

        // full_hash is published to clients as *the* reconciliation key, so its
        // algorithm is part of the contract. Check the served value against the
        // file rather than trusting the column name: a client computing BLAKE3
        // locally and getting something else would match nothing, silently.
        let bytes = std::fs::read(music.join(format!("matrix.{extension}"))).unwrap();
        assert_eq!(
            track.full_hash,
            blake3::hash(&bytes).to_hex().to_string(),
            "full_hash must be unkeyed BLAKE3 over the whole {extension} file"
        );
        assert_eq!(track.full_hash.len(), 64);
    }
}

/// A scan can be told to ignore what it already knows.
///
/// The skip is unconditional today, and it is right: it is why rescanning a
/// large library costs seconds. But nothing could ask for the work to be done
/// again, and a change to how the catalogue derives its identifiers needs
/// exactly that — the files have not moved, only the meaning of the rows has.
#[tokio::test]
async fn a_full_scan_reads_what_an_ordinary_one_would_skip() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("full-scan", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("full-scan-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Only Track.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Full", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Full".into(),
        root_path: root.clone(),
    };

    let job_of = |scan_id: uuid::Uuid| {
        let state = state.clone();
        async move {
            state
                .db
                .scan_job_for_user(owner, scan_id)
                .await
                .unwrap()
                .unwrap()
        }
    };

    let first = scan_once(&state, owner, library.clone()).await;
    assert_eq!(job_of(first).await.added, 1);

    // The second run recognises the file and does nothing, which is the
    // behaviour worth keeping.
    let second = scan_once(&state, owner, library.clone()).await;
    let second = job_of(second).await;
    assert_eq!(second.skipped, 1);
    assert_eq!(second.updated, 0);

    // Asking changes that, on a file that has not moved by a single byte.
    assert!(!state.db.full_scan_requested(library_id).await.unwrap());
    state.db.request_full_scan_everywhere().await.unwrap();
    assert!(state.db.full_scan_requested(library_id).await.unwrap());
    let third = scan_once(&state, owner, library.clone()).await;
    let third = job_of(third).await;
    assert_eq!(third.skipped, 0, "a full scan skips nothing");
    assert_eq!(third.updated, 1);

    // And the request is spent, so the next run is ordinary again.
    assert!(!state.db.full_scan_requested(library_id).await.unwrap());
    let fourth = scan_once(&state, owner, library).await;
    assert_eq!(job_of(fourth).await.skipped, 1);
}

/// The request outlives a run that does not finish.
///
/// This is the whole reason it is a stored state rather than an argument. A
/// migration scan interrupted halfway has rewritten some rows under the new
/// scheme and left the rest under the old one; if the request died with the
/// run, the next scan would skip every remaining file — on the grounds that
/// their bytes had not changed — and freeze the catalogue in two halves.
#[tokio::test]
async fn a_full_scan_request_survives_a_scan_that_never_completes() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("interrupted", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("interrupted-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Interrupted",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    state.db.request_full_scan_everywhere().await.unwrap();
    let failed = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(failed, 1, true).await.unwrap();
    state.db.fail_scan_job(failed, "interrupted").await.unwrap();
    assert!(
        state.db.full_scan_requested(library_id).await.unwrap(),
        "a failed run leaves the request standing"
    );

    // Nor can a run that started before the request arrived: it read the
    // catalogue under the old rules, so completing it says nothing about the
    // new ones.
    let ordinary = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(ordinary, 1, false).await.unwrap();
    state.db.finish_scan_job(ordinary, 0).await.unwrap();
    assert!(
        state.db.full_scan_requested(library_id).await.unwrap(),
        "an ordinary run cannot spend a request it never honoured"
    );

    // Only a completed full run spends it.
    let completed = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(completed, 1, true).await.unwrap();
    state.db.finish_scan_job(completed, 0).await.unwrap();
    assert!(!state.db.full_scan_requested(library_id).await.unwrap());
}

#[tokio::test]
async fn a_re_encoded_file_that_moved_keeps_its_track_and_its_favourite() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("relocator", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("relocation-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Session Take.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Relocation library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Relocation library".into(),
        root_path: root.clone(),
    };

    run_scan(&state, owner, library.clone()).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    let original_id = tracks[0].id;
    state
        .services
        .set_star(owner, "track", original_id, true)
        .await
        .unwrap();

    // A row as an upgraded instance carries it: scanned before the column
    // existed, so it has no hint. An ordinary scan changes nothing about the
    // file, takes the skip path, and must fill it anyway — otherwise the hint
    // would only ever help catalogues built after the upgrade.
    sqlx::query("UPDATE track SET pid = NULL WHERE id = ?")
        .bind(original_id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    run_scan(&state, owner, library.clone()).await;
    let backfilled: Option<String> = sqlx::query_scalar("SELECT pid FROM track WHERE id = ?")
        .bind(original_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(
        backfilled.is_some(),
        "a scan that skipped the file still owes it a hint"
    );

    // Moved and re-encoded at once: neither the path nor the content hash can
    // recognise it, and only the tags are left to say the two files are one.
    std::fs::remove_file(music.join("Session Take.wav")).unwrap();
    std::fs::create_dir_all(music.join("remastered")).unwrap();
    write_test_wav_of_len(&music.join("remastered/Session Take.wav"), 1_600);

    run_scan(&state, owner, library).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1, "the file must not land as a second track");
    assert_eq!(
        tracks[0].id, original_id,
        "the relocation hint must carry the identity across the re-encode"
    );
    assert!(tracks[0].available);
    assert_eq!(tracks[0].relative_path, "remastered/Session Take.wav");

    let starred = state.services.starred(owner, &[]).await.unwrap();
    assert_eq!(
        starred.songs.iter().map(|song| song.id).collect::<Vec<_>>(),
        vec![original_id],
        "a favourite must survive the move it was never told about"
    );
}

/// Sets a file's modification time back far enough to clear `WRITE_GRACE`.
///
/// The sweep leaves a young file alone whatever the database says, which is
/// what stands in for the lock it cannot take over `waveflow_core`'s writer. A
/// test that wants to see a removal has to age the file rather than wait an
/// hour for it.
fn age_beyond_the_grace(path: &std::path::Path) {
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
}

/// Fails if anything has touched a file since [`age_beyond_the_grace`] set it
/// back.
///
/// `extract_cover` spawns a detached thread that writes `<hash>_1x.jpg` and
/// `<hash>_2x.jpg` into the same directory, and it writes them under the names
/// this test uses. It produces nothing for these fixtures — the store holds the
/// cover alone, checked at two seconds — so the files below are this test's
/// alone. Depending on that silently is the problem: if the thread ever did
/// write, it would restore a fresh timestamp and the sweep would spare a file
/// the test expects it to take, intermittently and for a reason nothing states.
/// So the ages are read back where they matter, and interference fails loudly
/// on the pass it happens rather than flaking on some later one.
fn assert_still_aged(path: &std::path::Path) {
    let age = std::fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .elapsed()
        .unwrap();
    assert!(
        age > std::time::Duration::from_secs(60 * 60),
        "{} was touched after it was aged: something else writes this name",
        path.display()
    );
}

/// The one name in the store the sweep must never take: a cover in use.
///
/// `artwork_dir` had no sweep at all, so the first thing worth proving is not
/// that the new one collects — it is that it collects nothing. A pass over a
/// freshly scanned library must leave every byte where it is, and must not
/// report the store as broken while doing it.
#[tokio::test]
async fn the_artwork_sweep_takes_nothing_a_library_still_names() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("sweep-live", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("sweep-live");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("One.wav"));
    write_test_png(&music.join("cover.png"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Live", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Live".into(),
            root_path: root,
        },
    )
    .await;

    // The name comes from the scan, never from this test: `extract_cover`
    // chooses it, and a fixture that spelled it independently would agree with
    // itself while the server did something else.
    let cover = std::fs::read_dir(&config.artwork_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "png"))
        .expect("the scan wrote a cover");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(stored, 1);

    // Aged deliberately: the grace period must not be what saves it here, or
    // this test would pass on a sweep that takes live covers a day later.
    age_beyond_the_grace(&cover);
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.rows_removed, 0);
    assert_eq!(swept.covers_removed, 0);
    assert_eq!(swept.dead_links, 0, "the row and the file agree");
    assert!(cover.exists(), "a cover in use is not the sweep's to take");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artwork")
            .fetch_one(state.db.pool())
            .await
            .unwrap(),
        1
    );

    // A name the sweep cannot read is left where it is and said out loud. The
    // store sits under the operator's `data/`, and the alternative — deleting
    // the unrecognised — is how a sweep eats something it was never told about.
    let stranger = config.artwork_dir.join("operator-notes.txt");
    std::fs::write(&stranger, b"not mine to remove").unwrap();
    age_beyond_the_grace(&stranger);
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.unknown, 1);
    assert_eq!(swept.covers_removed, 0);
    assert!(stranger.exists());
}

/// A cover nothing names any more goes, and takes its thumbnails with it.
///
/// This is the property `artwork_dir` never had. Three columns are
/// `ON DELETE SET NULL`, so a library that is deleted or rescanned returns them
/// to `NULL` and tells nobody — the row stays, the bytes stay, and both stay
/// for the life of the instance.
#[tokio::test]
async fn an_unreferenced_cover_and_its_thumbnails_go_once_they_are_old_enough() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("sweep-dead", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("sweep-dead");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("One.wav"));
    write_test_png(&music.join("cover.png"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Dead", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Dead".into(),
            root_path: root,
        },
    )
    .await;

    let stored_hash: String = sqlx::query_scalar("SELECT hash FROM artwork")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    let cover = std::fs::read_dir(&config.artwork_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "png"))
        .expect("the scan wrote a cover");

    // The thumbnail names come from `waveflow_core`, which is what writes them,
    // rather than from a convention spelled out here — a fixture that spelled
    // `_1x` itself would agree with itself while the library did something else.
    //
    // The bytes are written here rather than waited for, because the real job is
    // a detached thread with no handle to join: `spawn_thumbnail_job` returns
    // `()`. `assert_still_aged` is what keeps that from being a silent bet.
    let thumbnails: Vec<std::path::PathBuf> = [
        waveflow_core::artwork::thumbnails::THUMB_SMALL,
        waveflow_core::artwork::thumbnails::THUMB_MEDIUM,
    ]
    .into_iter()
    .map(|size| {
        let path = waveflow_core::artwork::thumbnails::thumbnail_path(
            &config.artwork_dir,
            &stored_hash,
            size,
        );
        std::fs::write(&path, b"thumbnail bytes").unwrap();
        path
    })
    .collect();

    // What deleting an album does, without deleting an album: the columns go
    // back to NULL and nothing anywhere is told.
    // Spelled out three times rather than looped over a table name: sqlx takes
    // static SQL only, which is the same rule that keeps the projection macros
    // injection-proof by construction.
    for statement in [
        "UPDATE track SET artwork_hash = NULL",
        "UPDATE album SET artwork_hash = NULL",
        "UPDATE artist SET artwork_hash = NULL",
    ] {
        sqlx::query(statement)
            .execute(state.db.pool())
            .await
            .unwrap();
    }

    // The row goes at once — it is a database fact under the writer gate, with
    // no window to wait out. The files do not, because they are young, and the
    // sweep cannot tell a cover abandoned a second ago from one a scan is at
    // this moment about to name.
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.rows_removed, 1);
    assert_eq!(swept.covers_removed, 0, "the grace period holds the bytes");
    assert_eq!(swept.thumbnails_removed, 0);
    assert!(cover.exists());

    age_beyond_the_grace(&cover);
    for thumbnail in &thumbnails {
        age_beyond_the_grace(thumbnail);
    }
    assert_still_aged(&cover);
    for thumbnail in &thumbnails {
        assert_still_aged(thumbnail);
    }
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.rows_removed, 0, "the row went on the pass before");
    assert_eq!(swept.covers_removed, 1);
    assert_eq!(
        swept.thumbnails_removed, 2,
        "a thumbnail is named by nothing, so only its stem can save it"
    );
    assert!(!cover.exists());
    for thumbnail in &thumbnails {
        assert!(!thumbnail.exists());
    }

    // The other direction, which is reported and never repaired: a row naming a
    // file the store does not hold. Deleting the row would answer a cover that
    // fails to load by removing the album's art outright.
    sqlx::query(
        "INSERT INTO artwork (hash, format, source, byte_size, created_at) \
         VALUES (?, 'png', 'embedded', 1, ?)",
    )
    .bind("b".repeat(64))
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    sqlx::query("UPDATE track SET artwork_hash = ?")
        .bind("b".repeat(64))
        .execute(state.db.pool())
        .await
        .unwrap();
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.dead_links, 1);
    assert_eq!(swept.rows_removed, 0, "a named row is not an orphan");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artwork")
            .fetch_one(state.db.pool())
            .await
            .unwrap(),
        1,
        "the dead link is reported, not repaired"
    );
}

/// The event stream speaks one shape, and every counter lands where it belongs.
///
/// `/api/v2/scans/{id}/events` opens with a `snapshot` carrying a
/// `ScanJobRecord` and then sends a `progress` per step. Those frames used to
/// be two different types: `progress` carried the scanner's own `ScanProgress`,
/// which names `id`, `total_files` and `processed_files` as `scan_id`, `total`
/// and `processed`. A watcher bound to the snapshot read `undefined` from the
/// first progress frame on, while the ten counters that happen to share a name
/// kept working — which is why it read as a display bug rather than a shape.
///
/// That the frames agree is now a compile-time fact: `ScanProgress` is no
/// longer `Serialize`, so the handler cannot yield it and has to convert. What
/// the compiler cannot check is that the conversion puts each number where it
/// belongs — eleven of the thirteen fields are `i64` or `Uuid`, so a transposed
/// pair type-checks perfectly. That is what this pins.
#[test]
fn scan_progress_converts_into_the_shape_the_stream_sends() {
    let scan = uuid::Uuid::new_v4();
    let library = uuid::Uuid::new_v4();
    let record = waveflow_server::scanner::ScanProgress {
        scan_id: scan,
        library_id: library,
        status: "running".into(),
        total: 97,
        processed: 61,
        added: 1,
        updated: 2,
        moved: 3,
        skipped: 4,
        unavailable: 5,
        errors: 6,
        excluded: 7,
        skipped_links: 8,
        current_path: Some("Rue Delacour/Nocturne.flac".into()),
        message: Some("still reading".into()),
    }
    .record();

    // Distinct values throughout, so a swap between any two shows up here
    // rather than in a client six months later.
    assert_eq!(
        record.id, scan,
        "the scan's own id, under the snapshot's name"
    );
    assert_eq!(record.library_id, library);
    assert_eq!(record.status, "running");
    assert_eq!(record.total_files, 97);
    assert_eq!(record.processed_files, 61);
    assert_eq!(record.added, 1);
    assert_eq!(record.updated, 2);
    assert_eq!(record.moved, 3);
    assert_eq!(record.skipped, 4);
    assert_eq!(record.unavailable, 5);
    assert_eq!(record.errors, 6);
    assert_eq!(record.excluded, 7);
    assert_eq!(record.skipped_links, 8);
    assert_eq!(
        record.current_path.as_deref(),
        Some("Rue Delacour/Nocturne.flac")
    );
    assert_eq!(record.message.as_deref(), Some("still reading"));

    // And the names themselves, since they are the whole point: a client binds
    // these fifteen and nothing else — the last two RFC-021's.
    let json = serde_json::to_value(&record).unwrap();
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "added",
            "current_path",
            "errors",
            "excluded",
            "id",
            "library_id",
            "message",
            "moved",
            "processed_files",
            "skipped",
            "skipped_links",
            "status",
            "total_files",
            "unavailable",
            "updated",
        ]
    );
}

// ── An incremental scan that reads only what changed (RFC-018) ──

/// A library of one owner's, holding the files `files` writes into its root.
async fn incremental_library(
    name: &str,
    files: impl FnOnce(&std::path::Path),
) -> (
    tempfile::TempDir,
    waveflow_server::AppState,
    uuid::Uuid,
    LibraryRecord,
    std::path::PathBuf,
) {
    let (temp, config, state) = test_app().await;
    // Inserted rather than created: a scan needs an owner, not a login.
    let owner = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(owner.to_string())
    .bind(name)
    .bind(now_ms())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let music = config.data_dir.join(name);
    std::fs::create_dir_all(&music).unwrap();
    files(&music);
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, name, &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: name.into(),
        root_path: root.clone(),
    };
    (temp, state, owner, library, root)
}

async fn job(
    state: &waveflow_server::AppState,
    owner: uuid::Uuid,
    scan: uuid::Uuid,
) -> waveflow_server::catalog::ScanJobRecord {
    state
        .db
        .scan_job_for_user(owner, scan)
        .await
        .unwrap()
        .unwrap()
}

async fn full_hash(state: &waveflow_server::AppState) -> Option<String> {
    sqlx::query_scalar("SELECT full_hash FROM track")
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

async fn sidecar_signature(state: &waveflow_server::AppState) -> Option<String> {
    sqlx::query_scalar("SELECT sidecar_signature FROM track")
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

/// Rewrites `path` in place with different bytes of the same length, then
/// puts its modification time back: a change only reading the file can see.
fn rewrite_keeping_size_and_time(path: &std::path::Path) {
    let modified = std::fs::metadata(path).unwrap().modified().unwrap();
    let mut bytes = std::fs::read(path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x5a;
    std::fs::write(path, &bytes).unwrap();
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
}

/// RFC-018 decision 1: a file whose size, modification time and sidecars are
/// unchanged is skipped without being read. A change only its bytes show —
/// same size, same time — therefore goes unseen until a full scan reads it,
/// which is the trade the RFC states.
#[tokio::test]
async fn an_unchanged_file_is_skipped_without_being_read() {
    let (_temp, state, owner, library, root) = incremental_library("incremental-skip", |music| {
        write_test_wav(&music.join("Only.wav"))
    })
    .await;
    let first = scan_once(&state, owner, library.clone()).await;
    assert_eq!(job(&state, owner, first).await.added, 1);
    let hash = full_hash(&state).await;

    rewrite_keeping_size_and_time(&root.join("Only.wav"));
    let second = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(second.skipped, 1);
    assert_eq!(second.updated, 0);
    assert_eq!(
        full_hash(&state).await,
        hash,
        "the file was read: its new hash was recorded"
    );

    state.db.request_full_scan_everywhere().await.unwrap();
    let full = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(full.updated, 1);
    assert_ne!(full_hash(&state).await, hash);
}

/// A sidecar added, edited or removed beside an unchanged audio file is still
/// read: the signature it is compared on covers both sidecars, absent ones
/// included.
#[tokio::test]
async fn a_sidecar_changed_beside_an_unchanged_file_is_read() {
    let (_temp, state, owner, library, root) =
        incremental_library("incremental-sidecar", |music| {
            write_test_wav(&music.join("Song.wav"))
        })
        .await;
    scan_once(&state, owner, library.clone()).await;
    assert_eq!(
        sidecar_signature(&state).await.as_deref(),
        Some("lrc:-|txt:-")
    );

    std::fs::write(root.join("Song.lrc"), "[00:01.00]A line\n").unwrap();
    let added = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(added.skipped, 0, "the new sidecar was not read");
    assert_eq!(added.updated, 1);

    let unchanged = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(unchanged.skipped, 1);

    std::fs::remove_file(root.join("Song.lrc")).unwrap();
    let removed = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(removed.skipped, 0, "the removed sidecar went unnoticed");
    assert_eq!(removed.updated, 1);
}

/// `NULL` means never computed and never matches: right after the migration
/// every file is read once, which fills the signature, and the scan after it
/// skips.
#[tokio::test]
async fn a_signature_never_computed_is_filled_by_the_next_scan() {
    let (_temp, state, owner, library, _root) = incremental_library("incremental-null", |music| {
        write_test_wav(&music.join("Only.wav"))
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    sqlx::query("UPDATE track SET sidecar_signature = NULL")
        .execute(state.db.pool())
        .await
        .unwrap();

    scan_once(&state, owner, library.clone()).await;
    assert_eq!(
        sidecar_signature(&state).await.as_deref(),
        Some("lrc:-|txt:-"),
        "an unchanged file read for want of a signature must record one"
    );
    let next = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(next.skipped, 1);
}

/// RFC-018 decision 3: a folder the walk cannot read costs that folder, not
/// the scan, and its tracks are not marked missing. Unix only: it is the one
/// place a test can take a folder's permissions away.
#[cfg(unix)]
#[tokio::test]
async fn an_unreadable_folder_costs_that_folder_only() {
    use std::os::unix::fs::PermissionsExt;

    let (_temp, state, owner, library, root) =
        incremental_library("incremental-unreadable", |music| {
            std::fs::create_dir_all(music.join("Readable")).unwrap();
            std::fs::create_dir_all(music.join("Locked")).unwrap();
            write_test_wav(&music.join("Readable/One.wav"));
            write_test_wav_of_len(&music.join("Locked/Two.wav"), 900);
        })
        .await;
    let first = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(first.added, 2);

    /// Puts the permissions back however the test ends, or the temporary
    /// directory could not be removed.
    struct Unlock(std::path::PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    let locked = root.join("Locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unlock = Unlock(locked.clone());
    if std::fs::read_dir(&locked).is_ok() {
        // Root reads through `chmod 000`: nothing here can be made unreadable.
        eprintln!("skipped: the folder stays readable to this user");
        return;
    }
    let second = scan_once(&state, owner, library.clone()).await;
    drop(unlock);

    let second = job(&state, owner, second).await;
    assert_eq!(second.errors, 1);
    assert_eq!(
        second.unavailable, 0,
        "a track under the unreadable folder was marked missing"
    );
    let available: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM track WHERE is_available = 1")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(available, 2);
}

/// When everything a library holds sits under the unreadable folder, the walk
/// finds no file and the scan runs no batch: the folder's error must still be
/// counted on the job, and its track still not marked missing.
#[cfg(unix)]
#[tokio::test]
async fn an_unreadable_folder_holding_everything_is_still_counted() {
    use std::os::unix::fs::PermissionsExt;

    struct Unlock(std::path::PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    let (_temp, state, owner, library, root) =
        incremental_library("incremental-all-locked", |music| {
            std::fs::create_dir_all(music.join("Locked")).unwrap();
            write_test_wav(&music.join("Locked/Only.wav"));
        })
        .await;
    assert_eq!(
        job(
            &state,
            owner,
            scan_once(&state, owner, library.clone()).await
        )
        .await
        .added,
        1
    );

    let locked = root.join("Locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unlock = Unlock(locked.clone());
    if std::fs::read_dir(&locked).is_ok() {
        eprintln!("skipped: the folder stays readable to this user");
        return;
    }
    let second = scan_once(&state, owner, library).await;
    drop(unlock);

    let second = job(&state, owner, second).await;
    assert_eq!(
        second.errors, 1,
        "the folder's error was not recorded on the job"
    );
    assert_eq!(second.unavailable, 0);
}

/// A sidecar the scan can list but not open is read past, without its lyrics,
/// and no signature is recorded: once the permission is fixed — which moves
/// neither its size nor its time — the next scan reads it. Unix only, for the
/// permissions.
#[cfg(unix)]
#[tokio::test]
async fn a_sidecar_that_cannot_be_opened_is_read_again_once_it_can() {
    use std::os::unix::fs::PermissionsExt;

    struct Unlock(std::path::PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o644));
        }
    }

    let (_temp, state, owner, library, root) =
        incremental_library("incremental-locked-lyrics", |music| {
            write_test_wav(&music.join("Song.wav"));
            std::fs::write(music.join("Song.lrc"), "[00:01.00]A line\n").unwrap();
        })
        .await;
    let sidecar = root.join("Song.lrc");
    std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unlock = Unlock(sidecar.clone());
    if std::fs::File::open(&sidecar).is_ok() {
        eprintln!("skipped: the sidecar stays readable to this user");
        return;
    }
    scan_once(&state, owner, library.clone()).await;
    assert_eq!(sidecar_signature(&state).await, None);

    drop(unlock);
    let again = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(again.skipped, 0, "the now-readable sidecar was never read");
    assert_eq!(again.updated, 1);
    assert!(sidecar_signature(&state).await.is_some());
}

/// A sidecar that is a FIFO with no writer does not hang the scan: the
/// signature never opens anything but a regular file, and the read never
/// blocks. `scan_once` gives up after two seconds, so a hang fails here.
#[cfg(unix)]
#[tokio::test]
async fn a_fifo_beside_an_audio_file_does_not_hang_the_scan() {
    let (_temp, state, owner, library, _root) = incremental_library("incremental-fifo", |music| {
        write_test_wav(&music.join("Song.wav"));
        let fifo = std::ffi::CString::new(
            music
                .join("Song.lrc")
                .to_str()
                .expect("a UTF-8 temporary path"),
        )
        .unwrap();
        // SAFETY: a valid, NUL-terminated path and an ordinary mode.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    })
    .await;
    let first = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(first.added, 1);
    assert_eq!(
        sidecar_signature(&state).await.as_deref(),
        Some("lrc:-|txt:-")
    );
    let second = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(second.skipped, 1);
}

// ── Libraries and scanning: exclusions, links, missing tracks (RFC-021) ──

/// An account inserted straight into the table: these tests need an owner, a
/// manager or a listener, never a login.
async fn insert_account(state: &waveflow_server::AppState, name: &str, role: &str) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', ?, 0, ?, ?)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(role)
    .bind(now_ms())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    id
}

async fn count(state: &waveflow_server::AppState, query: &'static str) -> i64 {
    sqlx::query_scalar(query)
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

async fn available_paths(state: &waveflow_server::AppState) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT relative_path FROM track WHERE is_available = 1 ORDER BY relative_path",
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap()
}

/// Waits for one job to end, whichever way.
async fn settled(
    state: &waveflow_server::AppState,
    owner: uuid::Uuid,
    scan: uuid::Uuid,
) -> waveflow_server::catalog::ScanJobRecord {
    for _ in 0..500 {
        let record = job(state, owner, scan).await;
        if record.status == "completed" || record.status == "failed" {
            return record;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("scan timed out");
}

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

/// A FLAC file tagged with a title, an album and an artist, its tone chosen by
/// `frequency` so no two fixtures share their bytes.
fn write_tagged_flac(path: &std::path::Path, title: &str, album: &str, frequency: usize) {
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={frequency}:duration=0.1"),
            "-metadata",
            &format!("title={title}"),
            "-metadata",
            &format!("album={album}"),
            "-metadata",
            "artist=Lifecycle Artist",
            "-c:a",
            "flac",
        ])
        .arg(path)
        .output()
        .expect("FFmpeg is required for tagged fixtures");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Two tracks in an album, one in another, scanned.
async fn library_with_albums(
    name: &str,
) -> (
    tempfile::TempDir,
    waveflow_server::AppState,
    uuid::Uuid,
    LibraryRecord,
    std::path::PathBuf,
) {
    let fixture = incremental_library(name, |music| {
        for (index, album) in ["First", "First", "Second"].into_iter().enumerate() {
            std::fs::create_dir_all(music.join(album)).unwrap();
            write_tagged_flac(
                &music.join(format!("{album}/{index}.flac")),
                &format!("Track {index}"),
                album,
                300 + index * 110,
            );
        }
    })
    .await;
    scan_once(&fixture.1, fixture.2, fixture.3.clone()).await;
    fixture
}

async fn track_id(state: &waveflow_server::AppState, path: &str) -> String {
    sqlx::query_scalar("SELECT id FROM track WHERE relative_path = ?")
        .bind(path)
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

/// RFC-021 decision 4: the listing says what a purge would cost, and the
/// purge removes only what is still missing, with what hangs off it, and
/// leaves no album, favourite or search row behind with nothing to stand on.
#[tokio::test]
async fn a_purge_removes_only_what_is_still_missing() {
    let (temp, state, owner, library, root) = library_with_albums("purge").await;
    let first = track_id(&state, "First/0.flac").await;
    let second = track_id(&state, "Second/2.flac").await;
    // A loop on the track that goes: its link cascades, and the blob it was
    // the last to name must go with it rather than sit in the store for ever.
    let canvas = "c".repeat(64);
    let canvas_file = temp
        .path()
        .join("data/canvas")
        .join(format!("{canvas}.mp4"));
    std::fs::create_dir_all(canvas_file.parent().unwrap()).unwrap();
    std::fs::write(&canvas_file, b"mp4").unwrap();
    sqlx::query("INSERT INTO canvas (hash, format, byte_size, created_at) VALUES (?, 'mp4', 3, 1)")
        .bind(&canvas)
        .execute(state.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO track_canvas (track_id, library_id, canvas_hash, created_at) \
         VALUES (?, ?, ?, 1)",
    )
    .bind(&second)
    .bind(library.id.to_string())
    .bind(&canvas)
    .execute(state.db.pool())
    .await
    .unwrap();
    let second_album: String = sqlx::query_scalar("SELECT album_id FROM track WHERE id = ?")
        .bind(&second)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    let playlist = uuid::Uuid::new_v4().to_string();
    for (statement, binds) in [
        (
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
             VALUES (?, 'track', ?, 1)",
            vec![owner.to_string(), second.clone()],
        ),
        (
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
             VALUES (?, 'album', ?, 1)",
            vec![owner.to_string(), second_album.clone()],
        ),
        (
            "INSERT INTO playlist (id, owner_user_id, name, public, created_at, updated_at) \
             VALUES (?, ?, 'Kept', 0, 1, 1)",
            vec![playlist.clone(), owner.to_string()],
        ),
        (
            "INSERT INTO playlist_track (playlist_id, track_id, position, added_at) \
             VALUES (?, ?, 0, 1)",
            vec![playlist.clone(), second.clone()],
        ),
        (
            "INSERT INTO play_event (user_id, track_id, submission, played_at) \
             VALUES (?, ?, 1, 1)",
            vec![owner.to_string(), second.clone()],
        ),
    ] {
        let mut query = sqlx::query(statement);
        for bind in binds {
            query = query.bind(bind);
        }
        query.execute(state.db.pool()).await.unwrap();
    }

    let first_bytes = std::fs::read(root.join("First/0.flac")).unwrap();
    std::fs::remove_file(root.join("First/0.flac")).unwrap();
    std::fs::remove_file(root.join("Second/2.flac")).unwrap();
    scan_once(&state, owner, library.clone()).await;

    let listed = state
        .services
        .unavailable_tracks(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(listed.total, 2);
    let costly = listed
        .tracks
        .iter()
        .find(|track| track.id.to_string() == second)
        .unwrap();
    assert_eq!(
        (costly.playlist_entries, costly.favorites, costly.plays),
        (1, 1, 1)
    );

    // One of them comes back between the listing and the purge.
    std::fs::write(root.join("First/0.flac"), first_bytes).unwrap();
    scan_once(&state, owner, library.clone()).await;

    let purged = state
        .services
        .purge_unavailable_tracks(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            waveflow_server::services::PurgeRequest {
                track_ids: Some(vec![first.parse().unwrap(), second.parse().unwrap()]),
                all: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(purged.purged, 1, "a track that came back must be kept");
    // Out of sight at once, deleted once the grace window has passed.
    assert!(purged.restorable_until.is_some());
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM track WHERE is_available = 0 AND purged_at IS NULL"
        )
        .await,
        0
    );
    assert_eq!(state.services.finish_purges(now_ms()).await.unwrap(), 1);
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE is_available = 0").await,
        0
    );
    assert_eq!(
        available_paths(&state).await,
        ["First/0.flac", "First/1.flac"]
    );
    for (query, what) in [
        ("SELECT COUNT(*) FROM playlist_track", "the playlist entry"),
        ("SELECT COUNT(*) FROM play_event", "the play"),
        ("SELECT COUNT(*) FROM user_star", "the favourites"),
        (
            "SELECT COUNT(*) FROM album \
             WHERE NOT EXISTS (SELECT 1 FROM track t WHERE t.album_id = album.id)",
            "an empty album",
        ),
        (
            "SELECT COUNT(*) FROM track_fts WHERE track_id NOT IN (SELECT id FROM track)",
            "a search row",
        ),
    ] {
        assert_eq!(count(&state, query).await, 0, "{what} outlived the purge");
    }
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM playlist").await,
        1,
        "the playlist itself is not the track's to take"
    );
    assert_eq!(count(&state, "SELECT COUNT(*) FROM canvas").await, 0);
    assert!(
        !canvas_file.exists(),
        "the purged track's canvas stayed in the store"
    );
}

/// RFC-021 decision 4: a purge is explicit, refused while the library is
/// scanned, and for curators only; the retention pass purges by age.
#[tokio::test]
async fn a_purge_is_explicit_and_waits_for_no_scan() {
    use waveflow_server::services::{PurgeRequest, ServiceError};

    let (_temp, state, owner, library, root) = library_with_albums("purge-rules").await;
    std::fs::remove_file(root.join("First/0.flac")).unwrap();
    std::fs::remove_file(root.join("Second/2.flac")).unwrap();
    scan_once(&state, owner, library.clone()).await;

    let neither = state
        .services
        .purge_unavailable_tracks(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            PurgeRequest::default(),
        )
        .await;
    assert!(
        matches!(neither, Err(ServiceError::Invalid)),
        "a purge naming nothing must be refused as invalid"
    );

    let listener = insert_account(&state, "purge-listener", "user").await;
    state
        .db
        .add_library_member(
            owner,
            library.id,
            listener,
            waveflow_server::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    let all = || PurgeRequest {
        track_ids: None,
        all: true,
    };
    assert!(matches!(
        state
            .services
            .purge_unavailable_tracks(
                waveflow_server::services::LibraryActor::operator(listener),
                library.id,
                all()
            )
            .await,
        Err(ServiceError::Forbidden)
    ));
    assert!(matches!(
        state
            .services
            .unavailable_tracks(
                waveflow_server::services::LibraryActor::operator(listener),
                library.id,
                Default::default()
            )
            .await,
        Err(ServiceError::Forbidden)
    ));

    let scanning = state.scanner.try_lock_library(library.id).unwrap();
    assert!(matches!(
        state
            .services
            .purge_unavailable_tracks(
                waveflow_server::services::LibraryActor::operator(owner),
                library.id,
                all()
            )
            .await,
        Err(ServiceError::Conflict)
    ));
    // During that scan, somebody who may not purge is refused as such, and a
    // stranger is told there is no library: a `Conflict` would say there is
    // one, and that it is being scanned.
    let stranger = insert_account(&state, "purge-stranger", "user").await;
    let refused = state
        .services
        .purge_unavailable_tracks(
            waveflow_server::services::LibraryActor::operator(stranger),
            library.id,
            all(),
        )
        .await;
    assert!(
        matches!(refused, Err(ServiceError::NotFound)),
        "a stranger must be told there is no library"
    );
    let refused = state
        .services
        .purge_unavailable_tracks(
            waveflow_server::services::LibraryActor::operator(listener),
            library.id,
            all(),
        )
        .await;
    assert!(
        matches!(refused, Err(ServiceError::Forbidden)),
        "a listener may see the library and may not purge it"
    );
    assert_eq!(
        state
            .services
            .purge_expired_unavailable_tracks(i64::MAX)
            .await
            .unwrap(),
        0,
        "the retention pass must leave a library being scanned alone"
    );
    drop(scanning);

    // One went missing long ago, the other just now.
    sqlx::query("UPDATE track SET unavailable_since = 1000 WHERE relative_path = 'First/0.flac'")
        .execute(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        state
            .services
            .purge_expired_unavailable_tracks(now_ms() - 60_000)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM track WHERE is_available = 0 AND purged_at IS NULL"
        )
        .await,
        1,
        "the retention purges like anyone else, behind the same grace window"
    );
}

/// RFC-021 decision 5: a root moved with its tree intact keeps every
/// identifier, and the scan it queues says why it ran.
#[tokio::test]
async fn a_moved_root_keeps_every_identity() {
    let (temp, state, owner, library, root) = library_with_albums("moved").await;
    let before: Vec<(String, String)> =
        sqlx::query_as("SELECT relative_path, id FROM track ORDER BY relative_path")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    let moved = temp.path().join("moved-elsewhere");
    std::fs::rename(&root, &moved).unwrap();

    let updated = state
        .services
        .update_library(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            waveflow_server::services::LibraryUpdate {
                path: Some(moved.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let scan = updated.scan_id.expect("a new root queues a scan");
    let record = settled(&state, owner, scan).await;
    assert_eq!(record.status, "completed", "{:?}", record.message);
    assert_eq!((record.added, record.unavailable), (0, 0));
    let trigger: String = sqlx::query_scalar("SELECT trigger FROM scan_job WHERE id = ?")
        .bind(scan.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(trigger, "library_moved");
    let after: Vec<(String, String)> = sqlx::query_as(
        "SELECT relative_path, id FROM track WHERE is_available = 1 ORDER BY relative_path",
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(after, before);
    let stored: String = sqlx::query_scalar("SELECT root_path FROM library WHERE id = ?")
        .bind(library.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        std::path::PathBuf::from(stored),
        std::fs::canonicalize(&moved).unwrap()
    );
}

/// RFC-021 decision 5: who may do what. The owner renames; the path, the
/// links setting and deletion are an administrator's; a listener does
/// nothing; and nothing that changes the walk happens during a scan.
#[tokio::test]
async fn the_lifecycle_follows_the_role_model() {
    use waveflow_server::services::{LibraryUpdate, ServiceError};

    let (temp, state, admin, library, _root) = library_with_albums("roles").await;
    let owner = insert_account(&state, "roles-owner", "user").await;
    let listener = insert_account(&state, "roles-listener", "user").await;
    sqlx::query("UPDATE library_member SET user_id = ? WHERE library_id = ?")
        .bind(owner.to_string())
        .bind(library.id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner,
            library.id,
            listener,
            waveflow_server::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    let rename = |name: &str| LibraryUpdate {
        name: Some(name.into()),
        ..Default::default()
    };

    state
        .services
        .update_library(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            rename("  Renamed  "),
        )
        .await
        .unwrap();
    let name: String = sqlx::query_scalar("SELECT name FROM library WHERE id = ?")
        .bind(library.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(name, "Renamed");
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(listener),
                library.id,
                rename("Mine now")
            )
            .await,
        Err(ServiceError::Forbidden)
    ));
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(owner),
                library.id,
                rename(" ")
            )
            .await,
        Err(ServiceError::Invalid)
    ));
    let elsewhere = temp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let repath = LibraryUpdate {
        path: Some(elsewhere.to_string_lossy().into_owned()),
        ..Default::default()
    };
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(owner),
                library.id,
                repath.clone()
            )
            .await,
        Err(ServiceError::Forbidden)
    ));
    assert!(matches!(
        state
            .services
            .delete_library(
                waveflow_server::services::LibraryActor::operator(owner),
                library.id,
                "Renamed"
            )
            .await,
        Err(ServiceError::Forbidden)
    ));
    let scanning = state.scanner.try_lock_library(library.id).unwrap();
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(admin),
                library.id,
                repath
            )
            .await,
        Err(ServiceError::Conflict)
    ));
    assert!(matches!(
        state
            .services
            .delete_library(
                waveflow_server::services::LibraryActor::operator(admin),
                library.id,
                "Renamed"
            )
            .await,
        Err(ServiceError::Conflict)
    ));
    drop(scanning);
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(admin),
                library.id,
                LibraryUpdate {
                    path: Some(temp.path().join("absent").to_string_lossy().into_owned()),
                    ..Default::default()
                }
            )
            .await,
        Err(ServiceError::Invalid)
    ));
    // Somebody else's library, by an outsider: as missing as no library.
    let outsider = insert_account(&state, "roles-outsider", "user").await;
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(outsider),
                library.id,
                rename("Ours")
            )
            .await,
        Err(ServiceError::NotFound)
    ));
}

/// RFC-021 decision 5: deleting a library is confirmed by its name, removes
/// every row it held and nothing on disk, and leaves the other libraries
/// alone.
#[tokio::test]
async fn a_deleted_library_leaves_no_row_behind() {
    use waveflow_server::services::ServiceError;

    let (temp, state, admin, library, root) = library_with_albums("deleted").await;
    let kept_root = temp.path().join("kept");
    std::fs::create_dir_all(&kept_root).unwrap();
    write_test_wav_of_len(&kept_root.join("Kept.wav"), 1234);
    let kept_root = std::fs::canonicalize(&kept_root).unwrap();
    let kept_id = state
        .db
        .create_library(
            admin,
            "kept",
            &kept_root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    scan_once(
        &state,
        admin,
        LibraryRecord {
            id: kept_id,
            name: "kept".into(),
            root_path: kept_root,
        },
    )
    .await;
    let doomed = track_id(&state, "First/0.flac").await;
    sqlx::query(
        "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
         VALUES (?, 'track', ?, 1)",
    )
    .bind(admin.to_string())
    .bind(&doomed)
    .execute(state.db.pool())
    .await
    .unwrap();

    assert!(matches!(
        state
            .services
            .delete_library(
                waveflow_server::services::LibraryActor::operator(admin),
                library.id,
                "Deleted"
            )
            .await,
        Err(ServiceError::Invalid)
    ));
    assert_eq!(count(&state, "SELECT COUNT(*) FROM library").await, 2);

    let deleted = state
        .services
        .delete_library(
            waveflow_server::services::LibraryActor::operator(admin),
            library.id,
            "deleted",
        )
        .await
        .unwrap();
    assert_eq!(deleted.tracks, 3);
    assert_eq!(deleted.albums, 2);

    for table in [
        "SELECT COUNT(*) FROM track WHERE library_id = ?",
        "SELECT COUNT(*) FROM album WHERE library_id = ?",
        "SELECT COUNT(*) FROM artist WHERE library_id = ?",
        "SELECT COUNT(*) FROM genre WHERE library_id = ?",
        "SELECT COUNT(*) FROM track_fts WHERE library_id = ?",
        "SELECT COUNT(*) FROM artist_fts WHERE library_id = ?",
        "SELECT COUNT(*) FROM library_member WHERE library_id = ?",
        "SELECT COUNT(*) FROM scan_job WHERE library_id = ?",
        "SELECT COUNT(*) FROM library_event WHERE library_id = ?",
    ] {
        let left: i64 = sqlx::query_scalar(table)
            .bind(library.id.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
        assert_eq!(left, 0, "{table}");
    }
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM user_star").await,
        0,
        "a favourite of a deleted track outlived it"
    );
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(state.db.pool())
        .await
        .unwrap();
    assert!(violations.is_empty());
    assert!(root.join("First/0.flac").exists(), "a file was touched");
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE is_available = 1").await,
        1,
        "the other library lost its track"
    );
    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_event WHERE kind = 'library.deleted' AND subject_id = ?",
    )
    .bind(library.id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(audited, 1);
}

/// RFC-021 decision 6: a change in a watched library is scanned once it
/// settles, by a scan that says it came from the watcher.
#[tokio::test]
async fn a_watched_change_is_scanned_once_it_settles() {
    let (_temp, state, owner, library, root) = incremental_library("watched", |music| {
        write_test_wav(&music.join("First.wav"));
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    let watcher = waveflow_server::watcher::spawn(
        state.scanner.clone(),
        state.db.clone(),
        std::time::Duration::from_millis(200),
    );
    // Long enough for the watch to be placed before the change it must see.
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    write_test_wav_of_len(&root.join("Second.wav"), 900);

    let mut watched = None;
    for _ in 0..300 {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT added FROM scan_job WHERE trigger = 'watch' AND status = 'completed'",
        )
        .fetch_optional(state.db.pool())
        .await
        .unwrap();
        if found.is_some() {
            watched = found;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    watcher.abort();
    assert_eq!(watched, Some(1), "no scan followed the change");
    assert_eq!(available_paths(&state).await, ["First.wav", "Second.wav"]);
}

/// RFC-021 decision 5: a scan queued before a move walks the library as it is
/// when it runs, not as it was when it was queued — the watcher and the
/// interval queue scans that a move can overtake.
#[tokio::test]
async fn a_queued_scan_reads_the_library_as_it_is_when_it_runs() {
    let (temp, state, owner, library, root) = library_with_albums("queued").await;
    let held = state.scanner.try_lock_library(library.id).unwrap();
    // Queued with the old root, and waiting for the lock.
    let queued = state
        .scanner
        .trigger(library.clone(), Some(owner), "manual")
        .await
        .unwrap();
    let moved = temp.path().join("queued-elsewhere");
    std::fs::rename(&root, &moved).unwrap();
    sqlx::query("UPDATE library SET root_path = ? WHERE id = ?")
        .bind(
            std::fs::canonicalize(&moved)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        )
        .bind(library.id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    drop(held);
    let record = settled(&state, owner, queued).await;
    assert_eq!(record.status, "completed", "{:?}", record.message);
    assert_eq!((record.added, record.unavailable), (0, 0));
}

/// RFC-021 decision 6: a settled change whose scan cannot be queued stays
/// pending, rather than waiting for the periodic scan — which an interval of
/// zero turns off. A library the catalogue no longer holds refuses the job, the
/// simplest failure a test can provoke.
#[tokio::test]
async fn a_scan_the_watcher_cannot_queue_stays_pending() {
    let (_temp, _config, state) = test_app().await;
    let ghost = uuid::Uuid::new_v4();
    let watched = std::collections::HashMap::from([(
        ghost,
        LibraryRecord {
            id: ghost,
            name: "ghost".into(),
            root_path: std::path::PathBuf::from("/nowhere"),
        },
    )]);
    let delay = std::time::Duration::from_millis(10);
    let mut settle = waveflow_server::watcher::Settle::new(delay);
    let start = tokio::time::Instant::now();
    settle.touch(ghost, start);
    waveflow_server::watcher::dispatch_due(&mut settle, &watched, &state.scanner, start + delay)
        .await;
    assert!(
        settle.is_pending(ghost),
        "a change whose scan could not be queued was dropped"
    );
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

/// RFC-021 decision 5: a new root is never stored without the scan it owes.
/// Here the job cannot be queued — a trigger refuses it — and the root must
/// stay where it was, rather than commit and wait for a scan nobody queued.
#[tokio::test]
async fn a_move_whose_scan_cannot_be_queued_is_not_stored() {
    let (temp, state, owner, library, root) = library_with_albums("move-unqueued").await;
    sqlx::query(
        "CREATE TRIGGER refuse_moved_scans BEFORE INSERT ON scan_job \
         WHEN NEW.trigger = 'library_moved' \
         BEGIN SELECT RAISE(ABORT, 'no scan for you'); END",
    )
    .execute(state.db.pool())
    .await
    .unwrap();
    let elsewhere = temp.path().join("move-unqueued-elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();

    let refused = state
        .services
        .update_library(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            waveflow_server::services::LibraryUpdate {
                path: Some(elsewhere.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await;
    assert!(refused.is_err(), "{refused:?}");
    let stored: String = sqlx::query_scalar("SELECT root_path FROM library WHERE id = ?")
        .bind(library.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        std::path::PathBuf::from(stored),
        root,
        "the new root was stored without the scan it owes"
    );
    // Nor the record of a move that did not happen: the audit row is written
    // in the same transaction, and goes with it.
    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_event WHERE kind = 'library.moved' AND subject_id = ?",
    )
    .bind(library.id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(audited, 0, "a move that was refused was audited as made");
}

/// RFC-021 decision 5: a job a previous run committed and never started — it
/// stopped between the commit and the start — runs when the server boots, and
/// its library is not scanned a second time for the boot.
#[tokio::test]
async fn a_scan_a_previous_run_left_queued_runs_at_boot() {
    let (temp, state, owner, library, _root) = library_with_albums("left-queued").await;
    let other_root = temp.path().join("left-queued-other");
    std::fs::create_dir_all(&other_root).unwrap();
    let other = state
        .db
        .create_library(
            owner,
            "other",
            &std::fs::canonicalize(&other_root).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let left = state
        .db
        .create_scan_job(library.id, Some(owner), "library_moved")
        .await
        .unwrap();

    state.scanner.boot().await;

    assert_eq!(
        settled(&state, owner, left).await.status,
        "completed",
        "the queued job was never run"
    );
    let boots = |library: uuid::Uuid| {
        let state = state.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM scan_job WHERE library_id = ? AND trigger = 'boot'",
            )
            .bind(library.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap()
        }
    };
    assert_eq!(
        boots(library.id).await,
        0,
        "the resumed library was scanned twice"
    );
    assert_eq!(
        boots(other).await,
        1,
        "the other library missed its boot scan"
    );
    // Let the other library's boot scan finish before the directory goes.
    let other_job: String =
        sqlx::query_scalar("SELECT id FROM scan_job WHERE library_id = ? AND trigger = 'boot'")
            .bind(other.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    settled(&state, owner, other_job.parse().unwrap()).await;
}

/// The journal entries one account was written, as `(entity_type, entity_id,
/// action)`, oldest first.
async fn journal_of(
    state: &waveflow_server::AppState,
    user: uuid::Uuid,
) -> Vec<(String, String, String, serde_json::Value)> {
    sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT entity_type, entity_id, action, payload_json FROM sync_event \
         WHERE user_id = ? ORDER BY cursor",
    )
    .bind(user.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap()
    .into_iter()
    .map(|(kind, id, action, payload)| (kind, id, action, serde_json::from_str(&payload).unwrap()))
    .collect()
}

/// RFC-021, what was still open: a purge can be taken back within its grace
/// window — explicitly, or by the file coming back — and when it does land,
/// every account that held a favourite, a rating or a playlist entry of what
/// went is told through its own sync journal, as a library deletion tells
/// them too.
#[tokio::test]
async fn a_purge_can_be_taken_back_and_tells_every_journal_when_it_lands() {
    use waveflow_server::services::{LibraryActor, PurgeRequest};

    let (_temp, state, owner, library, root) = library_with_albums("undo").await;
    let actor = LibraryActor::operator(owner);
    let listener = insert_account(&state, "undo-listener", "user").await;
    state
        .db
        .add_library_member(
            owner,
            library.id,
            listener,
            waveflow_server::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    let doomed = track_id(&state, "Second/2.flac").await;
    let kept = track_id(&state, "First/1.flac").await;
    let doomed_album: String = sqlx::query_scalar("SELECT album_id FROM track WHERE id = ?")
        .bind(&doomed)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    for (statement, user, entity) in [
        (
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
             VALUES (?, 'track', ?, 1)",
            listener,
            doomed.clone(),
        ),
        (
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
             VALUES (?, 'album', ?, 1)",
            listener,
            doomed_album.clone(),
        ),
        (
            "INSERT INTO user_rating (user_id, entity_type, entity_id, rating, updated_at) \
             VALUES (?, 'track', ?, 4, 1)",
            owner,
            doomed.clone(),
        ),
    ] {
        sqlx::query(statement)
            .bind(user.to_string())
            .bind(entity)
            .execute(state.db.pool())
            .await
            .unwrap();
    }
    let playlist = state
        .services
        .create_playlist(
            listener,
            "Mixed",
            &[kept.parse().unwrap(), doomed.parse().unwrap()],
        )
        .await
        .unwrap();
    let journalled = |entries: &[(String, String, String, serde_json::Value)]| entries.len();
    let listener_before = journalled(&journal_of(&state, listener).await);
    let owner_before = journalled(&journal_of(&state, owner).await);

    let bytes = std::fs::read(root.join("Second/2.flac")).unwrap();
    std::fs::remove_file(root.join("Second/2.flac")).unwrap();
    scan_once(&state, owner, library.clone()).await;
    let only_doomed = || PurgeRequest {
        track_ids: Some(vec![doomed.parse().unwrap()]),
        all: false,
    };

    // Purged: out of the missing listing, into the purged one, nothing gone.
    let purged = state
        .services
        .purge_unavailable_tracks(actor, library.id, only_doomed())
        .await
        .unwrap();
    assert_eq!(purged.purged, 1);
    let until = purged.restorable_until.expect("a grace window by default");
    let missing = state
        .services
        .unavailable_tracks(actor, library.id, Default::default())
        .await
        .unwrap();
    assert_eq!(missing.total, 0);
    let held = state
        .services
        .purged_tracks(actor, library.id, Default::default())
        .await
        .unwrap();
    assert_eq!(held.total, 1);
    assert_eq!(held.tracks[0].id.to_string(), doomed);
    assert_eq!(held.tracks[0].deleted_after, Some(until));
    assert_eq!(held.tracks[0].favorites, 1, "nothing is lost yet");

    // Taken back explicitly.
    let restored = state
        .services
        .restore_purged_tracks(actor, library.id, only_doomed())
        .await
        .unwrap();
    assert_eq!(restored.restored, 1);
    assert_eq!(
        state
            .services
            .unavailable_tracks(actor, library.id, Default::default())
            .await
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        state.services.finish_purges(i64::MAX).await.unwrap(),
        0,
        "a restored track is no longer due"
    );

    // Taken back by the file coming back within the window.
    state
        .services
        .purge_unavailable_tracks(actor, library.id, only_doomed())
        .await
        .unwrap();
    std::fs::write(root.join("Second/2.flac"), &bytes).unwrap();
    scan_once(&state, owner, library.clone()).await;
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM track WHERE purged_at IS NOT NULL"
        )
        .await,
        0,
        "a track found again is not purged any more"
    );
    assert_eq!(state.services.finish_purges(i64::MAX).await.unwrap(), 0);
    assert_eq!(track_id(&state, "Second/2.flac").await, doomed);
    assert_eq!(
        journalled(&journal_of(&state, listener).await),
        listener_before,
        "nothing reached a journal while the purge could be taken back"
    );

    // Purged for good: the window passes.
    std::fs::remove_file(root.join("Second/2.flac")).unwrap();
    scan_once(&state, owner, library.clone()).await;
    state
        .services
        .purge_unavailable_tracks(actor, library.id, only_doomed())
        .await
        .unwrap();
    assert_eq!(state.services.finish_purges(now_ms()).await.unwrap(), 1);
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE is_available = 0").await,
        0
    );

    let listener_journal = journal_of(&state, listener).await;
    let told = &listener_journal[listener_before..];
    assert!(
        told.iter()
            .any(|(kind, id, action, payload)| kind == "favorite"
                && *id == doomed
                && action == "delete"
                && payload["entity_type"] == "track"),
        "the listener's favourite of the track"
    );
    assert!(
        told.iter()
            .any(|(kind, id, action, payload)| kind == "favorite"
                && *id == doomed_album
                && action == "delete"
                && payload["entity_type"] == "album"),
        "the listener's favourite of the album left empty"
    );
    let playlist_entry = told
        .iter()
        .find(|(kind, id, ..)| kind == "playlist" && *id == playlist.id.to_string())
        .expect("the listener's playlist that lost an entry");
    assert_eq!(playlist_entry.2, "upsert");
    assert_eq!(playlist_entry.3["track_ids"], serde_json::json!([kept]));
    assert_eq!(playlist_entry.3["name"], "Mixed");
    let owner_told = journal_of(&state, owner).await;
    assert!(
        owner_told[owner_before..]
            .iter()
            .any(|(kind, id, action, _)| kind == "rating" && *id == doomed && action == "delete"),
        "the owner's rating"
    );

    // A deletion tells them the same way.
    let before_deletion = journal_of(&state, listener).await.len();
    state
        .services
        .delete_library(actor, library.id, "undo")
        .await
        .unwrap();
    let after = journal_of(&state, listener).await;
    let emptied = after[before_deletion..]
        .iter()
        .find(|(kind, ..)| kind == "playlist")
        .expect("the playlist the deletion emptied");
    assert_eq!(emptied.3["track_ids"], serde_json::json!([]));
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(state.db.pool())
        .await
        .unwrap();
    assert!(violations.is_empty());
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

async fn path_ids(state: &waveflow_server::AppState) -> Vec<(String, String, i64)> {
    sqlx::query_as("SELECT relative_path, id, is_available FROM track ORDER BY relative_path")
        .fetch_all(state.db.pool())
        .await
        .unwrap()
}

/// RFC-021, what decision 6 left open: a scan the watcher limits to the
/// folders that changed keeps RFC-018's guarantees. Only what it walked may be
/// marked missing, relocation claims no track outside it — a file moved in
/// from a folder it did not walk is a new track, and the old one is left as
/// it was for the next full scan — and a move inside it keeps its identity.
#[tokio::test]
async fn a_scoped_scan_touches_nothing_it_did_not_walk() {
    let (_temp, state, owner, library, root) = incremental_library("scoped", |music| {
        for (folder, length) in [("A", 1_000), ("B", 1_100), ("C", 1_200)] {
            std::fs::create_dir_all(music.join(folder)).unwrap();
            write_test_wav_of_len(&music.join(format!("{folder}/{folder}.wav")), length);
        }
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    let before = path_ids(&state).await;
    let id_of = |path: &str| {
        before
            .iter()
            .find(|(stored, ..)| stored == path)
            .unwrap()
            .1
            .clone()
    };
    let scoped = |folders: &[&str]| {
        let state = state.clone();
        let library = library.clone();
        let folders: Vec<String> = folders.iter().map(|f| (*f).to_owned()).collect();
        async move {
            let scan = state
                .scanner
                .trigger_scoped(library, "watch", &folders)
                .await
                .unwrap();
            settled(&state, owner, scan).await
        }
    };

    // B's file goes, and C's moves into A; only A is walked.
    std::fs::remove_file(root.join("B/B.wav")).unwrap();
    std::fs::rename(root.join("C/C.wav"), root.join("A/From C.wav")).unwrap();
    let record = scoped(&["A"]).await;
    assert_eq!(record.status, "completed", "{:?}", record.message);
    assert_eq!(
        (record.added, record.moved, record.unavailable),
        (1, 0, 0),
        "{record:?}"
    );
    let after = path_ids(&state).await;
    let row = |path: &str| after.iter().find(|(stored, ..)| stored == path).cloned();
    assert_eq!(
        row("B/B.wav").unwrap().2,
        1,
        "B was not walked: not missing"
    );
    assert_eq!(
        row("C/C.wav").unwrap(),
        ("C/C.wav".to_owned(), id_of("C/C.wav"), 1),
        "C's track was not looked for, so nothing may claim it"
    );
    assert_ne!(row("A/From C.wav").unwrap().1, id_of("C/C.wav"));

    // Walking B marks what B lost, and nothing else.
    let record = scoped(&["B"]).await;
    assert_eq!((record.added, record.unavailable), (0, 1), "{record:?}");
    let after = path_ids(&state).await;
    let row = |path: &str| after.iter().find(|(stored, ..)| stored == path).cloned();
    assert_eq!(row("B/B.wav").unwrap().2, 0);
    assert_eq!(row("C/C.wav").unwrap().2, 1);

    // A move inside the walked folders keeps the track it moves.
    std::fs::create_dir_all(root.join("A/Deeper")).unwrap();
    std::fs::rename(root.join("A/A.wav"), root.join("A/Deeper/A.wav")).unwrap();
    let record = scoped(&["A"]).await;
    assert_eq!((record.moved, record.unavailable), (1, 0), "{record:?}");
    let after = path_ids(&state).await;
    assert_eq!(
        after
            .iter()
            .find(|(stored, ..)| stored == "A/Deeper/A.wav")
            .unwrap()
            .1,
        id_of("A/A.wav")
    );

    // A folder excluded on the way down counts as not discovered, as the
    // full walk would have it.
    std::fs::write(root.join("A/.waveflowignore"), "/Deeper/\n").unwrap();
    let record = scoped(&["A/Deeper"]).await;
    assert_eq!((record.unavailable, record.excluded), (1, 1), "{record:?}");

    // A scope it cannot trust is the whole library.
    let record = scoped(&["../elsewhere"]).await;
    assert_eq!(record.status, "completed");
    let scope: Option<String> = sqlx::query_scalar("SELECT scope_json FROM scan_job WHERE id = ?")
        .bind(record.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(scope, None);
    assert_eq!(
        path_ids(&state)
            .await
            .iter()
            .find(|(stored, ..)| stored == "C/C.wav")
            .unwrap()
            .2,
        0,
        "the full walk finds what the scoped ones were not allowed to"
    );
}

/// RFC-021, the per-library switch decision 6 left open: a library whose
/// watch is off is not watched, and a watched change in a library whose walk
/// reads no link is scanned in the folder it happened in.
#[tokio::test]
async fn a_library_is_watched_only_when_it_asks_and_in_the_folder_that_changed() {
    use waveflow_server::services::{LibraryActor, LibraryUpdate};

    let (_temp, state, owner, library, root) = incremental_library("watch-switch", |music| {
        std::fs::create_dir_all(music.join("Sub")).unwrap();
        write_test_wav(&music.join("Sub/First.wav"));
        std::fs::create_dir_all(music.join("Other")).unwrap();
        write_test_wav_of_len(&music.join("Other/Kept.wav"), 777);
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    let set_watch = |watch: bool| {
        let state = state.clone();
        async move {
            state
                .services
                .update_library(
                    LibraryActor::operator(owner),
                    library.id,
                    LibraryUpdate {
                        watch: Some(watch),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
        }
    };
    assert!(
        set_watch(false).await.scan_id.is_none(),
        "a switch queues no scan"
    );
    assert!(state.db.watched_libraries().await.unwrap().is_empty());
    let watcher = waveflow_server::watcher::spawn(
        state.scanner.clone(),
        state.db.clone(),
        std::time::Duration::from_millis(200),
    );
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    write_test_wav_of_len(&root.join("Sub/Unwatched.wav"), 850);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let watched_scans = || async {
        sqlx::query_as::<_, (Option<String>, String)>(
            "SELECT scope_json, status FROM scan_job WHERE trigger = 'watch'",
        )
        .fetch_all(state.db.pool())
        .await
        .unwrap()
    };
    assert!(
        watched_scans().await.is_empty(),
        "a library whose watch is off was scanned by the watcher"
    );

    set_watch(true).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    write_test_wav_of_len(&root.join("Sub/Second.wav"), 900);
    let mut completed = None;
    for _ in 0..300 {
        let scans = watched_scans().await;
        if let Some(done) = scans.iter().find(|(_, status)| status == "completed") {
            completed = Some(done.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    watcher.abort();
    let (scope, _) = completed.expect("no scan followed the change");
    assert_eq!(scope.as_deref(), Some(r#"["Sub"]"#));
    assert_eq!(
        available_paths(&state).await,
        [
            "Other/Kept.wav",
            "Sub/First.wav",
            "Sub/Second.wav",
            "Sub/Unwatched.wav"
        ]
    );
}

/// A scoped scan is no answer to a pending full-scan request: it walks the
/// whole library, reads every file, and only then is the request spent
/// (review of #335).
#[tokio::test]
async fn a_pending_full_scan_is_not_spent_by_a_scoped_one() {
    let (_temp, state, owner, library, root) = incremental_library("scoped-full", |music| {
        for (folder, length) in [("A", 1_000), ("B", 1_100)] {
            std::fs::create_dir_all(music.join(folder)).unwrap();
            write_test_wav_of_len(&music.join(format!("{folder}/{folder}.wav")), length);
        }
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    std::fs::remove_file(root.join("B/B.wav")).unwrap();
    state.db.request_full_scan(library.id).await.unwrap();
    let scan = state
        .scanner
        .trigger_scoped(library.clone(), "watch", &["A".to_owned()])
        .await
        .unwrap();
    let record = settled(&state, owner, scan).await;
    assert_eq!(record.status, "completed", "{:?}", record.message);
    assert_eq!(
        (record.unavailable, record.skipped),
        (1, 0),
        "the whole library walked, every file read"
    );
    assert!(
        !state.db.full_scan_requested(library.id).await.unwrap(),
        "the request was answered by a full walk"
    );
}

/// A purge past its grace window is due for deletion: neither listed as
/// restorable nor restored, even before the finishing pass has run (review of
/// #335).
#[tokio::test]
async fn a_purge_past_its_window_is_neither_listed_nor_restored() {
    use waveflow_server::services::{LibraryActor, PurgeRequest};

    let (_temp, state, owner, library, root) = library_with_albums("past-window").await;
    let actor = LibraryActor::operator(owner);
    std::fs::remove_file(root.join("Second/2.flac")).unwrap();
    scan_once(&state, owner, library.clone()).await;
    state
        .services
        .purge_unavailable_tracks(
            actor,
            library.id,
            PurgeRequest {
                track_ids: None,
                all: true,
            },
        )
        .await
        .unwrap();
    // Purged eight days ago, against a window of a week.
    sqlx::query("UPDATE track SET purged_at = ? WHERE purged_at IS NOT NULL")
        .bind(now_ms() - 8 * 24 * 60 * 60 * 1000)
        .execute(state.db.pool())
        .await
        .unwrap();
    let listed = state
        .services
        .purged_tracks(actor, library.id, Default::default())
        .await
        .unwrap();
    assert_eq!((listed.total, listed.tracks.len()), (0, 0));
    let restored = state
        .services
        .restore_purged_tracks(
            actor,
            library.id,
            PurgeRequest {
                track_ids: None,
                all: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        restored.restored, 0,
        "a purge past its window is due, not undone"
    );
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM track WHERE purged_at IS NOT NULL"
        )
        .await,
        1
    );
}

/// With the grace window set to zero, the purges a former window left behind
/// are deleted by the pass, which therefore runs whatever the settings
/// (review of #335).
#[tokio::test]
async fn a_zero_window_deletes_the_purges_a_former_one_left() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.purge_grace = std::time::Duration::ZERO;
    let state = waveflow_server::initialize(&config).await.unwrap();
    let owner = insert_account(&state, "zero-window", "admin").await;
    let music = temp.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Gone.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let id = state
        .db
        .create_library(owner, "zero", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let library = LibraryRecord {
        id,
        name: "zero".into(),
        root_path: root.clone(),
    };
    scan_once(&state, owner, library.clone()).await;
    std::fs::remove_file(root.join("Gone.wav")).unwrap();
    scan_once(&state, owner, library).await;
    // Purged under a former window, an hour ago.
    sqlx::query("UPDATE track SET purged_at = ? WHERE is_available = 0")
        .bind(now_ms() - 60 * 60 * 1000)
        .execute(state.db.pool())
        .await
        .unwrap();
    state.services.spawn_unavailable_purge();
    for _ in 0..200 {
        if count(&state, "SELECT COUNT(*) FROM track").await == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the purge a former window left was never deleted");
}
