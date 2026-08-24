//! Projection of a domain item onto a response node.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

pub(super) fn artist_node(artist: &ArtistItem, album_count: usize) -> Node {
    Node::new("artist")
        .attr("id", artist.id.to_string())
        .attr("name", artist.name.clone())
        .attr("albumCount", album_count as i64)
        .maybe_attr("coverArt", artist.artwork_hash.clone())
        .maybe_attr("starred", artist.starred_at.map(iso_time))
        .maybe_attr("userRating", artist.user_rating)
        // An OpenSubsonic addition, under the presence rule the media items
        // already follow: on an artist the identifier means the artist, and it
        // is emitted empty rather than omitted so a client can tell an untagged
        // artist from a server that does not read the tag at all.
        .attr(
            "musicBrainzId",
            artist.musicbrainz_id.clone().unwrap_or_default(),
        )
        // Now that the column exists the field is supported, so it is emitted
        // with its default rather than omitted: absent would go on saying the
        // server cannot answer, which stopped being true.
        .attr("sortName", artist.sort_name.clone().unwrap_or_default())
        // The capacities this artist is credited in. Ordered by name inside
        // the projection, where the reference emits them in map-iteration
        // order and answers differently on every request. Its two synthetic
        // roles — `total` and `maincredit` — are not OpenSubsonic role names
        // and are not stored, so they cannot leak here.
        .children(
            artist
                .roles
                .iter()
                .map(|role| Node::new("roles").text(role.clone())),
        )
}

/// `songCount` and `duration` come from the album projection rather than from
/// a slice of loaded tracks: counting them caller-side is what forced every
/// album listing to materialise the tenant's whole track list first.
pub(super) fn album_node(album: &AlbumItem) -> Node {
    Node::new("album")
        .attr("id", album.id.to_string())
        .attr("name", album.title.clone())
        .attr("title", album.title.clone())
        .maybe_attr("artist", album.artist.clone())
        .maybe_attr("artistId", album.artist_id.map(|id| id.to_string()))
        .maybe_attr("coverArt", album.artwork_hash.clone())
        .maybe_attr("year", album.year)
        .maybe_attr("starred", album.starred_at.map(iso_time))
        .maybe_attr("userRating", album.user_rating)
        .attr("songCount", album.song_count)
        .attr("duration", album.duration_ms / 1000)
        .attr("created", iso_time(album.created_at))
        // OpenSubsonic additions, under the same presence rule as `song`.
        .attr("isCompilation", album.is_compilation)
        .attr("playCount", album.play_count)
        .attr("displayArtist", album.artist.clone().unwrap_or_default())
        .attr("sortName", album.sort_name.clone().unwrap_or_default())
        .maybe_attr("played", album.last_played_at.map(iso_time))
        .children(album.artists.iter().map(|artist| {
            Node::new("artists")
                .attr("id", artist.id.to_string())
                .attr("name", artist.name.clone())
        }))
        .children(
            album
                .genres
                .iter()
                .map(|genre| Node::new("genres").attr("name", genre.clone())),
        )
        // On an album the identifier means the release, not the recording the
        // song carries. It is derived from the album's own tracks at scan time,
        // so it is a plain column read here.
        .attr(
            "musicBrainzId",
            album.musicbrainz_id.clone().unwrap_or_default(),
        )
        .children(
            album
                .record_labels
                .iter()
                .map(|label| Node::new("recordLabels").attr("name", label.clone())),
        )
        .children(
            album
                .release_types
                .iter()
                .map(|kind| Node::new("releaseTypes").text(kind.clone())),
        )
        .children(album.disc_titles.iter().map(|disc| {
            Node::new("discTitles")
                .attr("disc", disc.disc)
                .attr("title", disc.title.clone())
        }))
        // Omitted rather than emitted empty when the tag says nothing, which is
        // what the reference does: an `ItemDate` with no year is not a date.
        // The three arrays above already carry the presence signal for the
        // group, so a client can still tell "unknown" from "not supported".
        .children(
            item_date(
                "originalReleaseDate",
                album.original_release_date.as_deref(),
            )
            .into_iter()
            .chain(item_date("releaseDate", album.release_date.as_deref())),
        )
}

/// An `ItemDate` from the tag as the file spelled it.
///
/// Only the parts the tag actually names are emitted: `2019` is a year and
/// nothing more, and reporting it as 1 January 2019 would invent a precision
/// the file never claimed. Anything that does not start with a four-digit year
/// is no date at all — `19980405` written without separators is not the year
/// 19,980,405, and a bare `5` is not the year 5.
fn item_date(name: &'static str, raw: Option<&str>) -> Option<Node> {
    let raw = raw?.trim();
    let mut parts = raw.split(['-', '/', '.']);
    let head = parts.next()?.trim();
    if head.len() != 4 || !head.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year: i64 = head.parse().ok().filter(|y| *y > 0)?;
    let month = parts
        .next()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| (1..=12).contains(value));
    let day = month.and(
        parts
            .next()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .filter(|value| (1..=31).contains(value)),
    );
    Some(
        Node::new(name)
            .attr("year", year)
            .maybe_attr("month", month)
            .maybe_attr("day", day),
    )
}

