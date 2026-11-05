//! The files of #428: past what the guard will read, whose metadata it must
//! settle without reading the audio, and the behaviours of lofty that let it.

use super::*;

/// More than the check may read of one file, so a walk that reads the audio
/// runs out of budget on it.
const PAST_BUDGET: usize = MAX_SCAN_BYTES as usize + 6 * 1024 * 1024;

/// What a check that leaves the audio alone stays far under.
const A_FEW_HEAD_READS: u64 = 1024 * 1024;

/// Pseudo-random bytes standing for audio: no run of zeros for the walk to step
/// over and no chunk a header could be told from, [`PAST_BUDGET`] of them.
fn noise() -> &'static [u8] {
    static NOISE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    NOISE.get_or_init(|| {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut out = Vec::with_capacity(PAST_BUDGET + 8);
        while out.len() < PAST_BUDGET {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            out.extend_from_slice(&state.to_le_bytes());
        }
        out.truncate(PAST_BUDGET);
        out
    })
}

/// One chunk, with no padding byte behind it whatever its length.
fn chunk(id: &[u8; 4], body: &[u8], big_endian: bool) -> Vec<u8> {
    let size = body.len() as u32;
    let mut out = id.to_vec();
    out.extend_from_slice(&if big_endian {
        size.to_be_bytes()
    } else {
        size.to_le_bytes()
    });
    out.extend_from_slice(body);
    out
}

fn wav_format() -> Vec<u8> {
    let mut body = Vec::new();
    for field in [1_u16, 1] {
        body.extend_from_slice(&field.to_le_bytes());
    }
    body.extend_from_slice(&8000_u32.to_le_bytes());
    body.extend_from_slice(&16_000_u32.to_le_bytes());
    for field in [2_u16, 16] {
        body.extend_from_slice(&field.to_le_bytes());
    }
    chunk(b"fmt ", &body, false)
}

/// A WAV of `chunks` behind its `fmt `.
fn wav_of(chunks: &[u8]) -> Vec<u8> {
    let body = [b"WAVE".as_slice(), &wav_format(), chunks].concat();
    [
        b"RIFF".as_slice(),
        &(body.len() as u32).to_le_bytes(),
        &body,
    ]
    .concat()
}

/// An AIFF of `chunks` behind its `COMM`.
fn aiff_of(chunks: &[u8]) -> Vec<u8> {
    let mut common = 1_u16.to_be_bytes().to_vec();
    common.extend_from_slice(&800_u32.to_be_bytes());
    common.extend_from_slice(&16_u16.to_be_bytes());
    common.extend_from_slice(&[0x40, 0x0b, 0xfa, 0, 0, 0, 0, 0, 0, 0]);
    let body = [b"AIFF".as_slice(), &chunk(b"COMM", &common, true), chunks].concat();
    [
        b"FORM".as_slice(),
        &(body.len() as u32).to_be_bytes(),
        &body,
    ]
    .concat()
}

/// A FLAC made with `flac --keep-foreign-metadata`: the header of the WAV it
/// was encoded from and each of its chunks sit in `APPLICATION` blocks of their
/// own, so the letters `RIFF` are followed by a block header, not by a chunk.
fn flac_keeping_its_riff_chunks(audio: &[u8]) -> Vec<u8> {
    let block = |kind: u8, body: &[u8]| {
        let mut out = vec![kind];
        out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(body);
        out
    };
    let mut header = b"riffRIFF".to_vec();
    header.extend_from_slice(&(audio.len() as u32 + 36).to_le_bytes());
    header.extend_from_slice(b"WAVE");
    let mut data = b"riffdata".to_vec();
    data.extend_from_slice(&(audio.len() as u32).to_le_bytes());
    let mut out = b"fLaC".to_vec();
    out.extend_from_slice(&block(0, &[0x10; 34]));
    out.extend_from_slice(&block(2, &header));
    out.extend_from_slice(&block(2, &[b"riff".as_slice(), &wav_format()].concat()));
    out.extend_from_slice(&block(0x82, &data));
    out.extend_from_slice(audio);
    out
}

