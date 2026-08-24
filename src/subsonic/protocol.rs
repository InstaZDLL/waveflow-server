//! Rendering a node as XML or JSON, and the wire helpers.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

pub(super) fn render_protocol(node: Node, json: bool) -> Response {
    if json {
        let mut root = Map::new();
        root.insert(node.name.clone(), node_json(&node, ""));
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            Value::Object(root).to_string(),
        )
            .into_response()
    } else {
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
            node_xml(&node),
        )
            .into_response()
    }
}

pub(super) fn node_json(node: &Node, parent: &str) -> Value {
    // An array-typed element is its children, not an object wrapping them —
    // whether it holds none or several. Applying this only when empty would
    // hand a strictly typed client `[]` on an empty catalogue and an object on
    // a populated one, which is worse than being wrong consistently.
    if json_array_node(&node.name) {
        return Value::Array(
            node.children
                .iter()
                .map(|child| node_json(child, &node.name))
                .collect(),
        );
    }
    if node.attrs.is_empty() && node.children.is_empty() {
        if let Some(text) = &node.text {
            return Value::String(text.clone());
        }
    }
    let mut map = Map::new();
    for (key, value) in &node.attrs {
        if key != "xmlns" {
            map.insert(key.clone(), value.clone());
        }
    }
    let mut grouped: BTreeMap<&str, Vec<&Node>> = BTreeMap::new();
    for child in &node.children {
        grouped.entry(&child.name).or_default().push(child);
    }
    for (name, children) in grouped {
        let value = if children.len() == 1 && !json_array_field(&node.name, name) {
            node_json(children[0], &node.name)
        } else {
            Value::Array(
                children
                    .into_iter()
                    .map(|child| node_json(child, &node.name))
                    .collect(),
            )
        };
        map.insert(name.to_owned(), value);
    }
    // A browsing child is a song, an album or an artist under one element
    // name, and its own fields are what tell them apart: an artist carries
    // `albumCount`, an album `songCount`, a song neither. Injecting a song's
    // relations into a folder entry would have an artist answer `isrc: []`,
    // and injecting an album's would have it answer `artists: []` — a list of
    // the artists of an artist.
    let entry_kind = match (
        node.attrs.contains_key("albumCount"),
        node.attrs.contains_key("songCount"),
    ) {
        (true, _) => EntryKind::Artist,
        (_, true) => EntryKind::Album,
        _ => EntryKind::Song,
    };
    for name in json_required_array_fields(parent, &node.name) {
        let injected = match entry_kind {
            // An artist keeps its own array and takes nobody else's: a folder
            // entry answering `isrc: []` would say the server read a recording
            // identifier off a directory, and `artists: []` would be the list
            // of the artists of an artist.
            EntryKind::Artist => *name == "roles",
            EntryKind::Album => matches!(*name, "artists" | "genres"),
            EntryKind::Song => true,
        };
        if !injected {
            continue;
        }
        map.entry((*name).to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
    }
    if let Some(text) = &node.text {
        map.insert("value".into(), Value::String(text.clone()));
    }
    Value::Object(map)
}

/// Elements the OpenSubsonic specification types as a JSON array rather than an
/// object. They must serialise as `[]` when empty; an empty object breaks
/// strictly typed clients that decode the field into a list.
pub(super) fn json_array_node(name: &str) -> bool {
    matches!(name, "openSubsonicExtensions")
}

pub(super) fn json_required_array_fields(parent: &str, name: &str) -> &'static [&'static str] {
    // A contributor's artist is a reference — an identifier and a display
    // name — and shares its element name with the record. Without the parent
    // to tell them apart, every array the record carries would be injected
    // into the reference, which is exactly what
    // `an_artist_reference_is_not_an_artist_record` forbids.
    if parent == "contributors" && name == "artist" {
        return &[];
    }
    match name {
        "lyricsList" => &["structuredLyrics"],
        "structuredLyrics" => &["line"],
        // Emitted as `[]` rather than omitted when a track has no credited
        // artist or no genre: under the OpenSubsonic presence rule an absent
        // key means the server does not support the field at all.
        "song" | "entry" | "child" => &[
            "artists",
            "genres",
            "isrc",
            "moods",
            "albumArtists",
            "contributors",
        ],
        "album" => &["artists", "genres"],
        // The roles an artist is credited in, empty rather than absent for
        // the same reason: absent would say the server does not read them.
        "artist" => &["roles"],
        _ => &[],
    }
}

