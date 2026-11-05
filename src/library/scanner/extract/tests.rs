//! Unit tests of the tag readers and the lyrics sidecars.

use super::{extended_tags, extract_file, read_sidecar, MAX_LYRICS_BYTES};
use lofty::{
    prelude::ItemKey,
    tag::{Tag, TagType},
};

/// Smaller than the 2 MiB of a blocking task, so a file the guard lets through
/// to lofty or the `id3` crate would overflow it. What the guard refuses never
/// reaches a parser, so it needs no stack to speak of. Without the guard this
/// thread aborts the whole test process on the hostile fixtures below, which is
/// the regression these tests hold.
const SMALL_STACK: usize = 256 * 1024;

fn synchsafe_len(value: usize) -> [u8; 4] {
    [
        ((value >> 21) & 0x7f) as u8,
        ((value >> 14) & 0x7f) as u8,
        ((value >> 7) & 0x7f) as u8,
        (value & 0x7f) as u8,
    ]
}

/// A 2.4 tag of `depth` `CHAP` frames, each nested inside the last.
fn nested_tag(depth: usize) -> Vec<u8> {
    let prefix: &[u8] = b"c\0\0\0\0\0\0\0\x03\xe8\xff\xff\xff\xff\xff\xff\xff\xff";
    let mut inner = Vec::new();
    for _ in 0..depth {
        let mut frame = b"CHAP".to_vec();
        frame.extend_from_slice(&synchsafe_len(prefix.len() + inner.len()));
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(prefix);
        frame.extend_from_slice(&inner);
        inner = frame;
    }
    let mut tag = b"ID3\x04\0\0".to_vec();
    tag.extend_from_slice(&synchsafe_len(inner.len()));
    tag.extend_from_slice(&inner);
    tag
}

/// A DSF whose metadata offset points at `tag` sitting at the end.
///
/// The `fmt ` chunk carries valid values rather than zeros, so that with the
/// guard removed `parse_dsf` succeeds and the metadata reader really reaches the
/// `id3` crate — otherwise an invalid rate would be refused first and this
/// fixture would prove nothing about the chapter guard.
fn dsf_with(tag: &[u8]) -> Vec<u8> {
    let audio = vec![0_u8; 256];
    let offset = 28_u64 + 52 + (12 + audio.len() as u64);
    let mut out = b"DSD ".to_vec();
    out.extend_from_slice(&28_u64.to_le_bytes());
    out.extend_from_slice(&(offset + tag.len() as u64).to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&52_u64.to_le_bytes());
    out.extend_from_slice(&1_u32.to_le_bytes()); // format version
    out.extend_from_slice(&0_u32.to_le_bytes()); // raw DSD
    out.extend_from_slice(&2_u32.to_le_bytes()); // channel type: stereo
    out.extend_from_slice(&2_u32.to_le_bytes()); // channel count
    out.extend_from_slice(&2_822_400_u32.to_le_bytes()); // DSD64 sample rate
    out.extend_from_slice(&1_u32.to_le_bytes()); // bits per sample
    out.extend_from_slice(&1024_u64.to_le_bytes()); // samples per channel
    out.extend_from_slice(&4096_u32.to_le_bytes()); // block size per channel
    out.extend_from_slice(&0_u32.to_le_bytes()); // reserved
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(12 + audio.len() as u64).to_le_bytes());
    out.extend_from_slice(&audio);
    out.extend_from_slice(tag);
    out
}

fn on_small_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(work)
        .expect("spawn")
        .join()
        .expect("the guard refused the file without reaching a parser")
}