/// The file passes, and the check never read its audio to say so.
fn assert_passes_unread(file: Vec<u8>) {
    let mut reader = CountingReader::new(Cursor::new(file));
    assert_eq!(check_reader(&mut reader), Ok(()));
    assert!(
        reader.read < A_FEW_HEAD_READS,
        "the check read {} bytes of a file whose tags are a few dozen",
        reader.read
    );
}

/// The first shape of #428. Behind a chunk of odd length the walk went on from
/// the padding byte as well as from behind it, read the `data` header one byte
/// off, and so read the audio as a chunk of some other name until the budget
/// ran out.
#[test]
fn a_large_wav_with_an_odd_chunk_ahead_of_its_audio_passes() {
    let mut chunks = chunk(b"iXML", b"<x/>\n", false);
    chunks.push(0);
    chunks.extend_from_slice(&chunk(b"data", noise(), false));
    assert_passes_unread(wav_of(&chunks));
}

#[test]
fn a_large_aiff_with_an_odd_chunk_ahead_of_its_audio_passes() {
    let mut chunks = chunk(b"NAME", b"Title", true);
    chunks.push(0);
    let sound = [[0_u8; 8].as_slice(), noise()].concat();
    chunks.extend_from_slice(&chunk(b"SSND", &sound, true));
    assert_passes_unread(aiff_of(&chunks));
}

/// The second shape of #428: `RIFF` in the head of a file that is no WAV. The
/// walk it starts reads a block header as a chunk, whose length is then the
/// letters `riff`, and the rest of the file was read as that chunk.
#[test]
fn a_large_flac_that_keeps_its_riff_chunks_passes() {
    assert_passes_unread(flac_keeping_its_riff_chunks(noise()));
}

/// An encoder that pads nothing leaves the next chunk where the padding byte
/// would be. Both continuations are walked then, and the wrong one, which reads
/// the `data` header one byte off, must not cost the file its place either.
#[test]
fn a_large_wav_whose_odd_chunk_was_never_padded_passes() {
    let mut chunks = chunk(b"iXML", b"<x/>\n", false);
    chunks.extend_from_slice(&chunk(b"data", noise(), false));
    assert_eq!(check_reader(&mut Cursor::new(wav_of(&chunks))), Ok(()));
}

/// The leniency is only for the audio behind a chunk, never for a nested tag:
/// an `id3`/`ID3 ` chunk behind an odd one is still reached and refused,
/// whatever stands where its padding byte would, since both continuations are
/// walked. A `JUNK` chunk the size of the head read stands first, so the tag
/// lies past the signature search of the head and only the chunk walk can
/// reach it.
#[test]
fn nesting_behind_an_odd_chunk_is_refused_whatever_pads_it() {
    let hostile = tag(&nested(4, &[b"CHAP"], 64));
    let junk = vec![0_u8; HEAD_CHUNK];
    for pad in [&[][..], &[0], &[1], b"X"] {
        let mut chunks = chunk(b"JUNK", &junk, false);
        chunks.extend_from_slice(&chunk(b"iXML", b"<x/>\n", false));
        chunks.extend_from_slice(pad);
        chunks.extend_from_slice(&chunk(b"id3 ", &hostile, false));
        chunks.extend_from_slice(&chunk(b"data", &[0; 1600], false));
        assert_eq!(
            check_file_bytes(&wav_of(&chunks)),
            Err(Refusal::Nested),
            "WAV, padding {pad:?}"
        );
        let mut chunks = chunk(b"JUNK", &junk, true);
        chunks.extend_from_slice(&chunk(b"NAME", b"Title", true));
        chunks.extend_from_slice(pad);
        chunks.extend_from_slice(&chunk(b"ID3 ", &hostile, true));
        chunks.extend_from_slice(&chunk(b"SSND", &[0; 1608], true));
        assert_eq!(
            check_file_bytes(&aiff_of(&chunks)),
            Err(Refusal::Nested),
            "AIFF, padding {pad:?}"
        );
    }
}

/// The ID3v2 title lofty reads from `file` taken as `kind`, tags only — a file
/// whose chunks lead nowhere has no audio properties to read, so properties are
/// turned off to reach the tag the file does carry.
fn title_lofty_reads(file: Vec<u8>, kind: lofty::file::FileType) -> Option<String> {
    title_lofty_reads_from(Cursor::new(file), kind)
}

