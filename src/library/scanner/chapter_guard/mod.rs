//! Keeps a file whose ID3v2 chapter frames nest away from the tag parsers.
//!
//! An ID3v2 `CHAP` or `CTOC` frame may hold sub-frames, and both lofty and the
//! `id3` crate read those sub-frames with the same function that reads the tag's
//! frames. A chapter nested in a chapter therefore recurses once per level, with
//! no depth limit and no option for one (CWE-674). A crafted tag of a few
//! kilobytes overflows the thread's stack; the process aborts, and no
//! `catch_unwind` can stop it. Worse, a file left in a scanned folder would
//! overflow the next scan too, so the server would never finish booting again.
//!
//! The server reaches two parsers: lofty, through [`lofty::read_from_path`] on
//! every audio file, and the `id3` crate, through the DSD metadata reader on a
//! `DSF`/`DFF` file. symphonia's reader is not reached by any server path. So
//! this module is called before those two, over the bytes each one would read:
//! [`check_file`] for lofty's path, [`check_dsd`] for the ID3 blob inside a DSD
//! container.
//!
//! The check counts chapter-frame *names* rather than following the structure,
//! because a walk that imitates one parser can be led elsewhere by the bytes:
//! lofty stops inside a frame at a zero or partway through a URL and reads what
//! is left as the next frame. Counting leans only on what no parser can avoid:
//! a parser descends into a frame whose id it read, and a nested frame lies
//! inside the length its parent declares, so two frames of one row never
//! contain each other. A chain of chapters therefore holds at most one frame
//! per row, and every deeper link is the name `CHAP` or `CTOC` lying somewhere
//! else. Counting those names bounds the depth whatever a parser makes of the
//! bytes around them. A name inside the body of a frame every parser reads to
//! its end does not count, so a chapter titled "CHAPTER 12" stays a title.

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

mod frames;
pub(crate) use frames::check_tag;

#[cfg(test)]
mod tests;

/// Chapter frame names tolerated before a file is refused, so at most this many
/// levels under the first. Nothing real nests them: a table of contents names
/// its chapters, it does not wrap them. Three leaves room for an honest
/// `CTOC`-then-`CHAP` pairing while staying far below the overflow threshold of
/// the smallest stack a tag is read on.
const ALLOWED_UNEXPLAINED_CHAPTERS: usize = 3;

/// Tables of contents in one tag. A real tag has one; a long audiobook split
/// into parts has a few dozen. Past this the entries recurse on their own in
/// some readers, with no nesting in the bytes at all.
const MAX_TABLES_OF_CONTENTS: usize = 64;

const TAG_HEADER_LEN: usize = 10;
const FRAME_HEADER_LEN: usize = 10;

/// How far past the start of a file, or the end of one tag, another is looked
/// for. lofty skips junk before the first audio frame and reads a tag that
/// follows another behind a ten-byte footer.
const HEAD_WINDOW: u64 = 4096;

/// What lofty can lose past a tag's declared end when it carries an extended
/// header: an unsynchronised 2.3 one is read through a stream that buffers 8 KiB
/// and is then dropped, so its frames start and end that much later.
const EXTENDED_HEADER_SLACK: u64 = 16 * 1024;

/// How far behind a chapter name the flags are looked through for the `0xff` of
/// unsynchronisation, past which a compressed frame's zlib stream is not where
/// its flags say.
const COMPRESSED_BODY_PROBE: usize = 16;

/// How much of the file is read at a time while looking for a tag.
const HEAD_CHUNK: usize = 64 * 1024;

/// Long enough that a signature cut by the end of a chunk is whole in the next.
const SIGNATURE_OVERLAP: usize = 8;

/// Chunks walked in a WAV or an AIFF. A real one has a handful.
const MAX_CHUNKS: usize = 65_536;

/// How many bytes the whole check may read from one file before refusing it. A
/// real file's tags and metadata chunks are far smaller; only a crafted one —
/// chunks declaring huge sizes, or a body repeating `ID3 ` to make the walk
/// re-read the file — reaches this, and refusing it is the safe answer. The
/// audio itself is never read: `data` and `SSND` chunks are skipped.
const MAX_SCAN_BYTES: u64 = 64 * 1024 * 1024;

