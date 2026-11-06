//! A differential walk: built WAV and AIFF files of malformed chunk layouts,
//! each with one tag chunk somewhere, read by lofty and by the guard's chunk
//! walk. Wherever lofty hands the tag chunk to its ID3v2 parser, the guard must
//! refuse the hostile twin of that file.
//!
//! The generator is seeded and deterministic, so a seed names one file for
//! good. It covers what it can build and no more: a handful of chunk names,
//! sizes off by a little or declaring `u32::MAX`, padding bytes that are zero,
//! a letter or missing, bare names and bare headers inside a `LIST`, and runs
//! of lists whose last item runs past their end. The AIFF side has nothing as
//! adversarial as a `LIST INFO`. It calls `check_chunks` directly, so it holds
//! the chunk walk, not the head scan. That no file passed is evidence over
//! the files it built, not a proof.

use super::large_files::{aiff_of, chunk, wav_format, wav_of};
use super::*;

/// A xorshift generator: the same seed rolls the same file.
struct Dice(u64);

impl Dice {
    /// The next 64 bits.
    fn roll(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// A number from zero to `n`, `n` left out.
    fn below(&mut self, n: usize) -> usize {
        ((self.roll() >> 11) % n as u64) as usize
    }
    /// Whether an event of `percent` in a hundred happens.
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
    /// Up to `max` bytes of an alphabet of zeros, control bytes, letters,
    /// a space and bytes past the printable range.
    fn bytes(&mut self, max: usize) -> Vec<u8> {
        const ALPHABET: [u8; 10] = [0, 0, 1, b'I', b'a', b'x', b' ', 0xff, b'd', 0x7f];
        let n = self.below(max + 1);
        (0..n)
            .map(|_| ALPHABET[self.below(ALPHABET.len())])
            .collect()
    }
    /// The size a header declares for a body of `actual` bytes: mostly the
    /// truth, otherwise a little more or less, `u32::MAX`, or well past it.
    fn size(&mut self, actual: usize) -> u32 {
        let size = match self.below(20) {
            0..=11 => actual,
            12 => actual + 1 + self.below(12),
            13 => actual.saturating_sub(1 + self.below(9)),
            14 => actual + 8,
            15 => return u32::MAX,
            16 => actual + 16 + self.below(300),
            17 => actual + 4,
            18 => actual + 1,
            _ => actual + 2,
        };
        size as u32
    }
    /// The padding behind a body of `len` bytes: zero, a letter or none
    /// behind an odd one, and now and then a stray byte behind an even one.
    fn pad(&mut self, out: &mut Vec<u8>, len: usize) {
        if len % 2 == 1 {
            match self.below(4) {
                0 | 1 => out.push(0),
                2 => out.push(b'X'),
                _ => {}
            }
        } else if self.chance(5) {
            out.push(if self.chance(50) { 0 } else { b'X' });
        }
    }
}

/// A chunk name that is no FourCC: lofty ends its walk on it.
const BAD_NAME: &[u8; 4] = &[1, b'x', b'y', b'z'];
/// The name of an empty chunk, all zeros.
const NO_NAME: &[u8; 4] = &[0, 0, 0, 0];

/// A chunk header: `name`, then `size` in the byte order of the format.
fn put(out: &mut Vec<u8>, name: &[u8], size: u32, be: bool) {
    out.extend_from_slice(name);
    out.extend_from_slice(&if be {
        size.to_be_bytes()
    } else {
        size.to_le_bytes()
    });
}

/// The tag chunk holding `tag`, as `ID3 ` or `id3 `, its size now and then
/// past its body, and its padding.
fn dice_tag(dice: &mut Dice, out: &mut Vec<u8>, tag: &[u8], be: bool) {
    let size = if dice.chance(85) {
        tag.len()
    } else {
        tag.len() + 1 + dice.below(12)
    };
    let name = if dice.chance(50) { b"ID3 " } else { b"id3 " };
    put(out, name, size as u32, be);
    out.extend_from_slice(tag);
    dice.pad(out, size);
}

/// One chunk of a name the format knows, or one it rejects, with a body of
/// a few bytes — a real `fmt ` most of the time — and a size that may lie.
fn dice_plain(dice: &mut Dice, out: &mut Vec<u8>, be: bool) {
    const WAV: [&[u8; 4]; 11] = [
        b"fmt ", b"fact", b"data", b"JUNK", b"LIST", BAD_NAME, b"a b ", NO_NAME, b"Xdat", b"id3 ",
        b"bext",
    ];
    const AIFF: [&[u8; 4]; 13] = [
        b"COMM", b"SSND", b"NAME", b"ANNO", b"COMT", b"AUTH", b"(c) ", b"JUNK", b"LIST", BAD_NAME,
        NO_NAME, b"XSSN", b"ID3 ",
    ];
    let name = if be {
        AIFF[dice.below(AIFF.len())]
    } else {
        WAV[dice.below(WAV.len())]
    };
    let mut body = if name == b"fmt " && dice.chance(70) {
        wav_format()[8..].to_vec()
    } else {
        dice.bytes(24)
    };
    if matches!(name, b"id3 " | b"ID3 ") {
        body.truncate(9);
    }
    let size = dice.size(body.len());
    put(out, name, size, be);
    out.extend_from_slice(&body);
    dice.pad(out, body.len());
}

/// A `LIST`, of type `INFO` most of the time, of up to four items — bare
/// names, bare headers declaring `u32::MAX`, or items whose size may lie —
/// with `tag` among them when given.
fn dice_list(dice: &mut Dice, out: &mut Vec<u8>, tag: Option<&[u8]>, be: bool) {
    let mut body = if dice.chance(92) {
        b"INFO".to_vec()
    } else {
        b"adtl".to_vec()
    };
    let items = dice.below(5);
    let tag_at = dice.below(items + 1);
    for item in 0..=items {
        if item == tag_at {
            if let Some(tag) = tag {
                dice_tag(dice, &mut body, tag, be);
            }
        }
        if item == items {
            break;
        }
        const NAMES: [&[u8; 4]; 7] = [
            b"INAM", b"IART", b"ICMT", BAD_NAME, b"a b ", NO_NAME, b"id3 ",
        ];
        let name = NAMES[dice.below(NAMES.len())];
        match dice.below(10) {
            // A bare name, or a bare header declaring everything.
            0 | 1 => body.extend_from_slice(name),
            2 => put(&mut body, name, u32::MAX, be),
            _ => {
                let text = dice.bytes(12);
                let size = dice.size(text.len());
                put(&mut body, name, size, be);
                body.extend_from_slice(&text);
                dice.pad(&mut body, text.len());
            }
        }
    }
    let size = dice.size(body.len());
    put(out, b"LIST", size, be);
    out.extend_from_slice(&body);
    dice.pad(out, body.len());
}

/// A run of up to forty lists whose one item runs past their end, followed
/// by ones, by random bytes or by nothing: what puts lofty's count of what
/// is left of the file ahead of where it reads.
fn dice_overruns(dice: &mut Dice, out: &mut Vec<u8>, be: bool) {
    let lists = 1 + dice.below(40);
    let full = dice.chance(70);
    let filler = dice.below(3);
    for _ in 0..lists {
        let inside = dice.below(3) * 2;
        let over = if full { 8 } else { 1 + dice.below(8) };
        let mut body = b"INFO".to_vec();
        put(&mut body, b"INAM", (inside + over) as u32, be);
        body.extend(std::iter::repeat_n(b'a', inside));
        put(out, b"LIST", body.len() as u32, be);
        out.extend_from_slice(&body);
        match filler {
            0 => out.extend(std::iter::repeat_n(1_u8, over)),
            1 => {
                let junk = dice.bytes(over);
                out.extend_from_slice(&junk);
            }
            _ => {}
        }
    }
}

/// The file `seed` names: a `RIFF…WAVE` or `FORM…AIFF` of up to seven
/// elements — plain chunks, lists, runs of overrunning lists, raw bytes —
/// with `tag` in one of them. The same seed gives the same layout whatever
/// the tag, as long as its length is the same.
fn dice_file(seed: u64, be: bool, tag: &[u8]) -> Vec<u8> {
    let mut dice = Dice(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let elements = 1 + dice.below(7);
    let tag_at = dice.below(elements);
    let mut body = if be {
        b"AIFF".to_vec()
    } else {
        b"WAVE".to_vec()
    };
    for element in 0..elements {
        let with_tag = element == tag_at;
        match dice.below(10) {
            0..=3 => {
                dice_plain(&mut dice, &mut body, be);
                if with_tag {
                    dice_tag(&mut dice, &mut body, tag, be);
                }
            }
            4..=6 => {
                if with_tag && dice.chance(60) {
                    dice_list(&mut dice, &mut body, Some(tag), be);
                } else {
                    dice_list(&mut dice, &mut body, None, be);
                    if with_tag {
                        dice_tag(&mut dice, &mut body, tag, be);
                    }
                }
            }
            7 | 8 => {
                dice_overruns(&mut dice, &mut body, be);
                if with_tag {
                    if dice.chance(60) {
                        dice_list(&mut dice, &mut body, Some(tag), be);
                    } else {
                        dice_tag(&mut dice, &mut body, tag, be);
                    }
                }
            }
            _ => {
                let raw = dice.bytes(10);
                body.extend_from_slice(&raw);
                if with_tag {
                    dice_tag(&mut dice, &mut body, tag, be);
                }
            }
        }
    }
    let mut file = if be {
        b"FORM".to_vec()
    } else {
        b"RIFF".to_vec()
    };
    let size = body.len() as u32;
    file.extend_from_slice(&if be {
        size.to_be_bytes()
    } else {
        size.to_le_bytes()
    });
    file.extend_from_slice(&body);
    file
}

/// What lofty answers, read the way the server reads (default options).
fn lofty_answer(file: Vec<u8>, kind: lofty::file::FileType) -> Result<Option<String>, String> {
    let read = std::panic::catch_unwind(|| {
        lofty::probe::Probe::with_file_type(Cursor::new(file), kind).read()
    });
    match read {
        Ok(Ok(_)) => Ok(None),
        Ok(Err(error)) => {
            let mut chain = format!("{error:?}");
            let mut cause = std::error::Error::source(&error);
            while let Some(inner) = cause {
                chain.push_str(&format!(" <- {inner}"));
                cause = inner.source();
            }
            Ok(Some(chain))
        }
        Err(_) => Err("lofty panicked".to_owned()),
    }
}

/// How many seeded files of each format the sweep builds: lofty reaches the
/// tag in thousands of them, and the sweep still runs in seconds.
const ROUNDS: u64 = 100_000;

/// WAV seeds a broken guard passed, by measurement over millions of files:
/// the first twelve with the items of a `LIST` longer than the file left
/// unwalked, the last twelve with the places lofty goes on from after a
/// `LIST INFO` left unfollowed. The sweep reaches few of the first kind, so
/// these run whatever [`ROUNDS`] is.
const REGRESSIONS: [u64; 24] = [
    46047, 69675, 71471, 79378, 86001, 97735, 116237, 139474, 212026, 221819, 239550, 274521, 11,
    27, 70, 189, 227, 274, 376, 390, 406, 421, 446, 473,
];

/// What one seed came to.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// lofty never handed the tag chunk to its ID3v2 parser.
    Unreached,
    /// lofty did, and the guard refused the hostile twin.
    Refused,
    /// lofty did, and the guard passed the hostile twin: a way past it.
    Passed,
}

/// The two tags a seed is built with, and how lofty answers on reaching one.
struct Probe {
    /// A tag of the hostile one's exact length that lofty refuses to parse
    /// with an error of its own (version 9): seeing that error is seeing lofty
    /// hand the chunk to its parser, whatever it would have made of the rest
    /// of the file — a file can fail after its tag is parsed, as one with no
    /// `data` does, and a title read back would miss that.
    poison: Vec<u8>,
    /// The tag the guard must refuse.
    hostile: Vec<u8>,
    /// lofty's answer to the poison, read from a control file of each format
    /// rather than written down.
    parsed: [String; 2],
}

impl Probe {
    /// The hostile tag, its poison twin, and lofty's answer to the poison in
    /// each format.
    fn new() -> Self {
        let hostile = tag(&nested(4, &[b"CHAP"], 6));
        assert!(check_tag(&hostile).is_err(), "the guard refuses the tag");
        let mut poison = vec![b'I', b'D', b'3', 9, 0, 0];
        poison.extend_from_slice(&synchsafe_bytes(hostile.len() - 10));
        poison.resize(hostile.len(), 0);
        let answer = |file, kind| {
            lofty_answer(file, kind)
                .expect("lofty does not panic on the control")
                .expect("lofty refuses the poison")
        };
        let parsed = [
            answer(
                wav_of(&chunk(b"id3 ", &poison, false)),
                lofty::file::FileType::Wav,
            ),
            answer(
                aiff_of(&chunk(b"ID3 ", &poison, true)),
                lofty::file::FileType::Aiff,
            ),
        ];
        Self {
            poison,
            hostile,
            parsed,
        }
    }

