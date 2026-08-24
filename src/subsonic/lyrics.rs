//! The two lyrics methods.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

pub(super) async fn get_lyrics(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let artist = params.first("artist");
    let title = params.first("title");
    let Some(lyrics) = state
        .services
        .lyrics_by_metadata(principal.id, artist, title)
        .await
        .map_err(service_protocol)?
    else {
        return Ok(Node::new("lyrics")
            .maybe_attr("artist", artist.map(str::to_owned))
            .maybe_attr("title", title.map(str::to_owned)));
    };
    let Some(first) = lyrics.structured_lyrics.first() else {
        return Ok(Node::new("lyrics"));
    };
    Ok(Node::new("lyrics")
        .maybe_attr("artist", first.display_artist.clone())
        .attr("title", first.display_title.clone())
        .text(
            first
                .lines
                .iter()
                .map(|line| line.value.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ))
}

pub(super) async fn get_lyrics_by_song_id(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let lyrics = state
        .services
        .lyrics(principal.id, params.uuid("id")?)
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("lyricsList")
        .children(lyrics.structured_lyrics.iter().map(structured_lyrics_node)))
}

pub(super) fn structured_lyrics_node(lyrics: &crate::lyrics::StructuredLyrics) -> Node {
    Node::new("structuredLyrics")
        .maybe_attr("displayArtist", lyrics.display_artist.clone())
        .attr("displayTitle", lyrics.display_title.clone())
        .attr("lang", lyrics.lang.clone())
        .attr("synced", lyrics.synced)
        .children(lyrics.lines.iter().map(|line| {
            Node::new("line")
                .maybe_attr("start", line.start)
                .text(line.value.clone())
        }))
}