pub(super) fn song_node(song: &SongItem) -> Node {
    Node::new("song")
        .attr("id", song.id.to_string())
        .attr(
            "parent",
            song.album_id.unwrap_or(song.library_id).to_string(),
        )
        .attr("isDir", false)
        .attr("title", song.title.clone())
        .maybe_attr("album", song.album.clone())
        .maybe_attr("artist", song.artist.clone())
        .maybe_attr("genre", song.genre.clone())
        .maybe_attr("year", song.year)
        .maybe_attr("track", song.track)
        .maybe_attr("discNumber", song.disc)
        .attr("duration", song.duration_ms / 1000)
        .maybe_attr("bitRate", song.bitrate)
        .attr("size", song.size)
        .attr("suffix", song.suffix.clone())
        .attr("contentType", content_type(&song.suffix))
        .attr("type", "music")
        .maybe_attr("coverArt", song.artwork_hash.clone())
        .maybe_attr("albumId", song.album_id.map(|id| id.to_string()))
        .maybe_attr("artistId", song.artist_id.map(|id| id.to_string()))
        .maybe_attr("starred", song.starred_at.map(iso_time))
        .maybe_attr("userRating", song.user_rating)
        .attr("created", iso_time(song.created_at))
        // From here down the fields are OpenSubsonic additions, and they follow
        // its presence rule rather than the omission rule the frozen 1.16
        // fields above use: a field the server supports is emitted even when
        // the value is unknown, because presence is the only way a client can
        // tell "this server does not implement it" from "this track has none".
        .attr("mediaType", "song")
        .attr("isVideo", false)
        .attr("samplingRate", song.sample_rate.unwrap_or_default())
        .attr("channelCount", song.channels.unwrap_or_default())
        .attr("bitDepth", song.bit_depth.unwrap_or_default())
        .attr("playCount", song.play_count)
        .attr("displayArtist", song.artist.clone().unwrap_or_default())
        // `played` is the one exception. Its default would be the empty
        // string, which is not a timestamp: a client parsing it strictly would
        // fail on every track nobody has played. `playCount` is always present
        // and already tells the client play statistics are supported.
        .maybe_attr("played", song.last_played_at.map(iso_time))
        .children(song.artists.iter().map(|artist| {
            Node::new("artists")
                .attr("id", artist.id.to_string())
                .attr("name", artist.name.clone())
        }))
        .children(
            song.genres
                .iter()
                .map(|genre| Node::new("genres").attr("name", genre.clone())),
        )
        // Every artist the album is credited to, not just the one the frozen
        // `artistId` field can name.
        .children(song.album_artists.iter().map(|artist| {
            Node::new("albumArtists")
                .attr("id", artist.id.to_string())
                .attr("name", artist.name.clone())
        }))
        // Everyone else the file credits: composer, producer, performer and
        // the rest, each naming what it did. `subRole` is the instrument a
        // performer is credited on, and only a performer has one.
        .children(song.contributors.iter().map(|credit| {
            Node::new("contributors")
                .attr("role", credit.role.clone())
                .maybe_attr("subRole", credit.sub_role.clone())
                .child(
                    Node::new("artist")
                        .attr("id", credit.artist.id.to_string())
                        .attr("name", credit.artist.name.clone()),
                )
        }))
        .attr(
            "displayComposer",
            song.contributors
                .iter()
                .filter(|credit| credit.role == "composer")
                .map(|credit| credit.artist.name.as_str())
                .collect::<Vec<_>>()
                .join(" \u{2022} "),
        )
        .attr(
            "displayAlbumArtist",
            song.album_artist.clone().unwrap_or_default(),
        )
        .attr(
            "musicBrainzId",
            song.musicbrainz_id.clone().unwrap_or_default(),
        )
        .attr("bpm", song.bpm.unwrap_or_default())
        .attr("sortName", song.sort_name.clone().unwrap_or_default())
        .attr("comment", song.comment.clone().unwrap_or_default())
        .children(
            song.isrc
                .iter()
                .map(|isrc| Node::new("isrc").text(isrc.clone())),
        )
        .children(
            song.moods
                .iter()
                .map(|mood| Node::new("moods").text(mood.clone())),
        )
        .attr(
            "explicitStatus",
            song.explicit_status.clone().unwrap_or_default(),
        )
        // ReplayGain is the one addition whose *members* are omitted when
        // unknown, on the specification's own instruction. The container is
        // still always present, because that is what says the server reads
        // gain tags at all; an untagged track carries an empty one.
        .child(
            Node::new("replayGain")
                .maybe_attr("trackGain", song.replay_gain_track_gain)
                .maybe_attr("trackPeak", song.replay_gain_track_peak)
                .maybe_attr("albumGain", song.replay_gain_album_gain)
                .maybe_attr("albumPeak", song.replay_gain_album_peak),
        )
}