/// An MP3 whose ID3v2 tag nests chapter frames aborts lofty with a stack
/// overflow, which `extract_file` must refuse before reaching it. Red before the
/// guard: this very thread overflows and takes the test binary down with it.
#[test]
fn a_nested_chapter_mp3_is_refused_before_lofty_overflows() {
    let temp = tempfile::tempdir().unwrap();
    let mut mp3 = nested_tag(64);
    mp3.extend_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
    mp3.resize(mp3.len() + 1024, 0);
    let path = temp.path().join("hostile.mp3");
    std::fs::write(&path, &mp3).unwrap();
    let artwork = temp.path().join("artwork");
    std::fs::create_dir_all(&artwork).unwrap();
    let error = on_small_stack(move || extract_file(&path, &artwork).err());
    assert!(
        error.is_some_and(|message| message.contains("nests chapter frames")),
        "the guard, not some other error, must be what refuses the file"
    );
}

/// The same for the `id3` crate, reached through the DSD metadata path.
#[test]
fn a_nested_chapter_dsf_is_refused_before_id3_overflows() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hostile.dsf");
    std::fs::write(&path, dsf_with(&nested_tag(64))).unwrap();
    let artwork = temp.path().join("artwork");
    std::fs::create_dir_all(&artwork).unwrap();
    let error = on_small_stack(move || extract_file(&path, &artwork).err());
    assert!(
        error.is_some_and(|message| message.contains("nests chapter frames")),
        "the guard, not the DSD parser, must be what refuses the file"
    );
}

#[test]
fn extended_tags_survive_the_shapes_real_files_use() {
    // MP4 is the tag type whose lofty mapping covers every key this reads.
    // The others cover a subset each — ID3v2 carries the recording MBID in
    // a UFID frame rather than a text one, and Vorbis comments spell BPM
    // only as `Bpm` — which is what the fallbacks in `extended_tags` are
    // for. This test is about the parsing, so it uses the tag type that
    // exercises all of it at once.
    let mut tag = Tag::new(TagType::Mp4Ilst);
    // ReplayGain is written with its unit, and a leading + is legal.
    assert!(tag.insert_text(ItemKey::ReplayGainTrackGain, "-7.32 dB".into()));
    assert!(tag.insert_text(ItemKey::ReplayGainAlbumGain, "+1.50 dB".into()));
    assert!(tag.insert_text(ItemKey::ReplayGainTrackPeak, "0.988525".into()));
    // BPM is often fractional even in the integer key.
    assert!(tag.insert_text(ItemKey::IntegerBpm, "128.5".into()));
    assert!(tag.insert_text(ItemKey::MusicBrainzRecordingId, "  rec-1  ".into()));
    assert!(tag.insert_text(ItemKey::Comment, "   ".into()));
    assert!(tag.insert_text(ItemKey::Isrc, "FRZ039800212".into()));
    assert!(tag.insert_text(ItemKey::Mood, "Melancholic; Warm".into()));
    assert!(tag.insert_text(ItemKey::ParentalAdvisory, "2".into()));
    // The three sort keys, which the catalogue derives album and artist
    // sort names from. A joined credit carries a joined sort tag, and the
    // pairing happens where the names are split — what this pins is that
    // all three arrive at all, under the tag type that spells them.
    assert!(tag.insert_text(ItemKey::AlbumTitleSortOrder, "Night Sessions, The".into()));
    assert!(tag.insert_text(ItemKey::AlbumArtistSortOrder, "Nocturnes, The".into()));
    assert!(tag.insert_text(
        ItemKey::TrackArtistSortOrder,
        "Nocturnes, The; Reed, Ada".into()
    ));
    let extended = extended_tags(Some(&tag));
    assert_eq!(extended.replay_gain_track_gain, Some(-7.32));
    assert_eq!(extended.replay_gain_album_gain, Some(1.5));
    assert_eq!(extended.replay_gain_track_peak, Some(0.988_525));
    assert_eq!(extended.replay_gain_album_peak, None);
    assert_eq!(extended.bpm, Some(128));
    // Trimmed, and a tag holding only whitespace is no value at all
    // rather than an empty one the presence rule would then report as
    // meaningful.
    assert_eq!(extended.musicbrainz_recording_id.as_deref(), Some("rec-1"));
    assert_eq!(extended.comment, None);
    assert_eq!(extended.isrc.as_deref(), Some("FRZ039800212"));
    assert_eq!(extended.moods.as_deref(), Some("Melancholic; Warm"));
    // The per-format spelling is normalised to the two words the
    // specification defines; a client compares against nothing else.
    assert_eq!(extended.explicit_status.as_deref(), Some("clean"));
    assert_eq!(extended.sort_album.as_deref(), Some("Night Sessions, The"));
    assert_eq!(
        extended.sort_album_artist.as_deref(),
        Some("Nocturnes, The")
    );
    assert_eq!(
        extended.sort_artist.as_deref(),
        Some("Nocturnes, The; Reed, Ada")
    );

    // Unparseable measurements are dropped rather than stored: a NaN gain
    // would reach a player as a volume adjustment.
    let mut broken = Tag::new(TagType::Mp4Ilst);
    assert!(broken.insert_text(ItemKey::ReplayGainTrackGain, "loud".into()));
    assert!(broken.insert_text(ItemKey::ReplayGainAlbumPeak, "NaN".into()));
    assert!(broken.insert_text(ItemKey::IntegerBpm, "0".into()));
    let broken = extended_tags(Some(&broken));
    assert_eq!(broken.replay_gain_track_gain, None);
    assert_eq!(broken.replay_gain_album_peak, None);
    assert_eq!(broken.bpm, None);
    // A tag saying "no advisory" is not a claim that the work is clean.
    let mut neutral = Tag::new(TagType::Mp4Ilst);
    assert!(neutral.insert_text(ItemKey::ParentalAdvisory, "0".into()));
    assert_eq!(extended_tags(Some(&neutral)).explicit_status, None);

    // The DSD path has no tag reader and must produce the same shape.
    let absent = extended_tags(None);
    assert_eq!(absent.musicbrainz_recording_id, None);
    assert_eq!(absent.bpm, None);
}

