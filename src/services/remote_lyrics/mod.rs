//! Lyrics from LRCLIB, for a track that has none of its own — RFC-024
//! decisions 2 and 5.
//!
//! **The file and the uploads always win.** [`DomainServices::remote_sheet`]
//! answers nothing for a track with a scanned sheet that says something, or
//! with any uploaded sheet, before the lookup is consulted at all. The two
//! readers call it only when their own answer is empty, and the check is
//! repeated here so neither can ask LRCLIB about a track whose owner already
//! chose its words.
//!
//! **The tenancy lookup comes first.** The track is resolved through
//! `library_member`, the caller's id the first bind, before its lookup key is
//! computed: a track the caller cannot see is `NotFound` before anything is
//! read from the cache or sent anywhere.

use super::*;
use crate::lrclib::{FoundLyrics, LyricsLookup, TrackLookup};

impl DomainServices {
    /// Places the LRCLIB lookup every lyrics reader goes through.
    pub fn use_lyrics_lookup(&self, lookup: LyricsLookup) {
        *self
            .lyrics_lookup
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = lookup;
    }

    fn lyrics_lookup(&self) -> LyricsLookup {
        self.lyrics_lookup
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// The sheet LRCLIB has for a track without lyrics of its own, or `None`.
    ///
    /// `credited` decides whether the sheet names its source: the native
    /// answer does, and the Subsonic façade's frozen element has no field
    /// for it.
    pub(super) async fn remote_sheet(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        credited: bool,
    ) -> Result<Option<StructuredLyrics>, ServiceError> {
        let lookup = self.lyrics_lookup();
        let Some(provider) = lookup.provider() else {
            return Ok(None);
        };
        let track = sqlx::query(
            "SELECT t.title, COALESCE(ovr.title, t.title) AS effective_title, \
                    t.artist_display, t.album_title, t.duration_ms, \
                    EXISTS (SELECT 1 FROM track_lyrics_upload u WHERE u.track_id=t.id) \
                      AS uploaded \
             FROM track t JOIN library_member m ON m.library_id=t.library_id \
             LEFT JOIN track_override ovr ON ovr.track_id=t.id \
             WHERE m.user_id=? AND t.id=? AND t.is_available=1",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        if track.try_get::<bool, _>("uploaded")? {
            return Ok(None);
        }
        // Read the way `track_lyrics` reads them, so "says something" means
        // the same thing to both: a synced sheet of bare timestamps does not.
        let scanned = sqlx::query_as::<_, (String, i64)>(
            "SELECT content, synced FROM track_lyrics WHERE track_id=?",
        )
        .bind(track_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        if scanned
            .iter()
            .any(|(content, synced)| lyrics::has_text(&lyrics::lines(content, *synced != 0)))
        {
            return Ok(None);
        }
        let display_title: String = track.try_get("title")?;
        let display_artist: Option<String> = track.try_get("artist_display")?;
        let Some(artist) = display_artist.clone() else {
            return Ok(None);
        };
        let request = TrackLookup {
            artist,
            title: track.try_get("effective_title")?,
            album: track.try_get("album_title")?,
            duration_ms: track.try_get("duration_ms")?,
        };
        let Some(found) = lookup.lyrics(&request).await? else {
            return Ok(None);
        };
        Ok(sheet_of(found).map(|(synced, lines)| StructuredLyrics {
            display_artist,
            display_title,
            lang: lyrics::UNKNOWN_LANG.to_owned(),
            synced,
            lines,
            source: credited.then(|| provider.as_str().to_owned()),
        }))
    }

    /// Whether lyrics may be looked up at all: the façade's `getLyrics`
    /// widens its match only then.
    pub(super) fn looks_up_lyrics(&self) -> bool {
        self.lyrics_lookup().is_enabled()
    }
}

/// The one sheet a lookup becomes: synced when LRCLIB has timings, plain
/// otherwise, and nothing when neither says anything.
fn sheet_of(found: FoundLyrics) -> Option<(bool, Vec<lyrics::LyricsLine>)> {
    if let Some(synced) = found
        .synced
        .as_deref()
        .filter(|text| lyrics::has_lrc_timestamps(text))
    {
        let lines = lyrics::lines(synced, true);
        if lyrics::has_text(&lines) {
            return Some((true, lines));
        }
    }
    // A "synced" field with no timing in it is plain text under another
    // name; one with timings and no words is no sheet at all.
    let untimed = found
        .synced
        .as_deref()
        .filter(|text| !lyrics::has_lrc_timestamps(text));
    [found.plain.as_deref(), untimed]
        .into_iter()
        .flatten()
        .map(|text| lyrics::lines(text, false))
        .find(|lines| lyrics::has_text(lines))
        .map(|lines| (false, lines))
}

#[cfg(test)]
mod tests;