/// Chunk positions a WAV or AIFF walk may hold at once. A real container has a
/// handful; a body stuffed with `ID3 ` markers would otherwise queue one per
/// occurrence, so the queue is bounded like the work it drives.
const MAX_PENDING_CHUNKS: usize = MAX_CHUNKS;

/// The file was refused: its ID3v2 chapter frames nest, or one hides its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NestedChapters;

impl fmt::Display for NestedChapters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the ID3v2 tag nests chapter frames, refusing to read it")
    }
}

impl std::error::Error for NestedChapters {}

/// Subtracts the bytes just read from the budget, refusing the file when it runs
/// out. A file that makes the check read more than [`MAX_SCAN_BYTES`] is
/// pathological, and refusing it is safer than reading on.
fn spend(budget: &mut u64, bytes: u64) -> Result<(), NestedChapters> {
    *budget = budget.checked_sub(bytes).ok_or(NestedChapters)?;
    Ok(())
}

/// Whether the file at `path` is safe to hand to lofty. A file that cannot be
/// opened or read passes: the parser is about to meet the same error and report
/// it, and refusing here would turn an unreadable file into an absent one.
pub fn check_file(path: &Path) -> Result<(), NestedChapters> {
    match std::fs::File::open(path) {
        Ok(mut file) => check_reader(&mut file),
        Err(_) => Ok(()),
    }
}

/// [`check_file`] over a reader, from where it stands, which is where it is left.
pub fn check_reader<R: Read + Seek>(reader: &mut R) -> Result<(), NestedChapters> {
    let Ok(start) = reader.stream_position() else {
        return Ok(());
    };
    let mut budget = MAX_SCAN_BYTES;
    let verdict = check_tags_from(reader, start, &mut budget);
    if reader.seek(SeekFrom::Start(start)).is_err() {
        tracing::warn!("could not rewind after looking at a file's tags");
    }
    verdict
}

/// Whether the DSD container at `path` carries an ID3 blob that nests chapters.
///
/// The `id3` crate is reached only here on the server, through the DSD metadata
/// reader. `DSF` keeps its tag at the offset named at byte twenty of the header;
/// `DFF` keeps it in an `ID3 ` chunk of the `FRM8` form. Neither lies where the
/// head scan of [`check_file`] would find it, so the blob is located the same
/// way the reader does and checked on its own. A container this cannot make
/// sense of passes: the reader will meet the same bytes and read nothing from
/// them.
pub fn check_dsd(path: &Path) -> Result<(), NestedChapters> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Ok(());
    };
    let mut magic = [0_u8; 4];
    if file.read_exact(&mut magic).is_err() {
        return Ok(());
    }
    let mut budget = MAX_SCAN_BYTES;
    match &magic {
        b"DSD " => check_dsf_blob(&mut file, &mut budget),
        b"FRM8" => check_dff_blobs(&mut file, &mut budget),
        _ => Ok(()),
    }
}

/// The ID3 blob a `DSF` names at byte twenty, read to the file's end and checked.
fn check_dsf_blob<R: Read + Seek>(file: &mut R, budget: &mut u64) -> Result<(), NestedChapters> {
    let mut offset = [0_u8; 8];
    if file.seek(SeekFrom::Start(20)).is_err() || file.read_exact(&mut offset).is_err() {
        return Ok(());
    }
    let offset = u64::from_le_bytes(offset);
    if offset == 0 {
        return Ok(());
    }
    check_tag_at(file, offset, budget)
}