#[test]
fn sidecar_reader_rejects_invalid_files_and_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let valid = temp.path().join("valid.lrc");
    std::fs::write(&valid, "[00:01]safe").unwrap();
    assert_eq!(read_sidecar(&valid).as_deref(), Some("[00:01]safe"));

    let oversized = temp.path().join("oversized.lrc");
    std::fs::write(&oversized, vec![b'x'; MAX_LYRICS_BYTES as usize + 1]).unwrap();
    assert!(read_sidecar(&oversized).is_none());

    assert!(read_sidecar(temp.path()).is_none());

    let link = temp.path().join("linked.lrc");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&valid, &link).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&valid, &link).is_ok();
    if linked {
        assert!(read_sidecar(&link).is_none());
    }
}

/// Lyrics saved by a tool on a Japanese Windows, in that system's code
/// page. A strict UTF-8 read dropped the file whole, so the track had no
/// lyrics at all.
#[test]
fn sidecar_lyrics_that_are_not_utf8_are_read_in_their_own_encoding() {
    let temp = tempfile::tempdir().unwrap();
    let text = "[00:01.00]今日はいい天気ですね、一緒に公園を散歩しましょう\n\
                [00:05.50]湖に光が映って、風が森を通り抜ける音がする";
    let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode(text);
    assert!(!unmappable, "Shift_JIS holds the sample");
    assert!(
        std::str::from_utf8(&bytes).is_err(),
        "the sample is not UTF-8 by accident"
    );
    let japanese = temp.path().join("japanese.lrc");
    std::fs::write(&japanese, &bytes).unwrap();
    assert_eq!(read_sidecar(&japanese).as_deref(), Some(text));

    // A byte-order mark names the encoding and is not part of the text.
    // Kept, it sat in front of the first timestamp.
    let marked = temp.path().join("marked.lrc");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"[00:01]marked");
    std::fs::write(&marked, bytes).unwrap();
    assert_eq!(read_sidecar(&marked).as_deref(), Some("[00:01]marked"));
}
