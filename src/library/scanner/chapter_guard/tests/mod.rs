use std::io::Cursor;

use super::*;

mod large_files;
mod lofty_walk;

fn synchsafe_bytes(value: usize) -> [u8; 4] {
    [
        ((value >> 21) & 0x7f) as u8,
        ((value >> 14) & 0x7f) as u8,
        ((value >> 7) & 0x7f) as u8,
        (value & 0x7f) as u8,
    ]
}

fn frame(major: u8, id: &[u8; 4], flags: u16, body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    if major == 4 {
        out.extend_from_slice(&synchsafe_bytes(body.len()));
    } else {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    }
    out.extend_from_slice(&flags.to_be_bytes());
    out.extend_from_slice(body);
    out
}

fn title(major: u8, text: &str) -> Vec<u8> {
    let mut body = vec![3_u8];
    body.extend_from_slice(text.as_bytes());
    frame(major, b"TIT2", 0, &body)
}

fn chapter_prefix(element: &str, start_ms: u32) -> Vec<u8> {
    let mut body = element.as_bytes().to_vec();
    body.push(0);
    body.extend_from_slice(&start_ms.to_be_bytes());
    body.extend_from_slice(&(start_ms + 999).to_be_bytes());
    body.extend_from_slice(&[0xff; 8]);
    body
}

fn toc_prefix() -> Vec<u8> {
    let mut body = b"toc\0".to_vec();
    body.extend_from_slice(&[0x03, 1]);
    body.extend_from_slice(b"c\0");
    body
}

/// `depth` chapter frames, each inside the last, cycling through `ids`.
fn nested(major: u8, ids: &[&[u8; 4]], depth: usize) -> Vec<u8> {
    let mut inner = title(major, "deep");
    for level in 0..depth {
        let id = ids[level % ids.len()];
        let mut body = if id == b"CHAP" {
            chapter_prefix("c", 0)
        } else {
            toc_prefix()
        };
        body.extend_from_slice(&inner);
        inner = frame(major, id, 0, &body);
    }
    inner
}

fn tag_with(major: u8, flags: u8, frames: &[u8]) -> Vec<u8> {
    let mut out = b"ID3".to_vec();
    out.extend_from_slice(&[major, 0, flags]);
    out.extend_from_slice(&synchsafe_bytes(frames.len()));
    out.extend_from_slice(frames);
    out
}

fn tag(frames: &[u8]) -> Vec<u8> {
    tag_with(4, 0, frames)
}

/// A real chapter layout: a table of contents naming two chapters, each with a
/// title, a link and a picture, plus a top-level title. Nothing nests.
fn honest_chapters() -> Vec<u8> {
    let picture = {
        let mut body = vec![3_u8];
        body.extend_from_slice(b"image/png\0");
        body.push(3);
        body.extend_from_slice(b"cover\0");
        body.extend_from_slice(&[0x89, b'P', b'N', b'G', 0, 1, 2, 3]);
        body
    };
    let link = {
        let mut body = b"https://example.invalid/chapter".to_vec();
        body.push(0);
        body
    };
    let chapter = |element: &str, start: u32| {
        let mut body = chapter_prefix(element, start);
        body.extend_from_slice(&title(4, "A chapter called CHAPTER something"));
        body.extend_from_slice(&frame(4, b"WXXX", 0, &link));
        body.extend_from_slice(&frame(4, b"APIC", 0, &picture));
        frame(4, b"CHAP", 0, &body)
    };
    let toc = {
        let mut body = b"toc\0".to_vec();
        body.extend_from_slice(&[0x03, 2]);
        body.extend_from_slice(b"chp1\0");
        body.extend_from_slice(b"chp2\0");
        body.extend_from_slice(&title(4, "Table of contents"));
        frame(4, b"CTOC", 0, &body)
    };
    let mut frames = title(4, "A whole audiobook");
    frames.extend_from_slice(&toc);
    frames.extend_from_slice(&chapter("chp1", 0));
    frames.extend_from_slice(&chapter("chp2", 60_000));
    frames
}

/// Enough MPEG-1 Layer III frames that lofty and the probe call it an MP3.
fn audio() -> Vec<u8> {
    let mut one = vec![0xff, 0xfb, 0x90, 0x00];
    one.resize(417, 0);
    one.repeat(8)
}

