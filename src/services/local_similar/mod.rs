//! Similar artists the catalogue itself suggests — RFC-024 decision 8.
//!
//! The fallback behind `getSimilarSongs`, `getSimilarSongs2` and
//! `GET /api/v2/similar-songs/{id}` when no metadata provider names a
//! neighbour the caller can see. Four signals, each read from the caller's
//! visible, available catalogue — every statement binds the caller's id first
//! and joins `library_member` — and the last from the caller's own listens:
//!
//! | signal | score |
//! |---|---|
//! | credited on the same track (or a track of an album credited to both) | 3 each, at most 9 |
//! | a shared canonical genre | 2 each, at most 6 |
//! | an average year within five years, or ten | 2, or 1 |
//! | listened to within 30 minutes after the seed, in its last 500 listens | 1 each, at most 5 |
//!
//! The era is only ever a bonus: an artist sharing no credit, no genre and
//! no listen with the seed is not a neighbour, whenever it was recorded.

use super::*;

/// How many local neighbours take part in the draw.
pub(super) const LOCAL_NEIGHBOURS: usize = 20;

/// The seed's listens a co-listen is looked for after, at most.
const SEED_LISTENS: i64 = 500;
/// How soon after a listen of the seed another listen counts as "together".
const TOGETHER_MS: i64 = 30 * 60 * 1000;

/// The seed's tracks: credited to it as track artist or as album artist.
/// Binds the user id, then the seed's canonical name twice.
macro_rules! seed_tracks {
    () => {
        concat!(
            "SELECT t.id, t.album_id, t.year FROM track t \
               JOIN library_member m ON m.library_id=t.library_id \
              WHERE m.user_id=? AND t.is_available=1",
            artist_credit_clause!("SELECT a.id FROM artist a WHERE a.canonical_name=?")
        )
    };
}

#[derive(Debug, Default, Clone)]
struct Score {
    credits: i64,
    genres: i64,
    together: i64,
    era: i64,
}

impl Score {
    fn qualifies(&self) -> bool {
        self.credits > 0 || self.genres > 0 || self.together > 0
    }

    fn total(&self) -> i64 {
        (3 * self.credits).min(9) + (2 * self.genres).min(6) + self.together.min(5) + self.era
    }
}