/// Every `ID3 ` chunk of a `DFF` `FRM8` form, checked where it sits.
fn check_dff_blobs<R: Read + Seek>(file: &mut R, budget: &mut u64) -> Result<(), NestedChapters> {
    let Ok(end) = file.seek(SeekFrom::End(0)) else {
        return Ok(());
    };
    // `FRM8` <8-byte size> "DSD " then the form's chunks.
    let mut at = 16_u64;
    let mut chunks = 0;
    while at + 12 <= end {
        chunks += 1;
        if chunks > MAX_CHUNKS {
            // Refused, not passed: the DSD reader walks on past this point, so a
            // tag beyond the chunk the walk gave up at would reach the `id3`
            // crate unchecked. A real `DFF` has a handful of chunks.
            return Err(NestedChapters);
        }
        let mut header = [0_u8; 12];
        if file.seek(SeekFrom::Start(at)).is_err() || file.read_exact(&mut header).is_err() {
            return Ok(());
        }
        let size = u64::from_be_bytes([
            header[4], header[5], header[6], header[7], header[8], header[9], header[10],
            header[11],
        ]);
        if &header[..4] == b"ID3 " {
            check_tag_at(file, at + 12, budget)?;
        }
        // IFF word alignment: an odd length is followed by one padding byte.
        let Some(next) = at.checked_add(12).and_then(|v| v.checked_add(size)) else {
            return Ok(());
        };
        at = if size % 2 == 1 { next + 1 } else { next };
    }
    Ok(())
}

/// What [`read_declared`] found at a position.
enum DeclaredTag {
    /// A tag, read as far as it declares (up to the cap).
    Tag(Vec<u8>),
    /// Not a tag a parser reads: wrong magic, or a version none of them take.
    /// The walk moves on, as the parser would.
    NotTag,
    /// The read failed partway. The parser that runs next may read past the
    /// failure, so the file is refused rather than passed.
    ReadFailed,
}

/// Reads the tag standing at `offset` as far as it declares, and checks it.
fn check_tag_at<R: Read + Seek>(
    file: &mut R,
    offset: u64,
    budget: &mut u64,
) -> Result<(), NestedChapters> {
    match read_declared(file, offset, budget.saturating_add(1)) {
        DeclaredTag::Tag(tag) => {
            spend(budget, tag.len() as u64)?;
            check_tag(&tag)
        }
        DeclaredTag::NotTag => Ok(()),
        DeclaredTag::ReadFailed => Err(NestedChapters),
    }
}

/// Every tag a parser could find from `start`: behind zeros, behind junk, behind
/// another tag or an APE tag, and in the chunks of a WAV or an AIFF.
fn check_tags_from<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    budget: &mut u64,
) -> Result<(), NestedChapters> {
    let mut buffer = vec![0_u8; HEAD_CHUNK];
    let mut at = start;
    let mut scan_end = start.saturating_add(HEAD_WINDOW);
    let mut leading_zeros = true;
    let mut behind_ape = false;
    while at < scan_end {
        // A read that fails midway is refused, not taken for the end of the
        // file: the parser that runs next may read the file to its end and meet
        // the chapter frames this walk never reached.
        let Ok(read) = read_at(reader, at, &mut buffer) else {
            return Err(NestedChapters);
        };
        spend(budget, read as u64)?;
        let chunk = &buffer[..read];
        if chunk.len() <= SIGNATURE_OVERLAP {
            return Ok(());
        }
        if leading_zeros {
            match chunk.iter().position(|byte| *byte != 0) {
                Some(first) => {
                    leading_zeros = false;
                    at += first as u64;
                }
                None => at += chunk.len() as u64,
            }
            scan_end = at.saturating_add(HEAD_WINDOW);
            continue;
        }
        let Some((offset, signature)) = find_signature(chunk) else {
            at += (chunk.len() - SIGNATURE_OVERLAP) as u64;
            continue;
        };
        let found = at + offset as u64;
        match signature {
            Signature::Id3 => {
                let tag = match read_declared(reader, found, budget.saturating_add(1)) {
                    DeclaredTag::Tag(tag) => Some(tag),
                    DeclaredTag::NotTag => None,
                    // An I/O error reading the tag is refused, not swallowed as
                    // "no tag here": the parser that runs next may read it whole.
                    DeclaredTag::ReadFailed => return Err(NestedChapters),
                };
                if let Some(tag) = &tag {
                    spend(budget, tag.len() as u64)?;
                    check_tag(tag)?;
                }
                at = match tag {
                    Some(tag) if !behind_ape => {
                        let end = found + tag_span(&tag);
                        scan_end = end.saturating_add(HEAD_WINDOW);
                        end
                    }
                    _ => found + 1,
                };
            }
            // lofty walks an APE tag's items, which is not what its header
            // declares, and reads on behind them. A tag there is rare enough to
            // read the whole file: from here on nothing is jumped over, since an
            // item may hold those three letters.
            Signature::Ape => {
                behind_ape = true;
                scan_end = u64::MAX;
                at = found + 1;
            }
            Signature::Riff => {
                check_chunks(reader, found, false, budget)?;
                at = found + 1;
            }
            Signature::Form => {
                check_chunks(reader, found, true, budget)?;
                at = found + 1;
            }
        }
    }
    Ok(())
}