/// The same, from a reader rather than bytes.
fn title_lofty_reads_from<R: Read + Seek>(file: R, kind: lofty::file::FileType) -> Option<String> {
    use lofty::file::TaggedFileExt;
    use lofty::prelude::Accessor;
    let options = lofty::config::ParseOptions::new().read_properties(false);
    let tagged = lofty::probe::Probe::with_file_type(file, kind)
        .options(options)
        .read()
        .ok()?;
    let tag = tagged.tag(lofty::tag::TagType::Id3v2)?;
    tag.title().map(|title| title.into_owned())
}

/// Why the walk follows a `RIFF`/`FORM` only at the head of the file: lofty
/// reads IFF chunks only when the file opens with them, so one deeper in leads
/// to nothing it reads. This holds lofty to that — the control reads the tag
/// from a file that opens with the signature, and every file that does not
/// opens hides it.
#[test]
fn lofty_reads_chunks_only_in_a_file_that_opens_with_them() {
    use lofty::file::FileType;
    let honest = tag(&title(4, "found"));
    for (file, kind) in [
        (wav(&honest), FileType::Wav),
        (aiff(&honest), FileType::Aiff),
    ] {
        assert_eq!(
            title_lofty_reads(file.clone(), kind).as_deref(),
            Some("found"),
            "the control: {kind:?} read from its first byte"
        );
        for ahead in [&[0_u8][..], &[0; 3], b"junk", &[0xff, 0xfb, 0x90, 0x00]] {
            let moved = [ahead, &file].concat();
            assert_eq!(
                title_lofty_reads(moved, kind),
                None,
                "{kind:?} behind {ahead:?}"
            );
        }
    }
}

/// More empty chunks than the walk follows, behind the letters that start one.
fn riff_of_too_many_chunks() -> Vec<u8> {
    let mut body = b"WAVE".to_vec();
    for _ in 0..=MAX_CHUNKS {
        body.extend_from_slice(b"junk\0\0\0\0");
    }
    [
        b"RIFF".as_slice(),
        &(body.len() as u32).to_le_bytes(),
        &body,
    ]
    .concat()
}

/// In a file that opens with `RIFF`, lofty walks as far as the chunks go, so a
/// walk that gives up before the end has not checked what lofty will read:
/// refused, and for an exhausted budget, not for a nesting nobody saw.
#[test]
fn a_wav_of_more_chunks_than_the_walk_follows_is_refused_as_unchecked() {
    assert_eq!(
        check_file_bytes(&riff_of_too_many_chunks()),
        Err(Refusal::OverBudget)
    );
}

/// The same chunks behind four bytes of something else are in a file no reader
/// takes for a WAV. The walk is not begun, and the file passes.
#[test]
fn a_walk_begun_inside_a_file_gives_up_without_refusing_it() {
    let file = [b"\xff\xfb\x90\0".as_slice(), &riff_of_too_many_chunks()].concat();
    assert_eq!(check_file_bytes(&file), Ok(()));
}

/// A chunk header whose read fails is refused, not passed over: the walk that
/// skipped it never reached the chunk, whose tag lofty goes on to read. Here a
/// nested tag stands behind a `JUNK` chunk larger than the head read, so only
/// the chunk walk reaches it, and the read of its header fails once. The
/// control, read without the failure, is refused for the nesting.
#[test]
fn a_chunk_header_whose_read_fails_is_refused() {
    let hostile = tag(&nested(4, &[b"CHAP"], 64));
    let mut chunks = chunk(b"JUNK", &vec![0_u8; 2 * HEAD_CHUNK], false);
    let header_at = 12 + wav_format().len() + chunks.len();
    chunks.extend_from_slice(&chunk(b"id3 ", &hostile, false));
    let file = wav_of(&chunks);
    assert_eq!(&file[header_at..header_at + 4], b"id3 ");
    assert_eq!(check_file_bytes(&file), Err(Refusal::Nested), "the control");
    let mut reader = FlakyReader::new(file);
    reader.fail_at = Some(header_at as u64);
    assert_eq!(check_reader(&mut reader), Err(Refusal::ReadFailed));
}

