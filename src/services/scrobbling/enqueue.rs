//! Queuing a listen: one outbox entry per linked destination, written in the
//! transaction that records the listen (decision 1).

use super::*;

impl DomainServices {
    /// Queues one listen for every destination this account has linked.
    ///
    /// Called from inside the transaction that writes `play_event`, under the
    /// same writer gate — decision 1. A listen therefore cannot be recorded
    /// without being queued, nor queued without being recorded, and the
    /// idempotence `claim_operation` already provides covers both at once: a
    /// replayed scrobble rolls the transaction back and so writes neither.
    ///
    /// Silent when the account has linked nothing, which is every account until
    /// somebody says otherwise.
    pub(in crate::services) async fn enqueue_scrobble_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        play_event_id: i64,
        track_id: Uuid,
        played_at: i64,
    ) -> Result<(), ServiceError> {
        let links = sqlx::query_scalar::<_, String>(
            "SELECT id FROM scrobble_link WHERE user_id=? AND status='active' ORDER BY provider",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        if links.is_empty() {
            return Ok(());
        }
        let Some(envelope) = self
            .scrobble_envelope_on(&mut *connection, track_id, played_at)
            .await?
        else {
            return Ok(());
        };
        let artists_json =
            serde_json::to_string(&envelope.artists).map_err(|_| ServiceError::Invalid)?;
        let now = now_ms();
        for link in links {
            sqlx::query(
                "INSERT INTO scrobble_outbox (public_id, link_id, play_event_id, played_at, title, \
                 artists_json, album, album_artist, duration_ms, musicbrainz_recording_id, \
                 state, next_attempt_at, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?)",
            )
            // One per row rather than one per listen: the two destinations of a
            // single listen are two entries, and a person acts on them one at a
            // time.
            .bind(Uuid::new_v4().to_string())
            .bind(&link)
            .bind(play_event_id)
            .bind(envelope.played_at)
            .bind(&envelope.title)
            .bind(&artists_json)
            .bind(envelope.album.as_deref())
            .bind(envelope.album_artist.as_deref())
            .bind(envelope.duration_ms)
            .bind(envelope.musicbrainz_recording_id.as_deref())
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&mut *connection)
            .await?;
        }
        Ok(())
    }

    /// The listen as it was heard, or `None` when it is not worth sending.
    ///
    /// Decision 7: a track the server cannot name is never submitted. Without a
    /// title or without a single credited artist the submission is unusable at
    /// the far end and skews the statistics it lands in, so it is dropped here
    /// rather than queued to be refused later.
    async fn scrobble_envelope_on(
        &self,
        connection: &mut SqliteConnection,
        track_id: Uuid,
        played_at: i64,
    ) -> Result<Option<ScrobbleEnvelope>, ServiceError> {
        // The corrected values, through the same `COALESCE` every projection
        // reads: what was heard is what the catalogue was showing at the time.
        let Some(row) = sqlx::query(
            "SELECT COALESCE(ovr.title, t.title) AS title, t.album_title, \
                    alb.album_artist_name, t.duration_ms, \
                    COALESCE(ovr.musicbrainz_recording_id, t.musicbrainz_recording_id) \
                      AS musicbrainz_recording_id \
             FROM track t \
             LEFT JOIN album alb ON alb.id=t.album_id \
             LEFT JOIN track_override ovr ON ovr.track_id=t.id \
             WHERE t.id=?",
        )
        .bind(track_id.to_string())
        .fetch_optional(&mut *connection)
        .await?
        else {
            return Ok(None);
        };
        let title: String = row.try_get("title")?;
        if title.trim().is_empty() {
            return Ok(None);
        }
        let artists = sqlx::query_scalar::<_, String>(
            "SELECT ar.name FROM track_participant tp JOIN artist ar ON ar.id=tp.artist_id \
             WHERE tp.track_id=? AND tp.role='artist' ORDER BY tp.position",
        )
        .bind(track_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        if artists.is_empty() {
            return Ok(None);
        }
        Ok(Some(ScrobbleEnvelope {
            played_at,
            title,
            artists,
            album: row.try_get("album_title")?,
            album_artist: row.try_get("album_artist_name")?,
            duration_ms: row.try_get("duration_ms")?,
            musicbrainz_recording_id: row.try_get("musicbrainz_recording_id")?,
        }))
    }
}