/// Extensions this server actually implements, with their supported versions.
///
/// The list was empty, which told every third-party client that WaveFlow
/// supports nothing optional — so a client that could have posted a long
/// request, authenticated with an API key or seeked a transcode fell back to
/// the lowest common denominator instead.
///
/// **Only advertise what is implemented and covered by tests.** Announcing an
/// extension the server does not honour is worse than announcing none: the
/// client stops probing and starts relying on it.
///
/// The specification defines no XML shape for this method, so `versions`
/// renders as a JSON array here and stringifies as `"[1]"` in the XML branch.
/// Clients that use the method request JSON.
pub(super) fn open_subsonic_extensions() -> Node {
    let extension = |name: &str, versions: Vec<i64>| {
        Node::new("openSubsonicExtension").attr("name", name).attr(
            "versions",
            Value::Array(versions.into_iter().map(Value::from).collect()),
        )
    };
    Node::new("openSubsonicExtensions")
        // POST with application/x-www-form-urlencoded, for requests too long
        // for a query string.
        .child(extension("formPost", vec![1]))
        // `apiKey` in place of the u/p and u/t/s pairs.
        .child(extension("apiKeyAuthentication", vec![1]))
        // `timeOffset` on stream, honoured for transcoded output.
        .child(extension("transcodeOffset", vec![1]))
        // Structured plain or line-synchronised lyrics by stable song UUID.
        .child(extension("songLyrics", vec![1]))
}

pub(super) fn json_array_field(parent: &str, name: &str) -> bool {
    matches!(
        (parent, name),
        ("musicFolders", "musicFolder")
            | ("indexes", "index")
            | ("artists", "index")
            | ("index", "artist")
            | ("artist", "album")
            | ("album", "song")
            | ("genres", "genre")
            | ("directory", "child")
            | ("albumList", "album")
            | ("albumList2", "album")
            | ("randomSongs", "song")
            | ("songsByGenre", "song")
            | ("searchResult3" | "searchResult2", "artist" | "album" | "song")
            | ("playlists", "playlist")
            | ("playlist", "entry")
            | ("bookmarks", "bookmark")
            | ("starred2" | "starred", "artist" | "album" | "song")
            | ("nowPlaying", "song")
            | ("playQueue", "song")
            | ("shares", "share")
            | ("share", "entry")
            | ("users", "user")
            | ("user", "folder")
            | ("openSubsonicExtensions", "openSubsonicExtension")
            | ("lyricsList", "structuredLyrics")
            | ("structuredLyrics", "line")
            // A media item is rendered as `song`, and renamed to `entry` inside
            // a playlist or share and to `child` inside a directory. Its
            // OpenSubsonic relations are arrays under all three names.
            | ("song" | "entry" | "child" | "album", "artists" | "genres")
            | ("song" | "entry" | "child", "isrc" | "moods" | "albumArtists")
            | ("song" | "entry" | "child", "contributors")
            // An artist rendered as a browsing child keeps the record's shape,
            // so its roles stay an array there too — otherwise the field
            // collapses into a bare object the moment a directory carries it.
            | ("artist" | "child", "roles")
    )
}

pub(super) fn empty_success_method(method: &str) -> bool {
    matches!(
        method,
        "updatePlaylist"
            | "deletePlaylist"
            | "star"
            | "unstar"
            | "setRating"
            | "scrobble"
            | "savePlayQueue"
            | "createBookmark"
            | "deleteBookmark"
            | "deleteShare"
            | "createUser"
            | "updateUser"
            | "deleteUser"
            | "changePassword"
    )
}

pub(super) fn node_xml(node: &Node) -> String {
    let mut output = String::new();
    write_xml(node, &mut output);
    output
}

pub(super) fn write_xml(node: &Node, output: &mut String) {
    output.push('<');
    output.push_str(&node.name);
    for (key, value) in &node.attrs {
        output.push(' ');
        output.push_str(key);
        output.push_str("=\"");
        output.push_str(&xml_escape(&value_string(value)));
        output.push('"');
    }
    if node.children.is_empty() && node.text.is_none() {
        output.push_str("/>");
        return;
    }
    output.push('>');
    if let Some(text) = &node.text {
        output.push_str(&xml_escape(text));
    }
    for child in &node.children {
        write_xml(child, output);
    }
    output.push_str("</");
    output.push_str(&node.name);
    output.push('>');
}

pub(super) fn parse_pairs(raw: &str) -> Result<Params, ProtocolError> {
    serde_urlencoded::from_str::<Vec<(String, String)>>(raw)
        .map(Params)
        .map_err(|_| invalid("Invalid parameters"))
}

pub(super) fn content_type(suffix: &str) -> &'static str {
    match suffix {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "dsf" | "dff" => "audio/dsd",
        _ => "application/octet-stream",
    }
}

pub(super) fn iso_time(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .unwrap_or_default()
        .to_rfc3339()
}

pub(super) fn parse_time(value: &str) -> Result<i64, ProtocolError> {
    if let Ok(millis) = value.parse::<i64>() {
        return Ok(millis);
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.timestamp_millis())
        .map_err(|_| invalid("Invalid date"))
}

pub(super) fn value_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        _ => value.to_string(),
    }
}

pub(super) fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