/// The two refusals read differently, since one of them found no nesting.
#[test]
fn an_exhausted_budget_does_not_claim_a_nesting() {
    assert!(Refusal::Nested.to_string().contains("nests chapter frames"));
    assert!(!Refusal::OverBudget.to_string().contains("nests"));
    assert!(!Refusal::ReadFailed.to_string().contains("nests"));
}

/// A file of `len` bytes: `head`, then pseudo-random bytes made as they are
/// read, so a file of gigabytes costs no memory. Counts what is read, and fails
/// a read past [`A_FEW_HEAD_READS`]: a check that got there is reading the
/// audio already, and stopping it there keeps a failing sweep from reading a
/// budget's worth of every file before it says so.
struct Virtual {
    head: Vec<u8>,
    /// Where in the made-up audio some chosen bytes stand, and those bytes.
    island: (u64, Vec<u8>),
    len: u64,
    pos: u64,
    read: u64,
}

impl Virtual {
    fn new(head: Vec<u8>, len: u64) -> Self {
        Self {
            head,
            island: (0, Vec::new()),
            len,
            pos: 0,
            read: 0,
        }
    }
}

impl Read for Virtual {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.read >= A_FEW_HEAD_READS {
            return Err(io::Error::other("the check is reading the audio"));
        }
        let count = (buf.len() as u64).min(self.len.saturating_sub(self.pos)) as usize;
        for (at, byte) in (self.pos..).zip(&mut buf[..count]) {
            let island = at
                .checked_sub(self.island.0)
                .and_then(|offset| usize::try_from(offset).ok())
                .and_then(|offset| self.island.1.get(offset));
            let chosen = usize::try_from(at).ok().and_then(|at| self.head.get(at));
            *byte = match chosen.or(island) {
                Some(value) => *value,
                None => {
                    let mut x = (at / 8).wrapping_mul(0x9e37_79b9_7f4a_7c15);
                    x ^= x >> 29;
                    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
                    x ^= x >> 32;
                    (x >> (8 * (at % 8))) as u8
                }
            };
        }
        self.pos += count as u64;
        self.read += count as u64;
        Ok(count)
    }
}

impl Seek for Virtual {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.pos = match pos {
            SeekFrom::Start(at) => at,
            SeekFrom::End(delta) => self.len.saturating_add_signed(delta),
            SeekFrom::Current(delta) => self.pos.saturating_add_signed(delta),
        };
        Ok(self.pos)
    }
}

/// A WAV whose `data` chunk declares `audio` bytes, behind a five-byte `iXML`
/// and `pad` where its padding byte goes.
fn virtual_wav(pad: &[u8], audio: u32) -> Virtual {
    let mut chunks = chunk(b"iXML", b"<x/>\n", false);
    chunks.extend_from_slice(pad);
    chunks.extend_from_slice(b"data");
    chunks.extend_from_slice(&audio.to_le_bytes());
    let head = wav_of(&chunks);
    let len = head.len() as u64 + u64::from(audio);
    Virtual::new(head, len)
}

/// The same as an AIFF: `COMM`, a five-byte `NAME`, then an `SSND` declaring
/// `audio` bytes, the first eight of them its zero offset and block size.
fn virtual_aiff(pad: &[u8], audio: u32) -> Virtual {
    let mut chunks = chunk(b"NAME", b"Title", true);
    chunks.extend_from_slice(pad);
    chunks.extend_from_slice(b"SSND");
    chunks.extend_from_slice(&audio.to_be_bytes());
    chunks.extend_from_slice(&[0; 8]);
    let head = aiff_of(&chunks);
    let len = head.len() as u64 - 8 + u64::from(audio);
    Virtual::new(head, len)
}

/// Whether the check passed `file` without reading its audio to say so.
fn passes_unread(mut file: Virtual) -> Result<(), String> {
    let verdict = check_reader(&mut file);
    if verdict == Ok(()) && file.read < A_FEW_HEAD_READS {
        Ok(())
    } else {
        Err(format!(
            "{verdict:?} after reading {} of {} bytes",
            file.read, file.len
        ))
    }
}

const MIB: u32 = 1024 * 1024;

