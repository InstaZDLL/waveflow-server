use super::*;

#[test]
fn a_target_must_sit_in_an_allowed_root_and_in_no_other_library() {
    let roots = [PathBuf::from("/srv/beets")];
    let libraries = [PathBuf::from("/srv/beets/shared-library")];
    assert!(admits(&roots, &libraries, Path::new("/srv/beets/a/b.flac")));
    assert!(!admits(
        &roots,
        &libraries,
        Path::new("/srv/elsewhere/b.flac")
    ));
    assert!(!admits(
        &roots,
        &libraries,
        Path::new("/srv/beets/shared-library/b.flac")
    ));
    // A prefix of a name is not a folder.
    assert!(!admits(&roots, &[], Path::new("/srv/beetsx/b.flac")));
    assert!(!admits(&[], &[], Path::new("/srv/beets/a.flac")));
}

#[test]
fn a_root_that_does_not_resolve_reaches_nothing() {
    let present = tempfile::tempdir().unwrap();
    let roots = LinkRoots::new(vec![
        present.path().to_path_buf(),
        present.path().join("not-there"),
    ]);
    assert_eq!(
        roots.resolved(),
        vec![std::fs::canonicalize(present.path()).unwrap()]
    );
}