    /// What the file of `seed` comes to: whether lofty parses its tag, and
    /// if so whether the guard refuses the hostile twin.
    fn outcome(&self, seed: u64, big_endian: bool) -> Outcome {
        let kind = if big_endian {
            lofty::file::FileType::Aiff
        } else {
            lofty::file::FileType::Wav
        };
        let answer = lofty_answer(dice_file(seed, big_endian, &self.poison), kind);
        let parsed = &self.parsed[usize::from(big_endian)];
        if !matches!(answer, Ok(Some(ref error)) if error == parsed) {
            return Outcome::Unreached;
        }
        let twin = dice_file(seed, big_endian, &self.hostile);
        let mut budget = MAX_SCAN_BYTES;
        match check_chunks(&mut Cursor::new(twin), 0, big_endian, &mut budget) {
            Err(_) => Outcome::Refused,
            Ok(()) => Outcome::Passed,
        }
    }
}

/// Over [`ROUNDS`] seeded files of each format, and the seeds of
/// [`REGRESSIONS`], no file whose tag lofty parses passes the guard. The
/// sweep must also reach the tag in one file in twenty at least, so a
/// generator that stopped building files lofty reads them in fails here
/// rather than passing on nothing.
#[test]
fn a_seeded_sweep_of_malformed_chunks_passes_no_tag_lofty_parses() {
    let probe = Probe::new();
    let mut failures = Vec::new();
    for big_endian in [false, true] {
        let (mut reached, mut passed) = (0_u64, Vec::new());
        for seed in 1..=ROUNDS {
            match probe.outcome(seed, big_endian) {
                Outcome::Unreached => {}
                Outcome::Refused => reached += 1,
                Outcome::Passed => {
                    reached += 1;
                    passed.push(seed);
                }
            }
        }
        let format = if big_endian { "AIFF" } else { "WAV" };
        if !passed.is_empty() {
            failures.push(format!("{format}: the guard passed seeds {passed:?}"));
        }
        if reached * 20 < ROUNDS {
            failures.push(format!(
                "{format}: lofty parsed the tag in {reached} files only"
            ));
        }
    }
    for seed in REGRESSIONS {
        let outcome = probe.outcome(seed, false);
        if outcome != Outcome::Refused {
            failures.push(format!("WAV seed {seed}: {outcome:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