impl DomainServices {
    /// The canonical names of at most `limit` artists the catalogue holds
    /// near the one named `seed`, best first, ties by name.
    pub(super) async fn local_neighbours(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        seed: &str,
        limit: usize,
    ) -> Result<Vec<String>, ServiceError> {
        let user = user_id.to_string();
        let mut scores: HashMap<String, Score> = HashMap::new();

        // Co-credits: every other artist credited on one of the seed's
        // tracks, as track artist or through the track's album.
        for row in sqlx::query(concat!(
            "SELECT name, COUNT(DISTINCT track_id) AS shared FROM (\
               SELECT s.id AS track_id, a.canonical_name AS name FROM (",
            seed_tracks!(),
            ") s JOIN track_participant tp ON tp.track_id=s.id AND tp.role='artist' \
               JOIN artist a ON a.id=tp.artist_id \
             UNION ALL \
             SELECT s.id, a.canonical_name FROM (",
            seed_tracks!(),
            ") s JOIN album_participant ap ON ap.album_id=s.album_id \
                    AND ap.role='albumartist' \
               JOIN artist a ON a.id=ap.artist_id) \
             WHERE name <> ? GROUP BY name"
        ))
        .bind(&user)
        .bind(seed)
        .bind(seed)
        .bind(&user)
        .bind(seed)
        .bind(seed)
        .bind(seed)
        .fetch_all(&mut *connection)
        .await?
        {
            scores.entry(row.try_get("name")?).or_default().credits = row.try_get("shared")?;
        }

        // Genres: how many of the seed's genres each other artist's tracks
        // carry, the artist being a track's first credit.
        let seed_genres: Vec<String> = sqlx::query_scalar(concat!(
            "SELECT DISTINCT g.canonical_name FROM (",
            seed_tracks!(),
            ") s JOIN track_genre tg ON tg.track_id=s.id JOIN genre g ON g.id=tg.genre_id"
        ))
        .bind(&user)
        .bind(seed)
        .bind(seed)
        .fetch_all(&mut *connection)
        .await?;
        if !seed_genres.is_empty() {
            let genres = serde_json::to_string(&seed_genres).map_err(|_| ServiceError::Invalid)?;
            for row in sqlx::query(
                "SELECT a.canonical_name AS name, COUNT(DISTINCT g.canonical_name) AS shared \
                   FROM track t JOIN library_member m ON m.library_id=t.library_id \
                   JOIN track_participant tp ON tp.track_id=t.id AND tp.role='artist' \
                   JOIN artist a ON a.id=tp.artist_id \
                   JOIN track_genre tg ON tg.track_id=t.id JOIN genre g ON g.id=tg.genre_id \
                  WHERE m.user_id=? AND t.is_available=1 AND a.canonical_name <> ? \
                    AND g.canonical_name IN (SELECT value FROM json_each(?)) \
                  GROUP BY a.canonical_name",
            )
            .bind(&user)
            .bind(seed)
            .bind(&genres)
            .fetch_all(&mut *connection)
            .await?
            {
                scores.entry(row.try_get("name")?).or_default().genres = row.try_get("shared")?;
            }
        }

        // Co-listens: what the caller played within half an hour after one
        // of the seed's last listens. Only the caller's own rows, on both
        // sides of the join.
        for row in sqlx::query(concat!(
            "SELECT a.canonical_name AS name, COUNT(*) AS together FROM (\
               SELECT pe.played_at FROM (",
            seed_tracks!(),
            ") s JOIN play_event pe ON pe.track_id=s.id AND pe.user_id=? \
                    AND pe.submission=1 \
              ORDER BY pe.played_at DESC LIMIT ?) seed_listens \
             JOIN play_event later ON later.user_id=? AND later.submission=1 \
                  AND later.played_at > seed_listens.played_at \
                  AND later.played_at <= seed_listens.played_at + ? \
             JOIN track t2 ON t2.id=later.track_id AND t2.is_available=1 \
             JOIN library_member m2 ON m2.library_id=t2.library_id \
                  AND m2.user_id=later.user_id \
             JOIN track_participant tp ON tp.track_id=t2.id AND tp.role='artist' \
             JOIN artist a ON a.id=tp.artist_id \
            WHERE a.canonical_name <> ? GROUP BY a.canonical_name"
        ))
        .bind(&user)
        .bind(seed)
        .bind(seed)
        .bind(&user)
        .bind(SEED_LISTENS)
        .bind(&user)
        .bind(TOGETHER_MS)
        .bind(seed)
        .fetch_all(&mut *connection)
        .await?
        {
            scores.entry(row.try_get("name")?).or_default().together = row.try_get("together")?;
        }

        scores.retain(|_, score| score.qualifies());
        if scores.is_empty() {
            return Ok(Vec::new());
        }

        // The era, as a bonus on the artists that already qualify.
        let seed_year: Option<f64> = sqlx::query_scalar(concat!(
            "SELECT AVG(s.year) FROM (",
            seed_tracks!(),
            ") s WHERE s.year IS NOT NULL"
        ))
        .bind(&user)
        .bind(seed)
        .bind(seed)
        .fetch_one(&mut *connection)
        .await?;
        if let Some(seed_year) = seed_year {
            let names = serde_json::to_string(&scores.keys().collect::<Vec<_>>())
                .map_err(|_| ServiceError::Invalid)?;
            for row in sqlx::query(
                "SELECT a.canonical_name AS name, AVG(t.year) AS year \
                   FROM track t JOIN library_member m ON m.library_id=t.library_id \
                   JOIN track_participant tp ON tp.track_id=t.id AND tp.role='artist' \
                   JOIN artist a ON a.id=tp.artist_id \
                  WHERE m.user_id=? AND t.is_available=1 AND t.year IS NOT NULL \
                    AND a.canonical_name IN (SELECT value FROM json_each(?)) \
                  GROUP BY a.canonical_name",
            )
            .bind(&user)
            .bind(&names)
            .fetch_all(&mut *connection)
            .await?
            {
                let name: String = row.try_get("name")?;
                let year: f64 = row.try_get("year")?;
                if let Some(score) = scores.get_mut(&name) {
                    score.era = era_bonus(seed_year, year);
                }
            }
        }

        let mut ranked: Vec<(String, i64)> = scores
            .into_iter()
            .map(|(name, score)| (name, score.total()))
            .collect();
        ranked.sort_by(|(a_name, a_score), (b_name, b_score)| {
            b_score.cmp(a_score).then_with(|| a_name.cmp(b_name))
        });
        Ok(ranked
            .into_iter()
            .take(limit)
            .map(|(name, _)| name)
            .collect())
    }
}

/// Two within five years, one within ten, nothing further.
fn era_bonus(seed: f64, other: f64) -> i64 {
    let distance = (seed - other).abs();
    if distance <= 5.0 {
        2
    } else if distance <= 10.0 {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests;
