use super::sheet_of;
use crate::lrclib::FoundLyrics;

#[test]
fn timings_win_and_a_sheet_without_words_is_none() {
    let both = sheet_of(FoundLyrics {
        synced: Some("[00:01.00]one\n[00:02.00]two".into()),
        plain: Some("one\ntwo".into()),
    })
    .unwrap();
    assert!(both.0);
    assert_eq!(both.1[1].start, Some(2_000));
    let plain = sheet_of(FoundLyrics {
        synced: None,
        plain: Some("one".into()),
    })
    .unwrap();
    assert!(!plain.0);
    assert!(sheet_of(FoundLyrics {
        synced: Some("[00:01.00]".into()),
        plain: None,
    })
    .is_none());
}
