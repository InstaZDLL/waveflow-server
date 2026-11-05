//! Reading one ID3v2 tag's frames and deciding whether its chapter frames nest.
//!
//! The counting strategy and why it leans only on the frame names no parser can
//! avoid reading are explained on the module above. This file is that count:
//! [`check_tag`] over a tag's bytes, the [`FrameScan`] walk it drives, and the
//! frame helpers that tell one parser's reading of a frame from another's.

use super::*;

/// One whole tag, from its `ID3` on, or bytes that are not one and pass.
pub(crate) fn check_tag(tag: &[u8]) -> Result<(), Refusal> {
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
    let mut scan = FrameScan {
        major,
        unexplained: 0,
        tables: 0,
        // An extended header's length is one more thing to disagree on, and in
        // 2.4 this flag means a frame's bytes are not what the row below sees.
        in_step: !extended && !(unsynchronised && major == 4),
    };
    scan.walk(frames, Level::Top)
}

/// Where in the frame tree the walk is: the tag's top level, or inside a
/// chapter's own sub-frames.
#[derive(Clone, Copy, PartialEq)]
enum Level {
    Top,
    InsideChapter,
}

/// The running state of one pass over a tag's frames.
struct FrameScan {
    major: u8,
    unexplained: usize,
    tables: usize,
    /// Every frame so far was read to its end by every parser, so all of them
    /// stand where this walk does.
    in_step: bool,
}

impl FrameScan {
    /// Walks `frames` at `level`, counting the chapter names no parser can
    /// avoid reading and refusing once more than [`ALLOWED_UNEXPLAINED_CHAPTERS`]
    /// are seen.
    fn walk(&mut self, frames: &[u8], level: Level) -> Result<(), Refusal> {
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
                    return Err(Refusal::Nested);
                }
                if level == Level::Top && id == b"CTOC" {
                    self.tables += 1;
                    if self.tables > MAX_TABLES_OF_CONTENTS {
                        return Err(Refusal::Nested);
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

    /// Adds `names` to the unexplained count, refusing once it passes the limit.
    fn count(&mut self, names: usize) -> Result<(), Refusal> {
        self.unexplained += names;
        if self.unexplained > ALLOWED_UNEXPLAINED_CHAPTERS {
            Err(Refusal::Nested)
        } else {
            Ok(())
        }
    }

    /// Counts the names from `from` up to the current frame's end, and in the two
    /// bytes just past the next frame's id — but never that id at `body_end`
    /// itself, which the main walk reaches next and counts on its own terms. A
    /// range that reached into it would count a legitimate `CHAP`/`CTOC`
    /// following a frame this walk stopped trusting (a `TIPL`, say) as a stray.
    fn count_around(&mut self, frames: &[u8], from: usize, body_end: usize) -> Result<(), Refusal> {
        self.count_names(frames, from, body_end)?;
        self.count_names(frames, body_end + 1, body_end + 3)
    }

    /// The names starting between `from` and `to` in `frames`, wherever they
    /// end. A parser that lost the row reads straight across the frames it is
    /// made of, so a name may start in one and end in the next, its flags
    /// further on still.
    fn count_names(&mut self, frames: &[u8], from: usize, to: usize) -> Result<(), Refusal> {
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
                    return Err(Refusal::Nested);
                }
                let flags = u16::from_be_bytes([header[4], header[5]]);
                let after = &frames[start + FRAME_HEADER_LEN..];
                let probe = &after[..after.len().min(COMPRESSED_BODY_PROBE)];
                if flags & self.compression_flag() != 0
                    && (probe.contains(&0xff) || inflatable_after(after))
                {
                    return Err(Refusal::Nested);
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

/// Whether `id` names a chapter frame — a `CHAP` or a `CTOC`.
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

/// Reverses ID3v2 unsynchronisation: drops the zero inserted after each `0xff`.
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
