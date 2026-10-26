//! Unit tests of the walk's rules: exclusions, ignore files, scopes, and what
//! counts as missing.

use super::{built_in_exclusion, load_rules, Rules, Walked, IGNORE_FILE, MAX_IGNORE_BYTES};

/// RFC-021 decision 1: what is left out by name, below the root only, and
/// the one dot folder that is not.
#[test]
fn built_in_exclusions_spare_the_folder_uploads_land_in() {
    use std::ffi::OsStr;
    for name in [
        ".hidden",
        "._Track.flac",
        ".Trash-1000",
        "@eaDir",
        "#recycle",
        "$Recycle.Bin",
        "System Volume Information",
        "LOST+FOUND",
    ] {
        assert!(built_in_exclusion(OsStr::new(name), 1, true), "{name}");
    }
    assert!(!built_in_exclusion(OsStr::new("Album"), 1, true));
    assert!(!built_in_exclusion(OsStr::new("eaDir"), 1, true));
    // Received files: kept directly under the root, and only there.
    assert!(!built_in_exclusion(
        OsStr::new(".waveflow-managed"),
        1,
        true
    ));
    assert!(built_in_exclusion(OsStr::new(".waveflow-managed"), 2, true));
    assert!(built_in_exclusion(
        OsStr::new(".waveflow-managed"),
        1,
        false
    ));
}

/// RFC-021 decision 1: an empty file excludes its folder except at the
/// root, and a file the walk cannot use says why.
#[test]
fn ignore_files_are_read_as_the_rfc_says() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    assert!(matches!(load_rules(dir, false), Rules::None));

    std::fs::write(dir.join(IGNORE_FILE), " \n\t\n").unwrap();
    assert!(matches!(load_rules(dir, false), Rules::ExcludeAll));
    assert!(
        matches!(load_rules(dir, true), Rules::None),
        "an empty file at the root must exclude nothing"
    );

    std::fs::write(dir.join(IGNORE_FILE), "# scans\n*.wav\n!Keep.wav\n").unwrap();
    let Rules::Matcher(rules) = load_rules(dir, false) else {
        panic!("a valid file is a matcher");
    };
    assert!(rules.matched(dir.join("Drop.wav"), false).is_ignore());
    assert!(rules.matched(dir.join("Keep.wav"), false).is_whitelist());

    std::fs::write(dir.join(IGNORE_FILE), "[z-a]\n").unwrap();
    assert!(matches!(load_rules(dir, false), Rules::Unusable(_)));

    std::fs::write(
        dir.join(IGNORE_FILE),
        vec![b'#'; MAX_IGNORE_BYTES as usize + 1],
    )
    .unwrap();
    assert!(matches!(load_rules(dir, false), Rules::Unusable(_)));

    std::fs::remove_file(dir.join(IGNORE_FILE)).unwrap();
    std::fs::create_dir(dir.join(IGNORE_FILE)).unwrap();
    assert!(matches!(load_rules(dir, false), Rules::Unusable(_)));
}

/// A track under a folder the walk could not read is not missing, so no
/// file elsewhere may claim it; nor may any track when the walk could not
/// name what it missed.
#[test]
fn a_track_the_walk_could_not_see_is_not_missing() {
    let discovered = std::collections::HashSet::from(["Found/One.wav".to_owned()]);
    let unreadable = vec!["Locked".to_owned()];
    let walked = Walked {
        discovered: &discovered,
        unreadable: &unreadable,
        incomplete: false,
        scope: None,
    };
    assert!(!walked.saw_missing("Found/One.wav"));
    assert!(!walked.saw_missing("Locked/Two.wav"));
    assert!(!walked.saw_missing("Locked"));
    assert!(walked.saw_missing("Lockedness/Three.wav"));
    assert!(walked.saw_missing("Gone/Four.wav"));
    let incomplete = Walked {
        incomplete: true,
        ..walked
    };
    assert!(!incomplete.saw_missing("Gone/Four.wav"));
    // A scoped walk looked in its folders only: a track anywhere else was
    // not looked for, so it was not seen missing.
    let scope = vec!["Gone".to_owned()];
    let scoped = Walked {
        scope: Some(&scope),
        ..walked
    };
    assert!(scoped.saw_missing("Gone/Four.wav"));
    assert!(!scoped.saw_missing("Elsewhere/Five.wav"));
    assert!(!scoped.saw_missing("Goner/Six.wav"));
}

/// A scope is a list of folders a walk can trust, or the whole library.
#[test]
fn a_scope_is_plain_folders_or_the_whole_library() {
    let folders = |list: &[&str]| list.iter().map(|f| (*f).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        super::normalised_scope(&folders(&["b/c", "a", "a/x/y", "b/c/", "bc"])),
        Some(folders(&["a", "b/c", "bc"])),
        "nested folders are covered by their parent, and a prefix is not a parent"
    );
    // A sibling that sorts between a folder and its child: "-", " " and
    // "(" all come before "/".
    assert_eq!(
        super::normalised_scope(&folders(&[
            "Rock/Album",
            "Rock - Live",
            "Rock",
            "Artist (Live)",
            "Artist/Album/CD1"
        ])),
        Some(folders(&[
            "Artist (Live)",
            "Artist/Album/CD1",
            "Rock",
            "Rock - Live"
        ])),
    );
    for whole in [&[""][..], &["a/../b"], &["/etc"], &["a//b"], &["."], &[]] {
        assert_eq!(super::normalised_scope(&folders(whole)), None, "{whole:?}");
    }
    let many: Vec<String> = (0..=super::MAX_SCOPE_FOLDERS)
        .map(|index| format!("f{index}"))
        .collect();
    assert_eq!(super::normalised_scope(&many), None);
}
