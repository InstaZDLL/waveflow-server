//! A music video beside its song is not a track (RFC-029 decision 4).
//!
//! `mp4` is an audio extension, so before this a video dropped beside its song
//! as `Song.mp4` was indexed as a second track. The rule is the core's
//! `drop_clips`, as the desktop's walk applies it; these tests hold what it
//! keeps as much as what it drops, since real audio must never be lost to it.

use super::*;

/// An `.mp4` with a moving picture and a soundtrack, as a music video is.
fn write_video_mp4(path: &std::path::Path, frequency: usize) {
    ffmpeg(
        path,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=64x64:d=0.2",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={frequency}:duration=0.2"),
            "-shortest",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
        ],
    );
}

/// An `.mp4` holding nothing but AAC, which is a track whatever its name.
fn write_audio_mp4(path: &std::path::Path, frequency: usize) {
    ffmpeg(
        path,
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={frequency}:duration=0.2"),
            "-c:a",
            "aac",
        ],
    );
}

fn ffmpeg(path: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(path)
        .output()
        .expect("FFmpeg is required for the music video fixtures");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn a_music_video_beside_its_song_is_not_a_track() {
    let (_temp, state, owner, library, _root) = incremental_library("videos", |music| {
        std::fs::create_dir_all(music.join("Album/Clips")).unwrap();
        // The clip beside its song, and the clip in a `Clips` folder beside it.
        write_tagged_flac(&music.join("Album/Song.flac"), "Song", "Album", 300);
        write_video_mp4(&music.join("Album/Song.mp4"), 310);
        write_tagged_flac(&music.join("Album/Other.flac"), "Other", "Album", 320);
        write_video_mp4(&music.join("Album/Clips/Other.mp4"), 330);
        // An audio `.mp4` beside a song of its name stays a track.
        write_tagged_flac(&music.join("Album/Audio.flac"), "Audio", "Album", 340);
        write_audio_mp4(&music.join("Album/Audio.mp4"), 350);
        // A video with no song of its name is a track: a library of videos.
        write_video_mp4(&music.join("Album/Lone.mp4"), 360);
        // A `.webm` is no audio file, so it cannot make its namesake a clip:
        // only the audio files of the walk take part in the rule.
        write_video_mp4(&music.join("Album/Pair.mp4"), 370);
        // Its content is never read: the EBML magic stands for a WebM.
        std::fs::write(
            music.join("Album/Pair.webm"),
            [0x1a, 0x45, 0xdf, 0xa3, 0, 0, 0, 0],
        )
        .unwrap();
    })
    .await;
    scan_once(&state, owner, library).await;

    assert_eq!(
        available_paths(&state).await,
        [
            "Album/Audio.flac",
            "Album/Audio.mp4",
            "Album/Lone.mp4",
            "Album/Other.flac",
            "Album/Pair.mp4",
            "Album/Song.flac",
        ]
    );
}

/// A server that indexed a clip as a track before this loses that track on the
/// next scan, as it would lose any file gone from the walk.
#[tokio::test]
async fn a_video_already_indexed_goes_once_its_song_is_beside_it() {
    let (_temp, state, owner, library, root) = incremental_library("indexed", |music| {
        std::fs::create_dir_all(music.join("Album")).unwrap();
        write_video_mp4(&music.join("Album/Song.mp4"), 400);
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    assert_eq!(available_paths(&state).await, ["Album/Song.mp4"]);

    write_tagged_flac(&root.join("Album/Song.flac"), "Song", "Album", 410);
    scan_once(&state, owner, library).await;
    assert_eq!(available_paths(&state).await, ["Album/Song.flac"]);
}

/// The watcher scopes a scan to the folder a change happened in. A clip dropped
/// into `Album/Clips/` is told from a track only by the song in `Album/`, so
/// that scan walks `Album/`.
#[tokio::test]
async fn a_scan_of_a_clips_folder_walks_its_parent() {
    let (_temp, state, owner, library, root) = incremental_library("scoped", |music| {
        std::fs::create_dir_all(music.join("Album/Clips")).unwrap();
        write_tagged_flac(&music.join("Album/Song.flac"), "Song", "Album", 500);
    })
    .await;
    scan_once(&state, owner, library.clone()).await;

    write_video_mp4(&root.join("Album/Clips/Song.mp4"), 510);
    let scan = state
        .scanner
        .trigger_scoped(library, "watch", &["Album/Clips".to_owned()])
        .await
        .unwrap();
    let record = settled(&state, owner, scan).await;
    assert_eq!(record.status, "completed", "{:?}", record.message);
    assert_eq!(available_paths(&state).await, ["Album/Song.flac"]);
    assert_eq!(
        links(&state).await,
        [link("Album/Song.flac", "Album/Clips/Song.mp4", "clips_dir")],
        "and links the clip to the song it found there"
    );
}

/// Every recorded link as `(track, clip, origin)`, in track order.
async fn links(state: &waveflow_server::AppState) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT t.relative_path, c.relative_path, c.origin \
         FROM track_clip c JOIN track t ON t.id = c.track_id \
         WHERE c.library_id = t.library_id ORDER BY t.relative_path",
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap()
}

fn link(track: &str, clip: &str, origin: &str) -> (String, String, String) {
    (track.to_owned(), clip.to_owned(), origin.to_owned())
}

/// RFC-029 decision 3: the scan records the clip of each track, by the three
/// origins of the desktop's rule.
#[tokio::test]
async fn a_scan_records_each_track_s_music_video() {
    let (_temp, state, owner, library, root) = incremental_library("linked", |music| {
        std::fs::create_dir_all(music.join("Album/Clips")).unwrap();
        write_tagged_flac(&music.join("Album/Song.flac"), "Song", "Album", 600);
        write_video_mp4(&music.join("Album/Song.mp4"), 605);
        write_tagged_flac(&music.join("Album/Two.flac"), "Two", "Album", 610);
        write_video_mp4(&music.join("Album/Clips/Two.mp4"), 615);
        write_video_mp4(&music.join("Album/Lone.mp4"), 620);
        write_tagged_flac(&music.join("Album/Audio.flac"), "Audio", "Album", 630);
        write_audio_mp4(&music.join("Album/Audio.mp4"), 635);
    })
    .await;
    scan_once(&state, owner, library).await;

    assert_eq!(
        links(&state).await,
        [
            link("Album/Lone.mp4", "Album/Lone.mp4", "itself"),
            link("Album/Song.flac", "Album/Song.mp4", "sibling"),
            link("Album/Two.flac", "Album/Clips/Two.mp4", "clips_dir"),
        ]
    );
    let (format, byte_size): (String, i64) = sqlx::query_as(
        "SELECT format, byte_size FROM track_clip WHERE relative_path = 'Album/Song.mp4'",
    )
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(format, "mp4");
    assert_eq!(
        byte_size,
        std::fs::metadata(root.join("Album/Song.mp4"))
            .unwrap()
            .len() as i64
    );
}

/// The link follows the disk from scan to scan: a clip added beside a track
/// the incremental scan skips is recorded, an excluded one is not, and one
/// deleted is forgotten.
#[tokio::test]
async fn a_link_follows_the_disk_from_scan_to_scan() {
    let (_temp, state, owner, library, root) = incremental_library("follows", |music| {
        std::fs::create_dir_all(music.join("Album/Clips")).unwrap();
        write_tagged_flac(&music.join("Album/Song.flac"), "Song", "Album", 700);
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    assert!(links(&state).await.is_empty());

    let clip = root.join("Album/Clips/Song.mp4");
    write_video_mp4(&clip, 705);
    scan_once(&state, owner, library.clone()).await;
    let linked = [link("Album/Song.flac", "Album/Clips/Song.mp4", "clips_dir")];
    assert_eq!(
        links(&state).await,
        linked,
        "added beside a track the scan skipped"
    );

    std::fs::write(root.join(".waveflowignore"), "Clips/\n").unwrap();
    scan_once(&state, owner, library.clone()).await;
    assert!(
        links(&state).await.is_empty(),
        "an excluded clip is not seen"
    );

    std::fs::remove_file(root.join(".waveflowignore")).unwrap();
    scan_once(&state, owner, library.clone()).await;
    assert_eq!(links(&state).await, linked);

    std::fs::remove_file(&clip).unwrap();
    scan_once(&state, owner, library).await;
    assert!(
        links(&state).await.is_empty(),
        "a deleted clip is forgotten"
    );
}
