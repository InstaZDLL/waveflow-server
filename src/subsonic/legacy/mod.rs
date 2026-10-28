//! The wire an older client is given: Subsonic 1.16.1, without what
//! OpenSubsonic added to the elements that schema already defines.
//!
//! **Why.** An old parser does not stop at the fields it does not know. DSub
//! 5.5.3 collects every `<artist>` element of an XML document wherever it sits,
//! so the reference nested inside a `contributors` credit became a starred
//! artist in `getStarred` — Bach and Gounod, composers of a starred track — and
//! the album header of `getMusicDirectory` showed the producer of one of its
//! tracks. Navidrome meets the same client with `subsonic.legacyclients`,
//! `DSub` by default, and answers the clients named there without any
//! OpenSubsonic field. `WAVEFLOW_SUBSONIC_LEGACY_CLIENTS` is the same setting
//! with the same default.
//!
//! **How.** One pass over the finished response, before it is rendered,
//! rather than a condition in every node builder: the builders stay a single
//! description of the full wire, and a field added to one of them later is
//! either named below or caught by the whitelist the integration tests hold
//! every legacy answer to. The pass removes a **named** list rather than
//! keeping a whitelist, so it can never take away a Subsonic 1.16.1 field that
//! nobody thought to list — the failure a whitelist would have in this file,
//! and one no client would report until it met it.
//!
//! The lists are per element, because a name means different things on
//! different elements: `comment` is an OpenSubsonic addition on a song and a
//! Subsonic attribute on a playlist, and `genres` is an OpenSubsonic array on
//! a song and the container `getGenres` answers with.
//!
//! **What stays.** Every attribute of `Child`, `AlbumID3`, `ArtistID3` and
//! `Artist` in the 1.16.1 schema, since one element name here stands for
//! several of those types — `album` is an `AlbumID3` in `getAlbumList2` and a
//! `Child` in `getAlbumList`, so `userRating` and `title`, which `Child`
//! carries, stay on it. And the envelope's `openSubsonic="true"`, as Navidrome
//! keeps it: it describes the server rather than the answer, an old client
//! ignores an attribute it does not know on the root as it does elsewhere, and
//! the methods only OpenSubsonic defines — `getOpenSubsonicExtensions`,
//! `tokenInfo`, `getLyricsBySongId` — still answer a client that asks for them.

use super::*;

/// How a response is shaped for the client that asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Wire {
    /// Everything the server supports, under the OpenSubsonic presence rule:
    /// a supported field is emitted even when the value is unknown.
    OpenSubsonic,
    /// Subsonic 1.16.1 only, for a client named in
    /// `WAVEFLOW_SUBSONIC_LEGACY_CLIENTS`.
    Legacy,
}

impl Wire {
    /// The wire for the client a request names with `c`, compared exactly with
    /// each configured name. A request without `c` is answered in full: the
    /// parameter is required by the protocol, and no legacy client omits it.
    pub(super) fn for_request(state: &AppState, params: &Params) -> Self {
        match params.first("c") {
            Some(client)
                if state
                    .subsonic_legacy_clients
                    .iter()
                    .any(|name| name == client) =>
            {
                Self::Legacy
            }
            _ => Self::OpenSubsonic,
        }
    }

    /// The response as this wire carries it.
    pub(super) fn shape(self, mut node: Node) -> Node {
        if self == Self::Legacy {
            strip_open_subsonic(&mut node);
        }
        node
    }
}

/// The attributes OpenSubsonic added to a media item, on any of the element
/// names one is rendered under.
///
/// The song's, the album's and the artist's together, because `child` in
/// `getMusicDirectory` is any of the three. None of them is an attribute of a
/// 1.16.1 `Child`, `AlbumID3`, `ArtistID3` or `Artist`, so no element under one
/// of these names loses a Subsonic field to the union.
const MEDIA_ITEM_ATTRIBUTES: &[&str] = &[
    "bitDepth",
    "bpm",
    "channelCount",
    "comment",
    "displayAlbumArtist",
    "displayArtist",
    "displayComposer",
    "explicitStatus",
    "isCompilation",
    "mediaType",
    "musicBrainzId",
    "played",
    "samplingRate",
    "sortName",
];

/// The elements OpenSubsonic added inside a media item. `contributors` is the
/// one that carried a nested `<artist>` to DSub; `artists` and
/// `albumArtists` are references a flattening parser reads the same way.
const MEDIA_ITEM_CHILDREN: &[&str] = &[
    "albumArtists",
    "artists",
    "contributors",
    "discTitles",
    "genres",
    "isrc",
    "moods",
    "originalReleaseDate",
    "recordLabels",
    "releaseDate",
    "releaseTypes",
    "replayGain",
    "roles",
];

/// What OpenSubsonic added to a playlist: a smart playlist's `readonly` and
/// `validUntil` (RFC-025 decision 6). Navidrome drops them for the same
/// clients.
const PLAYLIST_ATTRIBUTES: &[&str] = &["readonly", "validUntil"];

/// The OpenSubsonic additions on an element, as its attribute names and its
/// child element names. Empty for every element the additions never reached.
fn open_subsonic_additions(element: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match element {
        "song" | "entry" | "child" | "album" | "artist" | "similarArtist" => {
            (MEDIA_ITEM_ATTRIBUTES, MEDIA_ITEM_CHILDREN)
        }
        "playlist" => (PLAYLIST_ATTRIBUTES, &[]),
        _ => (&[], &[]),
    }
}

/// Removes the OpenSubsonic additions from a node and everything under it.
fn strip_open_subsonic(node: &mut Node) {
    let (attributes, children) = open_subsonic_additions(&node.name);
    node.attrs
        .retain(|key, _| !attributes.contains(&key.as_str()));
    node.children
        .retain(|child| !children.contains(&child.name.as_str()));
    for child in &mut node.children {
        strip_open_subsonic(child);
    }
}

#[cfg(test)]
mod tests;