/// `owner` is the caller: playlist reads are already scoped to their owner, so
/// there is no other name this could carry. Leaving it empty made Feishin
/// treat every playlist as someone else's and refuse to edit it.
/// `bookmarkPosition` is set on the entry rather than on every song node: it
/// is a legacy optional field, and a track only has a position inside the
/// bookmark that holds it.
pub(super) fn bookmark_node(bookmark: &crate::services::BookmarkItem, owner: &str) -> Node {
    Node::new("bookmark")
        .attr("position", bookmark.position_ms)
        .attr("username", owner)
        .maybe_attr("comment", bookmark.comment.clone())
        .attr("created", iso_time(bookmark.created_at))
        .attr("changed", iso_time(bookmark.updated_at))
        .child(
            song_node(&bookmark.song)
                .renamed("entry")
                .attr("bookmarkPosition", bookmark.position_ms),
        )
}

pub(super) fn playlist_node(playlist: &PlaylistItem, owner: &str) -> Node {
    Node::new("playlist")
        .attr("id", playlist.id.to_string())
        .attr("name", playlist.name.clone())
        .maybe_attr("comment", playlist.comment.clone())
        .attr("owner", owner)
        .attr("public", playlist.public)
        .attr("songCount", playlist.songs.len() as i64)
        .attr(
            "duration",
            playlist
                .songs
                .iter()
                .map(|song| song.duration_ms / 1000)
                .sum::<i64>(),
        )
        .attr("created", iso_time(playlist.created_at))
        .attr("changed", iso_time(playlist.updated_at))
}

pub(super) fn user_node(user: &crate::services::UserItem) -> Node {
    Node::new("user")
        .attr("username", user.username.clone())
        .attr("scrobblingEnabled", true)
        .attr("adminRole", user.role == AccountRole::Admin)
        .attr("settingsRole", user.role == AccountRole::Admin)
        .attr("downloadRole", true)
        .attr("uploadRole", false)
        .attr("playlistRole", true)
        .attr("coverArtRole", true)
        .attr("commentRole", false)
        .attr("podcastRole", false)
        .attr("streamRole", true)
        .attr("jukeboxRole", false)
        .attr("shareRole", true)
        .attr("videoConversionRole", false)
        .children(
            user.folder_ids
                .iter()
                .map(|id| Node::new("folder").text(id.to_string())),
        )
}

pub(super) fn share_node(
    share: &crate::services::ShareItem,
    owner: &str,
    public_url: Option<&str>,
) -> Node {
    let url = share.url_token.as_ref().map(|token| {
        let path = format!("/share/{token}");
        external_url(public_url, &path)
    });
    Node::new("share")
        .attr("id", share.id.to_string())
        .maybe_attr("url", url)
        .maybe_attr("description", share.description.clone())
        .maybe_attr("expires", share.expires_at.map(iso_time))
        .attr("username", owner)
        .attr("created", iso_time(share.created_at))
        .attr("visitCount", share.visit_count)
        .children(
            share
                .songs
                .iter()
                .map(|song| song_node(song).renamed("entry")),
        )
}

pub(super) fn external_url(base: Option<&str>, path: &str) -> String {
    base.map_or_else(|| path.to_owned(), |base| format!("{base}{path}"))
}

pub(super) fn ok_node() -> Node {
    Node::new("subsonic-response")
        .attr("xmlns", XMLNS)
        .attr("status", "ok")
        .attr("version", SUBSONIC_VERSION)
        .attr("type", "waveflow")
        .attr("serverVersion", env!("CARGO_PKG_VERSION"))
        .attr("openSubsonic", true)
}

pub(super) fn error_node(code: i64, message: &'static str) -> Node {
    Node::new("subsonic-response")
        .attr("xmlns", XMLNS)
        .attr("status", "failed")
        .attr("version", SUBSONIC_VERSION)
        .attr("type", "waveflow")
        .attr("serverVersion", env!("CARGO_PKG_VERSION"))
        .attr("openSubsonic", true)
        .child(
            Node::new("error")
                .attr("code", code)
                .attr("message", message),
        )
}
