use super::*;
use crate::library::scanner::discovery::{discover_audio, Reach};

fn ffmpeg(path: &Path, args: &[&str]) {
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

fn tone(frequency: usize) -> String {
    format!("sine=frequency={frequency}:duration=0.2")
}

/// A moving picture with a soundtrack, in whatever container the name says.
fn video(path: &Path, frequency: usize) {
    let tone = tone(frequency);
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
            &tone,
            "-shortest",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
        ],
    );
}

/// AAC alone.
fn audio_mp4(path: &Path, frequency: usize) {
    let tone = tone(frequency);
    ffmpeg(path, &["-f", "lavfi", "-i", &tone, "-c:a", "aac"]);
}

/// Audio with a chapter's still picture: a `vide` track whose sample entry is
/// `png `, which is no clip.
fn still_mp4(path: &Path, frequency: usize) {
    let tone = tone(frequency);
    ffmpeg(
        path,
        &[
            "-f",
            "lavfi",
            "-i",
            &tone,
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=16x16:d=0.2",
            "-map",
            "0",
            "-map",
            "1",
            "-frames:v",
            "1",
            "-c:a",
            "aac",
            "-c:v",
            "png",
            "-f",
            "mov",
        ],
    );
}

fn flac(path: &Path, frequency: usize) {
    let tone = tone(frequency);
    ffmpeg(path, &["-f", "lavfi", "-i", &tone, "-c:a", "flac"]);
}

/// The EBML magic is all either side reads of a WebM or a Matroska file.
fn ebml(path: &Path) {
    std::fs::write(path, [0x1a, 0x45, 0xdf, 0xa3, 0, 0, 0, 0]).unwrap();
}

