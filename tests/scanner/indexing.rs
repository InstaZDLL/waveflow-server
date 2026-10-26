//! What a scan indexes: formats, sidecars, artwork, compilations, moves and its
//! progress events.

use super::*;

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
    let record = waveflow_server::library::scanner::ScanProgress {
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