/// The sizes the first shape of #428 depends on. The walk that goes on from a
/// zero padding byte reads the `data` header one byte off: in a WAV its size is
/// then `0x61 + 256 × (the low 24 bits of the real size)`, in an AIFF
/// `0x4400_0000 + (the real size >> 8)`. Wherever that lands inside the file,
/// the misread chunk fits and its body — the audio — is read. These sizes are
/// picked so that it does: past the budget, under it, and in an AIFF past the
/// 1.06 GiB where the misread size first fits.
#[test]
fn an_odd_chunk_passes_whatever_size_of_audio_follows_it() {
    for audio in [70 * MIB, 0x1008_0000, 0x0802_0000, 0x0800_0400, 0x1060_0000] {
        assert_eq!(
            passes_unread(virtual_wav(&[0], audio)),
            Ok(()),
            "WAV of {audio:#x} bytes of audio"
        );
    }
    for audio in [1000 * MIB, 1100 * MIB, 1500 * MIB, 3000 * MIB] {
        assert_eq!(
            passes_unread(virtual_aiff(&[0], audio)),
            Ok(()),
            "AIFF of {audio:#x} bytes of audio"
        );
    }
}

/// Files of a sweep of audio sizes behind an odd chunk and `pad`, WAV and AIFF:
/// how many there were, and each the check refused or read the audio of.
fn sweep(pad: &[u8]) -> (usize, Vec<String>) {
    let mut total = 0;
    let mut failures = Vec::new();
    for base in [100 * MIB, 300 * MIB, 650 * MIB, 1500 * MIB] {
        for step in 0..48 {
            // A multiple of four is the size of some 16-bit stereo track.
            let audio = base + step * 1_000_004;
            total += 1;
            if let Err(why) = passes_unread(virtual_wav(pad, audio)) {
                failures.push(format!("WAV, pad {pad:?}, {audio:#x}: {why}"));
            }
        }
    }
    for base in [600 * MIB, 1100 * MIB, 2500 * MIB, 3500 * MIB] {
        for step in 0..48 {
            let audio = base + step * 1_000_004;
            total += 1;
            if let Err(why) = passes_unread(virtual_aiff(pad, audio)) {
                failures.push(format!("AIFF, pad {pad:?}, {audio:#x}: {why}"));
            }
        }
    }
    (total, failures)
}

/// The same over a sweep of sizes rather than chosen ones, behind a zero
/// padding byte and behind a non-zero one that spells no chunk name with the
/// three letters after it, which lofty skips as padding too. Not one refused.
#[test]
fn a_sweep_of_audio_sizes_behind_a_padded_odd_chunk_passes() {
    for pad in [0_u8, 1] {
        let (_, failures) = sweep(&[pad]);
        assert!(failures.is_empty(), "{failures:#?}");
    }
}

/// Behind a padding byte left out, or written as a letter, the wrong
/// continuation reads the audio's header one byte off from the other side,
/// and that misread chunk's body is left unread as well. What follows it is
/// still walked, though — lofty walks it when the padding is a letter — and
/// that lands inside the audio: on four printable bytes declaring a size that
/// fits, the walk reads on into the audio and the file is refused for the
/// budget. That is a known residue of #428, a few files in a thousand of these
/// malformed ones; this holds it there, against the third to all of them
/// refused before the misread was recognised.
#[test]
fn a_sweep_of_audio_sizes_behind_a_malformed_padding_mostly_passes() {
    for pad in [&[][..], b"X"] {
        let (total, failures) = sweep(pad);
        assert!(failures.len() * 50 <= total, "{failures:#?}");
    }
}