/// A tree with every case decision 2 names, and what the rule answers for
/// each of its tracks.
fn fixture_tree(root: &Path) -> Vec<(&'static str, Option<(&'static str, &'static str)>)> {
    for folder in ["A/Clips", "B/clips"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    // Beside the track.
    flac(&root.join("A/Song.flac"), 300);
    video(&root.join("A/Song.mp4"), 305);
    // In its `Clips` folder.
    flac(&root.join("A/Two.flac"), 310);
    video(&root.join("A/Clips/Two.mp4"), 315);
    // A video with no song: its own clip.
    video(&root.join("A/Lone.mp4"), 320);
    // An audio `.mp4` is a track, and no clip.
    flac(&root.join("A/Audio.flac"), 330);
    audio_mp4(&root.join("A/Audio.mp4"), 335);
    // A chapter still is no clip.
    flac(&root.join("A/Still.flac"), 340);
    still_mp4(&root.join("A/Still.mp4"), 345);
    // The extension order: `.mov` before `.mkv`.
    flac(&root.join("A/Order.flac"), 350);
    ebml(&root.join("A/Order.mkv"));
    video(&root.join("A/Order.mov"), 355);
    // The first candidate fails its content check, the next one is taken.
    flac(&root.join("A/Fallback.flac"), 360);
    audio_mp4(&root.join("A/Fallback.mp4"), 365);
    ebml(&root.join("A/Fallback.webm"));
    // Case folded, on the folder and on the name.
    flac(&root.join("B/CASE.flac"), 370);
    ebml(&root.join("B/clips/case.WEBM"));
    // A track inside a `Clips` folder is offered none of its neighbours.
    flac(&root.join("A/Clips/Inner.flac"), 380);
    ebml(&root.join("A/Clips/Inner.webm"));
    // A video track beside a video of its name.
    video(&root.join("A/Pair.mp4"), 390);
    ebml(&root.join("A/Pair.webm"));
    vec![
        ("A/Audio.flac", None),
        ("A/Audio.mp4", None),
        ("A/Clips/Inner.flac", None),
        ("A/Fallback.flac", Some(("A/Fallback.webm", "sibling"))),
        ("A/Fallback.mp4", Some(("A/Fallback.webm", "sibling"))),
        ("A/Lone.mp4", Some(("A/Lone.mp4", "itself"))),
        ("A/Order.flac", Some(("A/Order.mov", "sibling"))),
        ("A/Pair.mp4", Some(("A/Pair.webm", "sibling"))),
        ("A/Song.flac", Some(("A/Song.mp4", "sibling"))),
        ("A/Still.flac", None),
        ("A/Still.mp4", None),
        ("A/Two.flac", Some(("A/Clips/Two.mp4", "clips_dir"))),
        ("B/CASE.flac", Some(("B/clips/case.WEBM", "clips_dir"))),
    ]
}

/// RFC-029 decision 2: the server's matcher answers what the core's
/// `find_clip` answers, for every track of a tree that holds each case the
/// rule distinguishes. A core release that changes the rule fails here
/// before it changes a catalogue.
#[test]
fn the_matcher_answers_what_find_clip_answers() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let expected = fixture_tree(&root);
    let walk = discover_audio(
        &Reach {
            root: &root,
            follow_links: false,
            link_roots: &[],
            libraries: &[],
        },
        None,
    )
    .unwrap();
    let videos = Videos::new(&walk.videos);
    let mut answers = Vec::new();
    for (path, name) in &walk.paths {
        let ours = videos
            .candidates(name, path)
            .into_iter()
            .find_map(|candidate| confirm(candidate, None, true))
            .map(|link| (link.relative_path, link.origin.to_owned()));
        let core = waveflow_core::scanner::clips::find_clip(path).map(|(clip, origin)| {
            let relative = clip
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (
                relative,
                serde_json::to_value(origin)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        });
        assert_eq!(ours, core, "{name}");
        answers.push((name.clone(), ours));
    }
    let expected: Vec<(String, Option<(String, String)>)> = expected
        .into_iter()
        .map(|(track, clip)| {
            (
                track.to_owned(),
                clip.map(|(path, origin)| (path.to_owned(), origin.to_owned())),
            )
        })
        .collect();
    assert_eq!(answers, expected, "the tree holds what it says it holds");
}

fn link(relative_path: &str) -> TrackClip {
    TrackClip {
        relative_path: relative_path.to_owned(),
        format: "mp4".to_owned(),
        origin: "clips_dir",
        byte_size: 1,
        modified_at: 1,
    }
}

/// A track `A/Song.flac` linked to `A/Clips/Song.mp4`, whose clip this walk
/// did not see.
fn unseen_clip(walked: WalkedClips) -> Vec<(Uuid, Option<TrackClip>)> {
    let track = Uuid::new_v4();
    let facts = HashMap::from([(
        "A/Song.flac".to_owned(),
        TrackClipFacts {
            track_id: track,
            clip: Some(link("A/Clips/Song.mp4")),
        },
    )]);
    changes(&walked, &facts)
}

fn walked() -> WalkedClips {
    WalkedClips {
        tracks: vec![(
            PathBuf::from("/nowhere/A/Song.flac"),
            "A/Song.flac".to_owned(),
        )],
        videos: Vec::new(),
        unreadable: Vec::new(),
        incomplete: false,
        scope: None,
        full: false,
    }
}

/// A link goes only where the walk could see its clip go: a clip under a
/// folder the walk could not read, or a walk that could not say what it
/// missed, keeps it.
#[test]
fn a_link_goes_only_when_the_walk_saw_its_clip_go() {
    let gone = unseen_clip(walked());
    assert_eq!(gone.len(), 1);
    assert_eq!(
        gone[0].1, None,
        "the clip's folder was walked, and it was not there"
    );

    let unreadable = unseen_clip(WalkedClips {
        unreadable: vec!["A/Clips".to_owned()],
        ..walked()
    });
    assert!(unreadable.is_empty(), "the Clips folder could not be read");

    let incomplete = unseen_clip(WalkedClips {
        incomplete: true,
        ..walked()
    });
    assert!(
        incomplete.is_empty(),
        "the walk could not say what it missed"
    );
}

/// A clip the pass cannot read keeps its recorded link: the failed read says
/// nothing about the file. A file read whole that holds no video loses it, and
/// a file too short for its header is such a file, not a failed read.
#[test]
fn a_clip_that_cannot_be_read_keeps_its_link() {
    let temp = tempfile::tempdir().unwrap();
    let candidate = |path, name| Candidate {
        path,
        name,
        origin: "sibling",
    };
    let recorded = TrackClip {
        origin: "sibling",
        ..link("A/Song.webm")
    };

    let missing = temp.path().join("Song.webm");
    let cannot_read = candidate(&missing, "A/Song.webm");
    assert_eq!(
        confirm(cannot_read, Some(&recorded), true),
        Some(recorded.clone())
    );
    assert_eq!(
        confirm(cannot_read, None, true),
        None,
        "nothing recorded to keep"
    );
    let other = link("A/Clips/Song.mp4");
    assert_eq!(
        confirm(cannot_read, Some(&other), true),
        None,
        "another clip's link"
    );

    let short = temp.path().join("Short.webm");
    std::fs::write(&short, [0x1a, 0x45]).unwrap();
    let truncated = candidate(&short, "A/Song.webm");
    assert_eq!(confirm(truncated, Some(&recorded), true), None);
}