/// Fills `buffer` from `at`, returning how many bytes were read. Stops at the
/// end of the file, retries an interrupted read, and surfaces any other error.
fn read_at<R: Read + Seek>(reader: &mut R, at: u64, buffer: &mut [u8]) -> io::Result<usize> {
    reader.seek(SeekFrom::Start(at))?;
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            // An interrupted read is retried, as the parsers' own reads are:
            // treating it as the end of the file would stop the walk short of
            // frames the parser goes on to read.
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

#[derive(Clone, Copy)]
/// A container or tag marker the walk stops on: an ID3v2 tag, an APE tag, or the
/// `RIFF`/`FORM` header of a WAV or AIFF.
enum Signature {
    Id3,
    Ape,
    Riff,
    Form,
}

/// The first [`Signature`] in `chunk`, with its offset, or `None` if none is in
/// the part scanned.
fn find_signature(chunk: &[u8]) -> Option<(usize, Signature)> {
    chunk
        .windows(SIGNATURE_OVERLAP)
        .enumerate()
        .find_map(|(offset, window)| {
            let signature = match window {
                [b'I', b'D', b'3', ..] => Signature::Id3,
                [b'A', b'P', b'E', b'T', b'A', b'G', b'E', b'X'] => Signature::Ape,
                [b'R', b'I', b'F', b'F', ..] => Signature::Riff,
                [b'F', b'O', b'R', b'M', ..] => Signature::Form,
                _ => return None,
            };
            Some((offset, signature))
        })
}

/// The ten bytes at `at` and as many behind them as they declare, as far as
/// there are any, and never more than `cap` bytes of body — so a huge declared
/// size cannot allocate past the caller's budget. A tag with an extended header
/// gets [`EXTENDED_HEADER_SLACK`] more.
///
/// An I/O failure is reported as [`DeclaredTag::ReadFailed`], distinct from the
/// bytes simply not being a tag ([`DeclaredTag::NotTag`]): a read that failed
/// partway must refuse the file, since the parser that runs next may read it
/// whole, whereas bytes that are no tag are passed over.
fn read_declared<R: Read + Seek>(reader: &mut R, at: u64, cap: u64) -> DeclaredTag {
    let mut header = [0_u8; TAG_HEADER_LEN];
    if reader.seek(SeekFrom::Start(at)).is_err() {
        return DeclaredTag::ReadFailed;
    }
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        // Fewer than ten bytes here is the end of the file, not a failure: there
        // is no tag to read, and no parser would find one either.
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            return DeclaredTag::NotTag;
        }
        Err(_) => return DeclaredTag::ReadFailed,
    }
    // Validated before the body is read, not after. A signature whose version
    // no parser reads would otherwise have its declared body read and allocated
    // and then thrown away, uncharged — which repeated signatures could turn
    // into repeated large reads the budget never sees.
    if &header[..3] != b"ID3" || !(2..=4).contains(&header[3]) {
        return DeclaredTag::NotTag;
    }
    let slack = if header[5] & 0x40 != 0 {
        EXTENDED_HEADER_SLACK
    } else {
        0
    };
    let mut bytes = header.to_vec();
    match reader
        .by_ref()
        .take((u64::from(synchsafe(&header[6..10])) + slack).min(cap))
        .read_to_end(&mut bytes)
    {
        Ok(_) => DeclaredTag::Tag(bytes),
        Err(_) => DeclaredTag::ReadFailed,
    }
}

