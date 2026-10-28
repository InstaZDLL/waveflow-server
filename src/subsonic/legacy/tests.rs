use super::*;

fn names(node: &Node) -> Vec<&str> {
    node.children
        .iter()
        .map(|child| child.name.as_str())
        .collect()
}

/// The lists are per element: a name OpenSubsonic added to a media item is a
/// Subsonic field, or a whole container, somewhere else, and must survive
/// there.
#[test]
fn a_name_is_removed_only_from_the_element_opensubsonic_added_it_to() {
    let response = ok_node()
        .child(
            Node::new("playlist")
                .attr("id", "playlist")
                .attr("comment", "Road trip")
                .attr("readonly", true)
                .attr("validUntil", "2026-10-28T00:00:00+00:00")
                .child(
                    Node::new("entry")
                        .attr("id", "track")
                        .attr("title", "Ave Maria")
                        .attr("comment", "Second take")
                        .attr("userRating", 4)
                        .child(Node::new("genres").attr("name", "Classical"))
                        .child(
                            Node::new("contributors")
                                .attr("role", "producer")
                                .child(Node::new("artist").attr("name", "Hal Brennan")),
                        ),
                ),
        )
        .child(Node::new("genres").child(Node::new("genre").text("Classical")));

    let shaped = Wire::Legacy.shape(response.clone());

    // The envelope is the server's description and stays whole.
    assert_eq!(shaped.attrs, response.attrs);
    assert_eq!(names(&shaped), ["playlist", "genres"]);
    let playlist = &shaped.children[0];
    // A playlist's comment is Subsonic; what a smart playlist added is not.
    assert!(playlist.attrs.contains_key("comment"));
    assert!(!playlist.attrs.contains_key("readonly"));
    assert!(!playlist.attrs.contains_key("validUntil"));
    // A track's comment is OpenSubsonic, its rating is Subsonic.
    let entry = &playlist.children[0];
    assert!(!entry.attrs.contains_key("comment"));
    assert_eq!(entry.attrs["userRating"], 4);
    assert_eq!(entry.attrs["title"], "Ave Maria");
    assert!(entry.children.is_empty(), "{:?}", names(entry));
    // `genres` as getGenres' container is not the song's array.
    assert_eq!(names(&shaped.children[1]), ["genre"]);
}

/// The full wire is the response as the builders made it.
#[test]
fn the_opensubsonic_wire_changes_nothing() {
    let response = ok_node().child(
        Node::new("song")
            .attr("id", "track")
            .attr("displayComposer", "Bach")
            .child(Node::new("replayGain")),
    );
    let shaped = Wire::OpenSubsonic.shape(response.clone());
    assert_eq!(shaped.attrs, response.attrs);
    assert_eq!(shaped.children[0].attrs, response.children[0].attrs);
    assert_eq!(names(&shaped.children[0]), ["replayGain"]);
}