/// The audio's header misread one byte off has its body left unread, and that
/// must hide nothing: a nested tag in a chunk behind the audio is still reached
/// from the continuation that read the audio's header right, whatever stands
/// where the padding byte goes. A `JUNK` chunk the size of the head read
/// stands first, so only the chunk walk can reach the tag.
#[test]
fn nesting_behind_the_audio_of_an_odd_chunk_is_refused_whatever_pads_it() {
    let hostile = tag(&nested(4, &[b"CHAP"], 64));
    let junk = vec![0_u8; HEAD_CHUNK];
    for pad in [&[][..], &[0], &[1], b"X"] {
        let mut chunks = chunk(b"JUNK", &junk, false);
        chunks.extend_from_slice(&chunk(b"iXML", b"<x/>\n", false));
        chunks.extend_from_slice(pad);
        chunks.extend_from_slice(&chunk(b"data", &[0x55; 1600], false));
        chunks.extend_from_slice(&chunk(b"id3 ", &hostile, false));
        assert_eq!(
            check_file_bytes(&wav_of(&chunks)),
            Err(Refusal::Nested),
            "WAV, padding {pad:?}"
        );
        let mut chunks = chunk(b"JUNK", &junk, true);
        chunks.extend_from_slice(&chunk(b"NAME", b"Title", true));
        chunks.extend_from_slice(pad);
        chunks.extend_from_slice(&chunk(b"SSND", &[0x55; 1608], true));
        chunks.extend_from_slice(&chunk(b"ID3 ", &hostile, true));
        assert_eq!(
            check_file_bytes(&aiff_of(&chunks)),
            Err(Refusal::Nested),
            "AIFF, padding {pad:?}"
        );
    }
}

/// A file behind a `JUNK` chunk the size of the head read, a five-byte chunk and
/// a letter `X` for its padding byte, whose audio declares `audio` bytes and
/// holds `tag`, in a tag chunk, exactly where the audio's header read from that
/// letter ends: in a WAV, `Xdat` of `0x61 + 256 × (audio & 0xFF_FFFF)` bytes;
/// in an AIFF, `XSSN` of `0x4400_0000 + (audio >> 8)`. Its `RIFF`/`FORM` size
/// covers the whole file, as lofty needs to walk that far.
fn tag_where_the_misread_audio_ends(big_endian: bool, audio: u32, tag: &[u8]) -> Virtual {
    let junk = vec![0_u8; HEAD_CHUNK];
    let (mut head, len, letter, misread, tag_name) = if big_endian {
        let mut chunks = chunk(b"JUNK", &junk, true);
        chunks.extend_from_slice(&chunk(b"NAME", b"Title", true));
        chunks.extend_from_slice(b"XSSND");
        chunks.extend_from_slice(&audio.to_be_bytes());
        chunks.extend_from_slice(&[0; 8]);
        let head = aiff_of(&chunks);
        let len = head.len() as u64 - 8 + u64::from(audio);
        let letter = head.len() as u64 - 17;
        (
            head,
            len,
            letter,
            0x4400_0000 + u64::from(audio >> 8),
            b"ID3 ",
        )
    } else {
        let mut chunks = chunk(b"JUNK", &junk, false);
        chunks.extend_from_slice(&chunk(b"iXML", b"<x/>\n", false));
        chunks.extend_from_slice(b"Xdata");
        chunks.extend_from_slice(&audio.to_le_bytes());
        let head = wav_of(&chunks);
        let len = head.len() as u64 + u64::from(audio);
        let letter = head.len() as u64 - 9;
        (
            head,
            len,
            letter,
            0x61 + 256 * u64::from(audio & 0xff_ffff),
            b"id3 ",
        )
    };
    let form = (len - 8) as u32;
    head[4..8].copy_from_slice(&if big_endian {
        form.to_be_bytes()
    } else {
        form.to_le_bytes()
    });
    let island = chunk(tag_name, tag, big_endian);
    let at = letter + 8 + misread;
    assert!(
        at > head.len() as u64 && at + island.len() as u64 <= len,
        "the misread chunk must end inside the audio"
    );
    let mut file = Virtual::new(head, len);
    file.island = (at, island);
    file
}