fn mp3(tag: &[u8]) -> Vec<u8> {
    [tag, audio().as_slice()].concat()
}

fn wav(chunk: &[u8]) -> Vec<u8> {
    let mut body = b"WAVEfmt ".to_vec();
    body.extend_from_slice(&16_u32.to_le_bytes());
    for field in [1_u16, 1] {
        body.extend_from_slice(&field.to_le_bytes());
    }
    body.extend_from_slice(&8000_u32.to_le_bytes());
    body.extend_from_slice(&16_000_u32.to_le_bytes());
    for field in [2_u16, 16] {
        body.extend_from_slice(&field.to_le_bytes());
    }
    body.extend_from_slice(b"data");
    body.extend_from_slice(&1600_u32.to_le_bytes());
    body.extend_from_slice(&[0; 1600]);
    body.extend_from_slice(b"id3 ");
    body.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
    body.extend_from_slice(chunk);
    if chunk.len() % 2 == 1 {
        body.push(0);
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

fn aiff(chunk: &[u8]) -> Vec<u8> {
    let mut body = b"AIFFCOMM".to_vec();
    body.extend_from_slice(&18_u32.to_be_bytes());
    body.extend_from_slice(&1_u16.to_be_bytes());
    body.extend_from_slice(&800_u32.to_be_bytes());
    body.extend_from_slice(&16_u16.to_be_bytes());
    body.extend_from_slice(&[0x40, 0x0b, 0xfa, 0, 0, 0, 0, 0, 0, 0]);
    body.extend_from_slice(b"SSND");
    body.extend_from_slice(&1608_u32.to_be_bytes());
    body.extend_from_slice(&[0; 1608]);
    body.extend_from_slice(b"ID3 ");
    body.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
    body.extend_from_slice(chunk);
    if chunk.len() % 2 == 1 {
        body.push(0);
    }
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// A DSF whose metadata offset points at `tag`, sitting at the end.
fn dsf(tag: &[u8]) -> Vec<u8> {
    let audio = vec![0_u8; 256];
    let offset = 28_u64 + 52 + (12 + audio.len() as u64);
    let mut out = b"DSD ".to_vec();
    out.extend_from_slice(&28_u64.to_le_bytes());
    out.extend_from_slice(&(offset + tag.len() as u64).to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&52_u64.to_le_bytes());
    out.resize(28 + 52, 0);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(12 + audio.len() as u64).to_le_bytes());
    out.extend_from_slice(&audio);
    out.extend_from_slice(tag);
    out
}

/// A DFF whose `ID3 ` chunk holds `tag`.
fn dff(tag: &[u8]) -> Vec<u8> {
    let chunk = |id: &[u8; 4], body: &[u8]| {
        let mut out = id.to_vec();
        out.extend_from_slice(&(body.len() as u64).to_be_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    };
    let dsd = chunk(b"DSD ", &[0_u8; 256]);
    let id3 = chunk(b"ID3 ", tag);
    let mut body = b"DSD ".to_vec();
    body.extend_from_slice(&dsd);
    body.extend_from_slice(&id3);
    let mut out = b"FRM8".to_vec();
    out.extend_from_slice(&(body.len() as u64).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// What lofty inflates with no compression library needed to write it: zlib's
/// stored blocks and a checksum. Three bytes to the block, so no four-byte name
/// is left readable in the bytes, as none would be in a real compressed frame.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut chunks = data.chunks(3).peekable();
    while let Some(chunk) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        out.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

#[test]
fn an_honest_chapter_layout_passes() {
    assert_eq!(check_tag(&tag(&honest_chapters())), Ok(()));
}

#[test]
fn a_file_with_no_chapters_passes() {
    let frames = [title(4, "No chapters here, just a title")].concat();
    assert_eq!(check_tag(&tag(&frames)), Ok(()));
    assert_eq!(check_file_bytes(&mp3(&tag(&frames))), Ok(()));
}

#[test]
fn deeply_nested_chap_is_refused() {
    assert_eq!(
        check_tag(&tag(&nested(4, &[b"CHAP"], 64))),
        Err(Refusal::Nested)
    );
}

#[test]
fn deeply_nested_ctoc_is_refused() {
    assert_eq!(
        check_tag(&tag(&nested(4, &[b"CTOC"], 64))),
        Err(Refusal::Nested)
    );
}

#[test]
fn mixed_chap_and_ctoc_nesting_is_refused() {
    let frames = nested(4, &[b"CTOC", b"CHAP"], 64);
    assert_eq!(check_tag(&tag(&frames)), Err(Refusal::Nested));
}

#[test]
fn nesting_in_a_v23_tag_is_refused() {
    assert_eq!(
        check_tag(&tag_with(3, 0, &nested(3, &[b"CHAP"], 64))),
        Err(Refusal::Nested)
    );
}

#[test]
fn an_unsynchronised_v23_tag_that_nests_is_refused() {
    let frames = nested(3, &[b"CHAP"], 64);
    let mut raw = b"ID3".to_vec();
    raw.extend_from_slice(&[3, 0, 0x80]);
    raw.extend_from_slice(&synchsafe_bytes(frames.len()));
    // Unsynchronise the body the way a writer would: a zero after every 0xff.
    let mut body = Vec::new();
    for byte in &frames {
        body.push(*byte);
        if *byte == 0xff {
            body.push(0);
        }
    }
    raw[6..10].copy_from_slice(&synchsafe_bytes(body.len()));
    raw.extend_from_slice(&body);
    assert_eq!(check_tag(&raw), Err(Refusal::Nested));
}

#[test]
fn a_compressed_chapter_is_refused_unread() {
    let inner = nested(4, &[b"CHAP"], 8);
    let compressed = zlib_stored(&inner);
    // The 2.4 compression flag, with the decompressed size prepended.
    let mut body = (inner.len() as u32).to_be_bytes().to_vec();
    body.extend_from_slice(&compressed);
    let frames = frame(4, b"CHAP", 0x0008, &body);
    assert_eq!(check_tag(&tag(&frames)), Err(Refusal::Nested));
}

#[test]
fn a_title_that_reads_chapter_stays_a_title() {
    // A free-text title whose bytes contain "CHAP" is not a frame.
    let frames = [
        title(4, "CHAPTER CHAPTER CHAPTER CHAPTER CHAPTER"),
        title(4, "CHAPTERS, a retrospective"),
    ]
    .concat();
    assert_eq!(check_tag(&tag(&frames)), Ok(()));
}

#[test]
fn nesting_behind_junk_in_an_mp3_is_refused() {
    let mut file = vec![0_u8; 200];
    file.extend_from_slice(&tag(&nested(4, &[b"CHAP"], 64)));
    file.extend_from_slice(&audio());
    assert_eq!(check_file_bytes(&file), Err(Refusal::Nested));
}

#[test]
fn nesting_in_a_wav_id3_chunk_is_refused() {
    assert_eq!(
        check_file_bytes(&wav(&tag(&nested(4, &[b"CHAP"], 64)))),
        Err(Refusal::Nested)
    );
}

#[test]
fn an_honest_wav_passes() {
    assert_eq!(check_file_bytes(&wav(&tag(&honest_chapters()))), Ok(()));
}

#[test]
fn nesting_in_an_aiff_id3_chunk_is_refused() {
    assert_eq!(
        check_file_bytes(&aiff(&tag(&nested(4, &[b"CHAP"], 64)))),
        Err(Refusal::Nested)
    );
}

#[test]
fn nesting_in_a_dsf_blob_is_refused() {
    assert_eq!(
        check_dsd_bytes(&dsf(&tag(&nested(4, &[b"CHAP"], 64)))),
        Err(Refusal::Nested)
    );
}

#[test]
fn an_honest_dsf_passes() {
    assert_eq!(check_dsd_bytes(&dsf(&tag(&honest_chapters()))), Ok(()));
    // A DSF with no metadata at all (offset zero) passes.
    assert_eq!(check_dsd_bytes(&dsf(&[])), Ok(()));
}

#[test]
fn nesting_in_a_dff_id3_chunk_is_refused() {
    assert_eq!(
        check_dsd_bytes(&dff(&tag(&nested(4, &[b"CHAP"], 64)))),
        Err(Refusal::Nested)
    );
}

#[test]
fn an_honest_dff_passes() {
    assert_eq!(check_dsd_bytes(&dff(&tag(&honest_chapters()))), Ok(()));
}

/// A DFF whose chunk walk runs past `MAX_CHUNKS` is refused, not passed: the DSD
/// reader would walk on and could reach a tag beyond where the guard gave up.
#[test]
fn a_dff_with_too_many_chunks_is_refused() {
    // `FRM8` <8-byte size> "DSD ", then many empty chunks: a 4-byte id, an
    // 8-byte zero length, no body. One more than the walk will follow.
    let count = MAX_CHUNKS + 1;
    let mut form = b"DSD ".to_vec();
    for _ in 0..count {
        form.extend_from_slice(b"junk");
        form.extend_from_slice(&0_u64.to_be_bytes());
    }
    let mut dff = b"FRM8".to_vec();
    dff.extend_from_slice(&(form.len() as u64).to_be_bytes());
    dff.extend_from_slice(&form);
    assert_eq!(check_dsd_bytes(&dff), Err(Refusal::OverBudget));
}

/// A table of contents naming four chapters, then the four flat `CHAP` frames,
/// with innocuous element ids and no chapter text in any body.
fn toc_and_four_flat_chapters() -> Vec<u8> {
    let mut toc = b"toc\0".to_vec();
    toc.extend_from_slice(&[0x03, 4]);
    for id in ["ch01", "ch02", "ch03", "ch04"] {
        toc.extend_from_slice(id.as_bytes());
        toc.push(0);
    }
    let mut frames = frame(4, b"CTOC", 0, &toc);
    for (n, id) in ["ch01", "ch02", "ch03", "ch04"].iter().enumerate() {
        let mut body = id.as_bytes().to_vec();
        body.push(0);
        body.extend_from_slice(&((n as u32) * 1000).to_be_bytes());
        body.extend_from_slice(&((n as u32) * 1000 + 999).to_be_bytes());
        body.extend_from_slice(&[0xff; 8]);
        frames.extend_from_slice(&frame(4, b"CHAP", 0, &body));
    }
    frames
}

/// A frame the walk stops trusting must not drag the honest chapter frames
/// behind it into the count: its range would otherwise reach the next frame's
/// id. `TIPL` is read only partway by lofty, so it ends the trust.
#[test]
fn a_tipl_before_flat_chapters_passes() {
    let mut body = vec![3_u8];
    body.extend_from_slice(b"producer\0Alice\0engineer\0Bob");
    let mut frames = frame(4, b"TIPL", 0, &body);
    frames.extend_from_slice(&toc_and_four_flat_chapters());
    assert_eq!(check_tag(&tag(&frames)), Ok(()));
}

/// The same when the trust ends because a frame carries non-zero flags.
#[test]
fn a_flagged_frame_before_flat_chapters_passes() {
    let mut body = vec![3_u8];
    body.extend_from_slice(b"a plain comment");
    let mut frames = frame(4, b"TXXX", 0x0040, &body);
    frames.extend_from_slice(&toc_and_four_flat_chapters());
    assert_eq!(check_tag(&tag(&frames)), Ok(()));
}

/// The items of a `LIST INFO` are walked as lofty walks them, one after the
/// other, so a list of more of them than the walk follows has not been
/// checked: refused for the budget, and the walk returns rather than running
/// away. Named `ID3 `, they are items to lofty, not tags.
#[test]
fn a_list_info_of_more_items_than_the_walk_follows_is_refused() {
    let mut list_body = b"INFO".to_vec();
    for _ in 0..(MAX_CHUNKS + 100) {
        list_body.extend_from_slice(b"ID3 \0\0\0\0");
    }
    let mut body = b"WAVE".to_vec();
    body.extend_from_slice(b"LIST");
    body.extend_from_slice(&(list_body.len() as u32).to_le_bytes());
    body.extend_from_slice(&list_body);
    let mut riff = b"RIFF".to_vec();
    riff.extend_from_slice(&(body.len() as u32).to_le_bytes());
    riff.extend_from_slice(&body);
    assert_eq!(check_file_bytes(&riff), Err(Refusal::OverBudget));
}

/// The read of a tag body stops at the cap, so a header declaring a huge size
/// cannot allocate past the caller's budget before it is refused.
#[test]
fn read_declared_caps_the_body_at_the_given_limit() {
    let body = vec![0_u8; 1024 * 1024];
    let mut raw = b"ID3\x04\0\0".to_vec();
    raw.extend_from_slice(&synchsafe_bytes(body.len()));
    raw.extend_from_slice(&body);
    let DeclaredTag::Tag(tag) = read_declared(&mut Cursor::new(raw), 0, 100) else {
        panic!("the header and the capped body read as a tag");
    };
    assert!(
        tag.len() <= TAG_HEADER_LEN + 100,
        "the body read is capped at the limit, not the declared size: {} bytes",
        tag.len()
    );
}

/// A chunk declaring far more than the file holds is where lofty stops, so the
/// walk stops there too: its body is not read at all, not even up to the
/// budget. Measured through [`CountingReader`], so a read that ran into the
/// declared size would be seen here. A `LIST` has the four bytes naming its
/// type read as well, since lofty walks the items of a `LIST INFO` even past
/// the end of the file; this one is no `INFO`, and nothing more is read.
#[test]
fn a_chunk_larger_than_the_file_is_skipped_unread() {
    let payload = vec![b'A'; 8192];
    let mut body = b"WAVE".to_vec();
    body.extend_from_slice(b"LIST");
    body.extend_from_slice(&u32::MAX.to_le_bytes()); // declares ~4 GiB
    body.extend_from_slice(&payload);
    let mut riff = b"RIFF".to_vec();
    riff.extend_from_slice(&(body.len() as u32).to_le_bytes());
    riff.extend_from_slice(&body);
    let mut budget = 64_u64;
    let mut reader = CountingReader::new(Cursor::new(riff));
    assert_eq!(check_chunks(&mut reader, 0, false, &mut budget), Ok(()));
    // Only the eight-byte header of the oversized chunk and its list type; its
    // body is never read.
    assert!(
        reader.read <= 12,
        "the walk read {} bytes of a chunk no reader would read a body of",
        reader.read
    );
}

/// A reader that counts the bytes read through it, to prove a read stopped at
/// the budget rather than running to the declared size.
struct CountingReader<R> {
    inner: R,
    read: u64,
}

impl<R> CountingReader<R> {
    fn new(inner: R) -> Self {
        Self { inner, read: 0 }
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        Ok(n)
    }
}

impl<R: Seek> Seek for CountingReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn bytes_that_are_not_a_tag_pass() {
    assert_eq!(check_tag(b"not a tag at all"), Ok(()));
    assert_eq!(check_file_bytes(&audio()), Ok(()));
    assert_eq!(check_tag(&[]), Ok(()));
}

/// A reader that injects a one-off interrupted read, then a hard read error,
/// at chosen byte offsets, to prove the walk retries the first and refuses on
/// the second rather than taking either for the end of the file.
struct FlakyReader {
    inner: Cursor<Vec<u8>>,
    interrupt_at: Option<u64>,
    fail_at: Option<u64>,
}

impl FlakyReader {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            inner: Cursor::new(bytes),
            interrupt_at: None,
            fail_at: None,
        }
    }
}

impl Read for FlakyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let pos = self.inner.position();
        if let Some(at) = self.fail_at {
            if pos >= at {
                // One-off, like a transient I/O error: a later read of the same
                // bytes succeeds, which is what tells a guard that gave up on
                // the failure from one that keeps the byte budget's refusal.
                self.fail_at = None;
                return Err(io::Error::other("a read that fails partway"));
            }
        }
        if let Some(at) = self.interrupt_at {
            if pos >= at {
                self.interrupt_at = None;
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
        }
        // Stop a read just before the fail point so the error lands midway.
        let limit = self
            .fail_at
            .map_or(buf.len(), |at| (at.saturating_sub(pos)) as usize)
            .clamp(1, buf.len());
        self.inner.read(&mut buf[..limit])
    }
}

impl Seek for FlakyReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// An interrupted read is retried, not taken for the end of the file, so a
/// benign file read through one such hiccup still passes rather than being
/// refused.
#[test]
fn an_interrupted_read_is_retried() {
    let bytes = mp3(&tag(&[title(4, "a plain title, no chapters")].concat()));
    let mut reader = FlakyReader::new(bytes);
    reader.interrupt_at = Some(0);
    assert_eq!(check_reader(&mut reader), Ok(()));
}

/// A read that fails partway is refused, not taken for the end of a clean file:
/// the parser that runs next may read past the failure.
#[test]
fn a_read_that_fails_partway_is_refused() {
    let mut bytes = vec![b'J'; 2000];
    bytes.extend_from_slice(&tag(&nested(4, &[b"CHAP"], 64)));
    let mut reader = FlakyReader::new(bytes);
    reader.fail_at = Some(1000);
    assert_eq!(check_reader(&mut reader), Err(Refusal::ReadFailed));
}

/// An interruption does not stop the walk short: a nested tag that follows is
/// still reached and refused, proving the read continued past the interruption
/// rather than taking it for the end of the file.
#[test]
fn an_interrupted_read_still_reaches_a_later_tag() {
    let mut bytes = vec![b'J'; 64];
    bytes.extend_from_slice(&tag(&nested(4, &[b"CHAP"], 64)));
    let mut reader = FlakyReader::new(bytes);
    reader.interrupt_at = Some(0);
    assert_eq!(check_reader(&mut reader), Err(Refusal::Nested));
}

/// An I/O error while reading a tag's declared body is refused, not swallowed
/// as "no tag here". The tag sits past the head window so the failure lands in
/// the tag read rather than the head scan, and it is a one-off, so a guard that
/// gave up on it (rather than refusing) would pass the file on the retry.
#[test]
fn a_tag_whose_body_read_fails_is_refused() {
    let mut bytes = b"ID3\x04\0\0".to_vec();
    bytes.extend_from_slice(&synchsafe_bytes(200_000));
    bytes.resize(10 + 200_000, 0);
    let mut reader = FlakyReader::new(bytes);
    reader.fail_at = Some(100_000);
    assert_eq!(check_reader(&mut reader), Err(Refusal::ReadFailed));
}

/// The same for the read that walks a run of empty chunks: a `RIFF` whose first
/// chunk header is zeros sends the walk into `past_zero_chunks`, and a read that
/// fails there, past the head window, is refused rather than taken for the end.
#[test]
fn a_failed_read_walking_empty_chunks_is_refused() {
    // "RIFF" <size> "WAVE", then a zeroed chunk header and zeros to the end.
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.resize(120_000, 0);
    let mut reader = FlakyReader::new(bytes);
    reader.fail_at = Some(100_000);
    assert_eq!(check_reader(&mut reader), Err(Refusal::ReadFailed));
}

/// The same for the body of a chunk the walk reads: one that fits in the file
/// cannot end early, so a read of it that fails is an I/O error, and the walk
/// that took it for "nothing here" would skip the check of whatever the body
/// holds. Here an `id3 ` chunk runs past the head window, and its read fails
/// there.
#[test]
fn a_chunk_whose_body_read_fails_is_refused() {
    let mut body = tag(&title(4, "a plain title, no chapters"));
    body.resize(2 * HEAD_CHUNK, 0);
    let mut reader = FlakyReader::new(wav(&body));
    reader.fail_at = Some(HEAD_CHUNK as u64 + 1000);
    assert_eq!(check_reader(&mut reader), Err(Refusal::ReadFailed));
}

// Helpers that drive the reader-based checks over an in-memory file.
fn check_file_bytes(bytes: &[u8]) -> Result<(), Refusal> {
    check_reader(&mut Cursor::new(bytes.to_vec()))
}

fn check_dsd_bytes(bytes: &[u8]) -> Result<(), Refusal> {
    // Through a real file and the production entry point, so the DSF/DFF
    // dispatch itself is what these tests exercise.
    let file = tempfile::NamedTempFile::new().expect("temporary DSD file");
    std::fs::write(file.path(), bytes).expect("write temporary DSD file");
    check_dsd(file.path())
}
