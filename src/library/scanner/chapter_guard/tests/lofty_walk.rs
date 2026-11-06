//! Where lofty reads a tag in a WAV or an AIFF, held to lofty itself: the
//! bodies of the chunks it skips, which the walk therefore leaves unread, and
//! the one place it loses its own: a `LIST INFO` it stops reading partway.

use super::large_files::{aiff_of, chunk, title_lofty_reads, wav_format, wav_of};
use super::*;

/// Why the walk reads no chunk body but a tag's and a `LIST`'s. lofty skips
/// what it leaves of every chunk with one seek to its end (`Chunks::skip`), so
/// a tag chunk standing inside the body of another chunk is never reached as a
/// chunk. This holds lofty to that for every chunk it reads or knows of, in a
/// WAV and in an AIFF, with the tag right at the start of the body and behind
/// a few bytes: in a `LIST INFO` it is read as one of the list's items, not as
/// a tag. The control reads the same tag chunk where the walk meets it.
#[test]
fn lofty_reads_no_tag_inside_the_body_of_another_chunk() {
    use lofty::file::FileType;
    let honest = tag(&title(4, "found"));
    let mut found = Vec::new();
    for big_endian in [false, true] {
        let (kind, tag_name, names): (_, &[u8; 4], &[&[u8; 4]]) = if big_endian {
            (
                FileType::Aiff,
                b"ID3 ",
                &[
                    b"COMM", b"SSND", b"NAME", b"AUTH", b"(c) ", b"ANNO", b"COMT", b"MARK",
                    b"JUNK", b"LIST",
                ],
            )
        } else {
            (
                FileType::Wav,
                b"id3 ",
                &[
                    b"fmt ", b"fact", b"data", b"JUNK", b"iXML", b"bext", b"LIST",
                ],
            )
        };
        let of = |chunks: &[u8]| {
            if big_endian {
                aiff_of(chunks)
            } else {
                wav_of(chunks)
            }
        };
        let hidden = chunk(tag_name, &honest, big_endian);
        assert_eq!(
            title_lofty_reads(of(&hidden), kind).as_deref(),
            Some("found"),
            "the control, {kind:?}"
        );
        for name in names {
            for ahead in [&b""[..], b"INFO", b"adtl", &[0, 2, 0, 0, 0, 0]] {
                let file = of(&chunk(name, &[ahead, &hidden].concat(), big_endian));
                if let Some(title) = title_lofty_reads(file, kind) {
                    found.push(format!("{kind:?}: {name:?} behind {ahead:?} gave {title}"));
                }
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

/// How far into the audio the tag of [`wav_resuming_at`] stands: past every
/// read of the head, so only a chunk walk reaches it.
const DEEP: usize = 200_000;

/// A WAV of a `LIST` whose body is `info`, padded when odd, then the audio,
/// with a `JUNK` header written `resume` bytes into the `LIST` body — or past
/// it — declaring a size that lands on a tag chunk holding `payload` [`DEEP`]
/// into the audio. Reached only by a walk that goes on from `resume`. A small
/// `data` chunk follows the tag, so lofty, which never meets the real one on
/// that path, still finds the audio properties it reads by default: without
/// one it fails the file after the walk, having parsed the tag all the same.
fn wav_resuming_at(info: &[u8], resume: usize, payload: &[u8]) -> Vec<u8> {
    let mut chunks = chunk(b"LIST", info, false);
    if info.len() % 2 == 1 {
        chunks.push(0);
    }
    chunks.extend_from_slice(&chunk(b"data", &vec![0x01; DEEP + 4096], false));
    let mut file = wav_of(&chunks);
    let body = 12 + wav_format().len() + 8;
    let audio = body + info.len() + info.len() % 2 + 8;
    let target = audio + DEEP;
    let island = [
        chunk(b"id3 ", payload, false),
        chunk(b"data", &[0; 16], false),
    ]
    .concat();
    file[target..target + island.len()].copy_from_slice(&island);
    let at = body + resume;
    let jump = (target - at - 8) as u32;
    file[at..at + 8].copy_from_slice(&[b"JUNK".as_slice(), &jump.to_le_bytes()].concat());
    file
}

/// The `LIST INFO` bodies that make lofty lose its place, and where it goes
/// on from, counted from the start of the `LIST` body. lofty walks the items of
/// a `LIST INFO` as chunks of their own, and when it stops partway, it goes on
/// with the file's chunks from where it stopped rather than from the end of
/// the `LIST`: behind a name that is no FourCC, behind the header of an item
/// longer than what is left, or behind an item whose length is all that is
/// left counting its header, which runs eight bytes past the end of the list.
/// An odd `LIST` then has its padding looked for there.
fn lofty_losing_its_place() -> Vec<(&'static str, Vec<u8>, usize)> {
    let head = [b"INFO".as_slice(), &chunk(b"INAM", b"Title\0", false)].concat();
    let mut cases = Vec::new();
    for bad in [b"\x01xyz", b"a b ", b"ab\x7fc"] {
        let info = [head.as_slice(), bad, &[0; 8]].concat();
        cases.push(("a name that is no FourCC", info, 22));
    }
    let info = [head.as_slice(), b"IART", &u32::MAX.to_le_bytes(), &[0; 8]].concat();
    cases.push(("an item longer than the list", info, 26));
    let info = [head.as_slice(), b"ICMT", &14_u32.to_le_bytes(), b"abcdef"].concat();
    cases.push(("an item running past the list", info, 40));
    // An item of odd length: behind its zero padding byte, lofty steps over
    // it; with none, it reads the next item from where the padding would be.
    let odd = [b"INFO".as_slice(), &chunk(b"INAM", b"Title", false)].concat();
    let info = [odd.as_slice(), &[0], b"\x01xyz", &[0; 8]].concat();
    cases.push(("an odd item behind a zero", info, 22));
    let item = chunk(b"IART", b"ab", false);
    let info = [odd.as_slice(), &item, b"\x01xyz", &[0; 8]].concat();
    cases.push(("an odd item with no padding", info, 31));
    let info = [head.as_slice(), b"\x01xyz", &[0], &[0; 8]].concat();
    cases.push(("an odd list behind a zero", info, 23));
    let info = [head.as_slice(), b"\x01xyz", &[0; 8], b"x"].concat();
    cases.push(("an odd list behind a letter", info, 22));
    cases
}

/// lofty reads a tag standing where a `LIST INFO` it lost its place in sends
/// it, and the guard must refuse the hostile one there. Every case holds lofty
/// to reading the honest tag, then the guard to refusing the hostile twin.
#[test]
fn a_tag_where_lofty_resumes_from_a_list_info_is_reached() {
    use lofty::file::FileType;
    let honest = tag(&title(4, "found"));
    let hostile = tag(&nested(4, &[b"CHAP"], 64));
    let mut failures = Vec::new();
    for (case, info, resume) in lofty_losing_its_place() {
        let read = title_lofty_reads(wav_resuming_at(&info, resume, &honest), FileType::Wav);
        if read.as_deref() != Some("found") {
            failures.push(format!("lofty, {case}: {read:?}"));
        }
        let verdict = check_file_bytes(&wav_resuming_at(&info, resume, &hostile));
        if verdict != Err(Refusal::Nested) {
            failures.push(format!("the guard, {case}: {verdict:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The ID3v2 title the server's own call to lofty reads from a WAV of `bytes`:
/// [`lofty::read_from_path`] on a `.wav` file, with its default options, which
/// read the audio properties as well as the tags.
fn title_the_server_reads(bytes: &[u8]) -> Option<String> {
    use lofty::file::TaggedFileExt;
    use lofty::prelude::Accessor;
    let file = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile()
        .expect("temporary WAV");
    std::fs::write(file.path(), bytes).expect("write temporary WAV");
    let tagged = lofty::read_from_path(file.path()).ok()?;
    let tag = tagged.tag(lofty::tag::TagType::Id3v2)?;
    tag.title().map(|title| title.into_owned())
}

/// The same cases read the way the server reads them: lofty takes the same
/// path with the audio properties read too, since reading the `fmt ` and the
/// `data` whole or in part leaves `Chunks::skip` at the end of each, and the
/// `LIST INFO` is walked the same way. The guard reads no lofty options, so
/// its verdict on these files is the one the test above holds.
#[test]
fn a_tag_where_lofty_resumes_from_a_list_info_is_read_by_the_server() {
    let honest = tag(&title(4, "found"));
    let hostile = tag(&nested(4, &[b"CHAP"], 64));
    let mut failures = Vec::new();
    for (case, info, resume) in lofty_losing_its_place() {
        let read = title_the_server_reads(&wav_resuming_at(&info, resume, &honest));
        if read.as_deref() != Some("found") {
            failures.push(format!("lofty, {case}: {read:?}"));
        }
        let verdict = check_file_bytes(&wav_resuming_at(&info, resume, &hostile));
        if verdict != Err(Refusal::Nested) {
            failures.push(format!("the guard, {case}: {verdict:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A WAV whose `data` lofty meets first, then `lists` lists that each run an
/// item eight bytes past their end — lofty's count of what is left of the file
/// is then `8 × lists` bytes ahead of where it reads — and last a `LIST INFO`
/// declaring `over` bytes more than the file holds, whose items stop at a name
/// that is no FourCC, right before a tag chunk holding `payload`.
fn wav_of_lists_ahead_of_the_count(lists: usize, over: u32, payload: &[u8]) -> Vec<u8> {
    let mut chunks = chunk(b"data", &[0; 16], false);
    for _ in 0..lists {
        let info = [b"INFO".as_slice(), b"INAM", &8_u32.to_le_bytes()].concat();
        chunks.extend_from_slice(&chunk(b"LIST", &info, false));
        chunks.extend_from_slice(&[1; 8]);
    }
    let inside = [
        b"INFO".as_slice(),
        b"\x01xyz",
        &chunk(b"id3 ", payload, false),
        &[1; 4096],
    ]
    .concat();
    chunks.extend_from_slice(b"LIST");
    chunks.extend_from_slice(&(inside.len() as u32 + over).to_le_bytes());
    chunks.extend_from_slice(&inside);
    wav_of(&chunks)
}

/// lofty keeps its count of what is left of the file apart from where it reads.
/// An item running past the end of its `LIST INFO` moves the reader eight bytes
/// on, but `Chunks::unlock` puts the count back where the end of the list
/// leaves it, so each such list leaves the count eight bytes ahead, and they
/// add up. lofty then takes a last `LIST` longer than the file, as long as it
/// fits that count, walks its items and reads the tag they lead to. The guard
/// must walk the items of a `LIST` that does not fit the file.
///
/// The controls hold the test to that reason: with nothing over, the `LIST`
/// fits and is walked anyway; with no lists ahead, lofty reads no tag from the
/// `LIST` over; and a tag longer than the count left once it stops in the list
/// is one lofty does not read either. Four thousand lists keep the tag past
/// what the head scan reads.
#[test]
fn a_tag_in_a_list_longer_than_the_file_is_reached() {
    use lofty::file::FileType;
    let honest = tag(&title(4, "found"));
    let hostile = tag(&nested(4, &[b"CHAP"], 64));
    let padded = |tag: &[u8], len: usize| [tag, &vec![0; len]].concat();
    let mut failures = Vec::new();
    for (lists, over, filler, read) in [
        (4096, 0, 0, true),
        (4096, 1, 0, true),
        (4096, 2, 0, true),
        (4096, 2, 30_000, true),
        (0, 2, 0, false),
        (4096, 2, 40_000, false),
    ] {
        let case = format!("{lists} lists, {over} over, {filler} bytes of padding");
        let file = wav_of_lists_ahead_of_the_count(lists, over, &padded(&honest, filler));
        let wanted = read.then_some("found");
        let plain = title_lofty_reads(file.clone(), FileType::Wav);
        let server = title_the_server_reads(&file);
        if plain.as_deref() != wanted || server.as_deref() != wanted {
            failures.push(format!("lofty, {case}: {plain:?} and {server:?}"));
        }
        if read {
            let file = wav_of_lists_ahead_of_the_count(lists, over, &padded(&hostile, filler));
            let verdict = check_file_bytes(&file);
            if verdict != Err(Refusal::Nested) {
                failures.push(format!("the guard, {case}: {verdict:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