/// Behind a padding byte written as a letter, lofty takes the wrong
/// continuation itself: the letter and the three after it form a valid name, so
/// it reads the audio's header one byte off, skips the misread size and reads
/// on from there, inside the audio. A tag chunk standing exactly there is one
/// lofty reads, so the guard must walk on behind a misread chunk even though it
/// leaves its body unread. This holds lofty to reading the honest tag there,
/// and the guard to refusing the hostile one. In a WAV the misread chunk ends
/// inside the audio only from 16 MiB of it — below, `256 ×` its size is past the
/// end — and in an AIFF from about 1.07 GiB; the reader makes either up as it is
/// read. The AIFF is tried with a misread size even and odd, its padding byte
/// then being the tag chunk's first letter, which lofty re-reads from.
#[test]
fn a_tag_where_the_misread_audio_header_ends_is_reached() {
    use lofty::file::FileType;
    let honest = tag(&title(4, "found"));
    let hostile = tag(&nested(4, &[b"CHAP"], 64));
    let mut failures = Vec::new();
    for (kind, big_endian, audio) in [
        (FileType::Wav, false, 0x0100_0010),
        (FileType::Wav, false, 0x1008_0000),
        (FileType::Aiff, true, 0x4800_0000),
        (FileType::Aiff, true, 0x4800_0100),
    ] {
        let file = tag_where_the_misread_audio_ends(big_endian, audio, &honest);
        let read = title_lofty_reads_from(file, kind);
        if read.as_deref() != Some("found") {
            failures.push(format!("lofty, {kind:?} of {audio:#x}: {read:?}"));
        }
        let mut file = tag_where_the_misread_audio_ends(big_endian, audio, &hostile);
        let verdict = check_reader(&mut file);
        if verdict != Err(Refusal::Nested) {
            failures.push(format!(
                "the guard, {kind:?} of {audio:#x}: {verdict:?} after reading {}",
                file.read
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Every four bytes over an alphabet that reaches each edge of lofty's rule: a
/// control byte, a space, a letter, the last printable byte, and two past it.
fn names() -> impl Iterator<Item = [u8; 4]> {
    const ALPHABET: [u8; 7] = [0x00, 0x1f, b' ', b'a', b'~', 0x7f, 0x80];
    (0..ALPHABET.len().pow(4)).map(|mut index| {
        let mut name = [0_u8; 4];
        for byte in &mut name {
            *byte = ALPHABET[index % ALPHABET.len()];
            index /= ALPHABET.len();
        }
        name
    })
}

/// Why the walk ends a continuation at a chunk whose name is no FourCC: lofty
/// ends its whole walk there, so nothing behind that point is read by it along
/// that path. This holds lofty to it, and holds [`is_fourcc`] to lofty's rule
/// byte for byte — a name the guard rejects that lofty accepted would let a
/// tag behind it through unchecked.
#[test]
fn lofty_reads_no_chunk_behind_a_name_that_is_no_fourcc() {
    use lofty::file::FileType;
    let honest = tag(&title(4, "found"));
    for name in names() {
        let wav = wav_of(&[chunk(&name, &[], false), chunk(b"id3 ", &honest, false)].concat());
        let aiff = aiff_of(&[chunk(&name, &[], true), chunk(b"ID3 ", &honest, true)].concat());
        let expected = is_fourcc(name).then_some("found");
        assert_eq!(
            title_lofty_reads(wav, FileType::Wav).as_deref(),
            expected,
            "WAV behind {name:?}"
        );
        assert_eq!(
            title_lofty_reads(aiff, FileType::Aiff).as_deref(),
            expected,
            "AIFF behind {name:?}"
        );
    }
}

/// Why the walk reads nothing of a chunk that declares more than the file
/// holds: lofty reads no tag from it either. Past what is left counting the
/// header, it stops at the header; within the eight bytes of the header it
/// takes the chunk and then fails to read its body. Both sides of that edge are
/// here, with a control that fits.
#[test]
fn lofty_reads_no_tag_from_a_chunk_that_runs_past_the_file() {
    use lofty::file::FileType;
    let honest = tag(&title(4, "found"));
    for over in [0_u32, 1, 7, 8, 9, 1000] {
        let declared = honest.len() as u32 + over;
        let wav = wav_of(&[b"id3 ", &declared.to_le_bytes()[..], &honest].concat());
        let aiff = aiff_of(&[b"ID3 ", &declared.to_be_bytes()[..], &honest].concat());
        let expected = (over == 0).then_some("found");
        assert_eq!(
            title_lofty_reads(wav, FileType::Wav).as_deref(),
            expected,
            "WAV, {over} bytes past the end"
        );
        assert_eq!(
            title_lofty_reads(aiff, FileType::Aiff).as_deref(),
            expected,
            "AIFF, {over} bytes past the end"
        );
    }
}
