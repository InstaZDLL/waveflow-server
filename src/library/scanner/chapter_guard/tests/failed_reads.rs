//! The reads the guard cannot go on without, when they fail: the headers that
//! locate the tag of a DSD file, and the question of where a reader stands.
//! A failure there is refused like a failed read anywhere else, while a file
//! that merely ends early, or a read that is merely interrupted, is not.

use super::*;

/// The short reads that locate the tag of a DSD file, which lies
/// where no head scan finds it: the four bytes naming the container, the
/// offset a `DSF` gives its tag, and each chunk header of a `DFF`. A read of
/// one of those that fails is a one-off here, so the DSD reader's own read of
/// the same bytes would succeed and hand the tag to the `id3` crate: a guard
/// that took the failure for "no tag here" passes exactly that file. The
/// control reads each file whole and refuses it for the nesting.
#[test]
fn a_dsd_header_whose_read_fails_is_refused() {
    let hostile = tag(&nested(4, &[b"CHAP"], 64));
    // In the `DFF`: `FRM8`, its size and `DSD ` are sixteen bytes; the `DSD `
    // chunk that follows is a twelve-byte header and 256 bytes.
    let mut failures = Vec::new();
    for (case, file, fail_at) in [
        ("the name of a DSF", dsf(&hostile), 0),
        ("the offset a DSF gives its tag", dsf(&hostile), 20),
        ("the name of a DFF", dff(&hostile), 0),
        ("the first chunk header of a DFF", dff(&hostile), 16),
        (
            "the header of a DFF's tag chunk",
            dff(&hostile),
            16 + 12 + 256,
        ),
    ] {
        let control = check_dsd_reader(&mut Cursor::new(file.clone()));
        if control != Err(Refusal::Nested) {
            failures.push(format!("the control, {case}: {control:?}"));
        }
        let mut reader = FlakyReader::new(file);
        reader.fail_at = Some(fail_at);
        let verdict = check_dsd_reader(&mut reader);
        if verdict != Err(Refusal::ReadFailed) {
            failures.push(format!("{case}: {verdict:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A DSD file that ends before the header the walk needs is not a failed read:
/// the DSD reader meets the same end and reads no tag, so the file passes
/// rather than being refused for a failure that did not happen.
#[test]
fn a_dsd_file_cut_short_of_its_header_passes() {
    let hostile = tag(&nested(4, &[b"CHAP"], 64));
    for (case, file) in [
        ("two bytes", b"DS".to_vec()),
        (
            "a DSF cut before its tag offset",
            dsf(&hostile)[..24].to_vec(),
        ),
        ("the name of a DFF alone", b"FRM8".to_vec()),
        (
            "a DFF cut in its first chunk header",
            dff(&hostile)[..20].to_vec(),
        ),
    ] {
        assert_eq!(check_dsd_reader(&mut Cursor::new(file)), Ok(()), "{case}");
    }
}

/// An interrupted read of those headers is retried, as everywhere else: the
/// nested tag behind it is still reached and refused, and an honest file read
/// through the same hiccup passes.
#[test]
fn an_interrupted_read_of_a_dsd_header_is_retried() {
    let hostile = tag(&nested(4, &[b"CHAP"], 64));
    let honest = tag(&title(4, "a plain title, no chapters"));
    for (case, build, interrupt_at) in [
        ("the name of a DSF", dsf as fn(&[u8]) -> Vec<u8>, 0),
        ("the offset a DSF gives its tag", dsf, 20),
        ("the name of a DFF", dff, 0),
        ("the first chunk header of a DFF", dff, 16),
    ] {
        let mut reader = FlakyReader::new(build(&hostile));
        reader.interrupt_at = Some(interrupt_at);
        assert_eq!(
            check_dsd_reader(&mut reader),
            Err(Refusal::Nested),
            "{case}"
        );
        let mut reader = FlakyReader::new(build(&honest));
        reader.interrupt_at = Some(interrupt_at);
        assert_eq!(check_dsd_reader(&mut reader), Ok(()), "{case}, honest");
    }
}

/// A reader that fails the first time it is asked where it stands, and answers
/// every seek after that.
struct NoPositionAtFirst {
    inner: Cursor<Vec<u8>>,
    asked: bool,
}

impl Read for NoPositionAtFirst {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for NoPositionAtFirst {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if !self.asked && pos == SeekFrom::Current(0) {
            self.asked = true;
            return Err(io::Error::other("cannot tell where it stands"));
        }
        self.inner.seek(pos)
    }
}

/// A check that cannot learn where its reader stands has checked nothing, and
/// the parser that runs next opens the file for itself and reads it whole: the
/// file is refused rather than passed unread. The control is refused for the
/// nesting.
#[test]
fn a_reader_that_cannot_tell_where_it_stands_is_refused() {
    let bytes = mp3(&tag(&nested(4, &[b"CHAP"], 64)));
    assert_eq!(
        check_file_bytes(&bytes),
        Err(Refusal::Nested),
        "the control"
    );
    let mut reader = NoPositionAtFirst {
        inner: Cursor::new(bytes),
        asked: false,
    };
    assert_eq!(check_reader(&mut reader), Err(Refusal::ReadFailed));
}
