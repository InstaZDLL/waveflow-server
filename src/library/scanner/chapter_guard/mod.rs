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
            return Ok(());
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

/// Reads the tag standing at `offset` as far as it declares, and checks it.
fn check_tag_at<R: Read + Seek>(
    file: &mut R,
    offset: u64,
    budget: &mut u64,
) -> Result<(), NestedChapters> {
    match read_declared(file, offset, budget.saturating_add(1)) {
        Ok(tag) => {
            spend(budget, tag.len() as u64)?;
            check_tag(&tag)
        }
        Err(_) => Ok(()),
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
        let Ok(read) = read_at(reader, at, &mut buffer) else {
            return Ok(());
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
                let tag = read_tag(reader, found, budget.saturating_add(1));
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

fn read_at<R: Read + Seek>(reader: &mut R, at: u64, buffer: &mut [u8]) -> io::Result<usize> {
    reader.seek(SeekFrom::Start(at))?;
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

#[derive(Clone, Copy)]
enum Signature {
    Id3,
    Ape,
    Riff,
    Form,
}

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

/// The tag at `at`, header and all, as far as the bytes go. `None` when those
/// three letters start no tag a parser would take.
fn read_tag<R: Read + Seek>(reader: &mut R, at: u64, cap: u64) -> Option<Vec<u8>> {
    // `read_declared` already rejects anything that is not an ID3v2 tag a parser
    // reads, so a successful read is one to check.
    read_declared(reader, at, cap).ok()
}

/// The ten bytes at `at` and as many behind them as they declare, as far as
/// there are any, and never more than `cap` bytes of body — so a huge declared
/// size cannot allocate past the caller's budget. A tag with an extended header
/// gets [`EXTENDED_HEADER_SLACK`] more.
fn read_declared<R: Read + Seek>(reader: &mut R, at: u64, cap: u64) -> io::Result<Vec<u8>> {
    let mut header = [0_u8; TAG_HEADER_LEN];
    reader.seek(SeekFrom::Start(at))?;
    reader.read_exact(&mut header)?;
    // Validated before the body is read, not after. A signature whose version
    // no parser reads would otherwise have its declared body read and allocated
    // and then thrown away by the caller, uncharged — which repeated signatures
    // could turn into repeated large reads the budget never sees.
    if &header[..3] != b"ID3" || !(2..=4).contains(&header[3]) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not an ID3v2 tag a parser reads",
        ));
    }
    let slack = if header[5] & 0x40 != 0 {
        EXTENDED_HEADER_SLACK
    } else {
        0
    };
    let mut bytes = header.to_vec();
    reader
        .by_ref()
        .take((u64::from(synchsafe(&header[6..10])) + slack).min(cap))
        .read_to_end(&mut bytes)?;
    Ok(bytes)
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
        let Ok(read) = read_at(reader, from, &mut buffer) else {
            return Ok(None);
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

/// One whole tag, from its `ID3` on, or bytes that are not one and pass.
pub fn check_tag(tag: &[u8]) -> Result<(), NestedChapters> {
    let Some(header) = tag.get(..TAG_HEADER_LEN) else {
        return Ok(());
    };
    let major = header[3];
    // 2.2 has three-letter ids and no chapter frame; nothing reads a later one.
    if &header[..3] != b"ID3" || !(major == 3 || major == 4) {
        return Ok(());
    }
    let flags = header[5];
    let unsynchronised = flags & 0x80 != 0;
    let extended = flags & 0x40 != 0;
    let end = if extended {
        tag.len()
    } else {
        TAG_HEADER_LEN.saturating_add(synchsafe(&header[6..10]) as usize)
    };
    let body = &tag[TAG_HEADER_LEN..end.min(tag.len())];
    let resynchronised;
    // In 2.3 the unsynchronisation covers the whole tag, frame headers too.
    let frames = if unsynchronised && major == 3 {
        resynchronised = resynchronise(body);
        resynchronised.as_slice()
    } else {
        body
    };
    let mut audit = Audit {
        major,
        unexplained: 0,
        tables: 0,
        // An extended header's length is one more thing to disagree on, and in
        // 2.4 this flag means a frame's bytes are not what the row below sees.
        in_step: !extended && !(unsynchronised && major == 4),
    };
    audit.walk(frames, Level::Top)
}

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Top,
    InsideChapter,
}

struct Audit {
    major: u8,
    unexplained: usize,
    tables: usize,
    /// Every frame so far was read to its end by every parser, so all of them
    /// stand where this walk does.
    in_step: bool,
}

impl Audit {
    fn walk(&mut self, frames: &[u8], level: Level) -> Result<(), NestedChapters> {
        let mut at = 0;
        while let Some(header) = frames.get(at..at + FRAME_HEADER_LEN) {
            let raw = &header[4..8];
            // The `id3` crate keeps the top bit of a 2.4 length where lofty
            // drops it, so from a length like this on they read different rows.
            if !is_frame_id(&header[..4]) || (self.major == 4 && raw.iter().any(|b| b & 0x80 != 0))
            {
                break;
            }
            let length = if self.major == 4 {
                synchsafe(raw) as usize
            } else {
                u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize
            };
            let flags = u16::from_be_bytes([header[8], header[9]]);
            let body_start = at + FRAME_HEADER_LEN;
            let body_end = body_start.saturating_add(length);
            let Some(body) = frames.get(body_start..body_end) else {
                break;
            };
            let id = &header[..4];
            if is_chapter(id) {
                if flags & self.compression_flag() != 0 {
                    return Err(NestedChapters);
                }
                if level == Level::Top && id == b"CTOC" {
                    self.tables += 1;
                    if self.tables > MAX_TABLES_OF_CONTENTS {
                        return Err(NestedChapters);
                    }
                }
                match (level, self.in_step && flags == 0, children_of(id, body)) {
                    (Level::Top, true, Some(children)) => {
                        self.walk(children, Level::InsideChapter)?;
                        // A name cut by the end of what was just walked alone.
                        if !self.in_step {
                            let from = body_end.saturating_sub(FRAME_HEADER_LEN - 1);
                            self.count_around(frames, from.max(body_start), body_end)?;
                        }
                    }
                    (Level::Top, _, _) => {
                        self.in_step = false;
                        self.count_around(frames, at + 1, body_end)?;
                    }
                    (Level::InsideChapter, _, _) => {
                        self.in_step = false;
                        self.count(1)?;
                        self.count_around(frames, at + 1, body_end)?;
                    }
                }
            } else if !(self.in_step && flags == 0 && read_to_its_end(id, body)) {
                self.in_step = false;
                self.count_around(frames, at + 1, body_end)?;
            }
            at = body_end;
        }
        // What stops a list inside a chapter leaves the parser partway through it.
        if level == Level::InsideChapter && at < frames.len() {
            self.in_step = false;
        }
        self.count_names(frames, at, frames.len())
    }

    fn count(&mut self, names: usize) -> Result<(), NestedChapters> {
        self.unexplained += names;
        if self.unexplained > ALLOWED_UNEXPLAINED_CHAPTERS {
            Err(NestedChapters)
        } else {
            Ok(())
        }
    }

    /// Counts the names from `from` up to the current frame's end, and in the two
    /// bytes just past the next frame's id — but never that id at `body_end`
    /// itself, which the main walk reaches next and counts on its own terms. A
    /// range that reached into it would count a legitimate `CHAP`/`CTOC`
    /// following a frame this walk stopped trusting (a `TIPL`, say) as a stray.
    fn count_around(
        &mut self,
        frames: &[u8],
        from: usize,
        body_end: usize,
    ) -> Result<(), NestedChapters> {
        self.count_names(frames, from, body_end)?;
        self.count_names(frames, body_end + 1, body_end + 3)
    }

    /// The names starting between `from` and `to` in `frames`, wherever they
    /// end. A parser that lost the row reads straight across the frames it is
    /// made of, so a name may start in one and end in the next, its flags
    /// further on still.
    fn count_names(&mut self, frames: &[u8], from: usize, to: usize) -> Result<(), NestedChapters> {
        let to = to.min(frames.len());
        let Some(starts) = frames.get(from..to) else {
            return Ok(());
        };
        let names = starts
            .iter()
            .enumerate()
            .filter(|(_, byte)| matches!(byte, b'C'))
            .map(|(offset, _)| from + offset);
        for start in names {
            if !frames.get(start..start + 4).is_some_and(is_chapter) {
                continue;
            }
            if let Some(header) = frames.get(start + 4..start + FRAME_HEADER_LEN) {
                // In an unsynchronised frame, a parser reads this header with
                // the zero after a 0xff taken out, so its flags are not here.
                if header.contains(&0xff) {
                    return Err(NestedChapters);
                }
                let flags = u16::from_be_bytes([header[4], header[5]]);
                let after = &frames[start + FRAME_HEADER_LEN..];
                let probe = &after[..after.len().min(COMPRESSED_BODY_PROBE)];
                if flags & self.compression_flag() != 0
                    && (probe.contains(&0xff) || inflatable_after(after))
                {
                    return Err(NestedChapters);
                }
            }
            self.count(1)?;
        }
        Ok(())
    }

    /// lofty and `id3` inflate a compressed frame, and what comes out is not in
    /// these bytes to be counted. Nothing compresses a chapter.
    fn compression_flag(&self) -> u16 {
        if self.major == 4 {
            0x0008
        } else {
            0x0080
        }
    }
}

/// Whether a zlib stream starts where a compressed frame's body can, past the
/// group, encryption and length fields its flags may add. "CHAPTER 19" reads as
/// a compressed frame by its flags, and these two bytes tell it from one.
fn inflatable_after(body: &[u8]) -> bool {
    (0..=6).any(|skip| match body.get(skip..skip + 2) {
        Some([cmf, flg]) => cmf & 0x0f == 8 && (u16::from(*cmf) * 256 + u16::from(*flg)) % 31 == 0,
        _ => false,
    })
}

fn is_chapter(id: &[u8]) -> bool {
    id == b"CHAP" || id == b"CTOC"
}

/// Four capitals or digits: anything else is padding, or no longer a frame.
fn is_frame_id(id: &[u8]) -> bool {
    id.iter()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

/// Whether no parser can leave this frame partway, read off lofty's
/// `parse_content`: it stops a URL at its first zero and a comment's UTF-16 text
/// at its first terminator, and the frames with a layout of their own have no
/// free-length field at the end, the key-value lists included.
fn read_to_its_end(id: &[u8], body: &[u8]) -> bool {
    match id {
        b"TIPL" | b"TMCL" | b"IPLS" | b"RVA2" | b"OWNE" | b"ETCO" => false,
        b"COMM" | b"USLT" => comment_read_to_its_end(body),
        b"WXXX" => true,
        _ if id[0] == b'W' => match body.iter().position(|byte| *byte == 0) {
            Some(zero) => zero + 1 == body.len(),
            None => true,
        },
        _ => true,
    }
}

/// Encoding, language, a terminated description, then the text: read to its end
/// in every encoding but the first UTF-16, where it stops at a terminator.
fn comment_read_to_its_end(body: &[u8]) -> bool {
    if body.first() != Some(&1) {
        return true;
    }
    let units = |from: usize| {
        let slice = body.get(from..).unwrap_or_default();
        (0..slice.len() / 2)
            .find(|pair| slice[pair * 2] == 0 && slice[pair * 2 + 1] == 0)
            .map(|pair| from + pair * 2 + 2)
    };
    match units(4) {
        Some(text) => units(text).is_none_or(|end| end == body.len()),
        None => true,
    }
}

/// The frames inside a chapter or a table of contents, past its own fields.
/// `None` when those fields do not fit, and the body is then only bytes.
fn children_of<'a>(id: &[u8], body: &'a [u8]) -> Option<&'a [u8]> {
    let id_end = body.iter().position(|byte| *byte == 0)? + 1;
    let rest = body.get(id_end..)?;
    if id == b"CHAP" {
        return rest.get(16..);
    }
    let (entries, mut rest) = (*rest.get(1)?, rest.get(2..)?);
    for _ in 0..entries {
        let end = rest.iter().position(|byte| *byte == 0)? + 1;
        rest = rest.get(end..)?;
    }
    Some(rest)
}

/// Seven bits to the byte, the top one dropped, as lofty and the `id3` crate
/// read a synchsafe length.
fn synchsafe(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0_u32, |value, byte| (value << 7) | u32::from(byte & 0x7f))
}

fn resynchronise(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut after_ff = false;
    for byte in bytes {
        if !(after_ff && *byte == 0) {
            out.push(*byte);
        }
        after_ff = *byte == 0xff;
    }
    out
}
