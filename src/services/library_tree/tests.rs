use super::*;

#[test]
fn a_tree_path_has_one_spelling() {
    for refused in [
        "/Artist",
        "Artist/",
        "Artist//Album",
        "./Artist",
        "Artist/..",
        "A\\B",
        "A\0B",
    ] {
        assert!(TreePath::parse(refused).is_none(), "{refused:?}");
    }
    let root = TreePath::parse("").unwrap();
    assert_eq!((root.prefix.as_str(), root.upper), ("", None));
    let album = TreePath::parse("Björk/Post").unwrap();
    assert_eq!(album.prefix, "Björk/Post/");
    assert_eq!(album.upper.as_deref(), Some("Björk/Post0"));
    // Counted in characters, as `substr` counts: the ö is one.
    assert_eq!(album.rest_from(), 12);
    assert_eq!(album.child("Disc 1"), "Björk/Post/Disc 1");
}

#[test]
fn the_upper_bound_admits_exactly_the_paths_beneath() {
    let tree = TreePath::parse("A").unwrap();
    let upper = tree.upper.unwrap();
    let beneath = |path: &str| path >= tree.prefix.as_str() && path < upper.as_str();
    assert!(beneath("A/song.flac"));
    assert!(beneath("A/B/song.flac"));
    assert!(!beneath("A.flac"));
    assert!(!beneath("A-side/song.flac"));
    assert!(!beneath("AB/song.flac"));
    assert!(!beneath("A0/song.flac"));
}