/// From the start of a tag to what follows it. Not past a footer: lofty reads on
/// right behind the frames of a repeated tag, and a footer is no signature to
/// stop at.
fn tag_span(tag: &[u8]) -> u64 {
    (TAG_HEADER_LEN as u64) + u64::from(synchsafe(&tag[6..10]))
}

/// The `ID3 ` chunks of the WAV or AIFF at `start`. A chunk of odd length is
/// followed by a padding byte that lofty and symphonia disagree about when it is
/// not a zero, so both continuations are walked. The name of a chunk is looked
/// for inside every chunk but the audio, since a reader that lost its place in a
/// `COMM` looks for the next chunk wherever it stopped.
fn check_chunks<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    big_endian: bool,
    budget: &mut u64,
) -> Result<(), NestedChapters> {
    let Ok(len) = reader.seek(SeekFrom::End(0)) else {
        return Ok(());
    };
    let mut pending = vec![start + 12];
    let mut seen = std::collections::HashSet::new();
    while let Some(at) = pending.pop() {
        if !seen.insert(at) {
            continue;
        }
        if seen.len() > MAX_CHUNKS {
            return Err(NestedChapters);
        }
        let mut header = [0_u8; 8];
        if reader.seek(SeekFrom::Start(at)).is_err() || reader.read_exact(&mut header).is_err() {
            continue;
        }
        spend(budget, 8)?;
        // Silence behind a `data` chunk whose length was never written is a
        // chunk with no name and no length, every eight bytes. Not a reason to
        // give up on the rest.
        if header == [0; 8] {
            if let Some(next) = past_zero_chunks(reader, at, budget)? {
                pending.push(next);
            }
            continue;
        }
        let raw = [header[4], header[5], header[6], header[7]];
        let size = u64::from(if big_endian {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        });
        if !matches!(&header[..4], b"SSND" | b"data") {
            let mut body = Vec::new();
            // Bounded to the budget before the read, so the allocation itself
            // cannot exceed it: a chunk declaring a huge size reads one byte
            // past the budget at most, and `spend` then refuses the file.
            let limit = size.min(budget.saturating_add(1));
            if reader.by_ref().take(limit).read_to_end(&mut body).is_ok() {
                spend(budget, body.len() as u64)?;
                if matches!(&header[..4], b"ID3 " | b"id3 ") {
                    check_tag(&body)?;
                } else {
                    for (offset, name) in body.windows(4).enumerate() {
                        if matches!(name, b"ID3 " | b"id3 ") {
                            if pending.len() >= MAX_PENDING_CHUNKS {
                                return Err(NestedChapters);
                            }
                            pending.push(at + 8 + offset as u64);
                        }
                    }
                }
            }
        }
        let next = at + 8 + size;
        if next < len {
            if pending.len() >= MAX_PENDING_CHUNKS {
                return Err(NestedChapters);
            }
            pending.push(next);
            if size % 2 == 1 {
                pending.push(next + 1);
            }
        }
    }
    Ok(())
}

/// Where the run of empty chunks at `at` ends, on the eight-byte steps it is
/// made of. Reads against the shared budget, so a file that is zeros to the end
/// is refused rather than scanned in full.
fn past_zero_chunks<R: Read + Seek>(
    reader: &mut R,
    at: u64,
    budget: &mut u64,
) -> Result<Option<u64>, NestedChapters> {
    let mut buffer = vec![0_u8; HEAD_CHUNK];
    let mut from = at;
    loop {
        // A read that fails midway is refused, like the head walk above, rather
        // than taken for the end of the run.
        let Ok(read) = read_at(reader, from, &mut buffer) else {
            return Err(NestedChapters);
        };
        spend(budget, read as u64)?;
        if read == 0 {
            return Ok(None);
        }
        if let Some(first) = buffer[..read].iter().position(|byte| *byte != 0) {
            return Ok(Some(at + (from + first as u64 - at) / 8 * 8));
        }
        from += read as u64;
    }
}

/// Seven bits to the byte, the top one dropped, as lofty and the `id3` crate
/// read a synchsafe length.
fn synchsafe(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0_u32, |value, byte| (value << 7) | u32::from(byte & 0x7f))
}
