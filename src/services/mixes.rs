//! Mixes generated from local data only — RFC-024 decision 7.
//!
//! **Bounded work.** Each mix is chosen from a pool of at most [`MIX_POOL`]
//! candidates read in SQL — ranked ones by their rank, random ones from a
//! seeded starting point in the track id space — then ordered by a seeded
//! hash, capped per artist and interleaved here, and only then read through
//! the shared song projection.
//!
//! **Deterministic per day.** The seed is the account, the mix id and the
//! listener's day, so a mix is the same all day on every device.
//!
//! **Tenancy is in the pool.** [`mix_pool!`] binds the caller's id first and
//! joins `library_member`; every signal it reads about listens and stars is
//! the caller's own, through `m.user_id`.

use super::*;
use statistics::local_day;

/// The candidate projection: the track, its first credited artist folded the
/// way the catalogue folds names (else its display string, else itself, so a
/// track nobody is credited on is its own artist), and its cover. `$extra`
/// adds columns. Binds the user id, then the JSON library list twice.
macro_rules! mix_pool {
    ($extra:literal) => {
        concat!(
            "SELECT t.id, t.artwork_hash, \
                    COALESCE((SELECT a.canonical_name FROM track_participant tp \
                               JOIN artist a ON a.id=tp.artist_id \
                              WHERE tp.track_id=t.id AND tp.role='artist' \
                              ORDER BY tp.position LIMIT 1), \
                             t.artist_display, t.id) AS artist_key",
            $extra,
            " FROM track t JOIN library_member m ON m.library_id=t.library_id \
              WHERE m.user_id=? AND t.is_available=1 \
                AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?)))"
        )
    };
}

/// The most candidates a mix is chosen from.
pub const MIX_POOL: i64 = 2_000;
/// The largest mix a request may ask for, and the size one gets by default.
pub const MAX_MIX_SIZE: i64 = 100;
pub const DEFAULT_MIX_SIZE: i64 = 50;
/// How many genre and decade mixes are offered.
const GENRE_MIXES: i64 = 3;
const DECADE_MIXES: i64 = 2;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// "Recently": tracks added within this many days.
const RECENT_DAYS: i64 = 30;
/// A favourite is forgotten when not listened to for this long, and starred
/// at least this long ago so a fresh star is not one.
const FORGOTTEN_AFTER_DAYS: i64 = 90;
const FORGOTTEN_STARRED_DAYS: i64 = 30;
/// The window the "most listened" genres and decades are read over.
const TASTE_DAYS: i64 = 90;

/// Which mix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MixKind {
    MostPlayed,
    NeverPlayed,
    ForgottenFavourites,
    RecentlyAdded,
    /// A genre, by canonical name.
    Genre(String),
    /// A decade, by its first year.
    Decade(i64),
}

impl MixKind {
    /// Reads a mix id. A genre is named by its canonical form, so the id a
    /// listing hands out reads back to the same genre however it is spelled.
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "most-played" => Some(Self::MostPlayed),
            "never-played" => Some(Self::NeverPlayed),
            "forgotten-favourites" => Some(Self::ForgottenFavourites),
            "recently-added" => Some(Self::RecentlyAdded),
            _ => {
                if let Some(genre) = id.strip_prefix("genre:") {
                    let canonical = waveflow_core::scanner::canonical_name(genre);
                    (!canonical.is_empty()).then_some(Self::Genre(canonical))
                } else if let Some(decade) = id.strip_prefix("decade:") {
                    let year: i64 = decade.parse().ok()?;
                    ((1000..=2990).contains(&year) && year % 10 == 0).then_some(Self::Decade(year))
                } else {
                    None
                }
            }
        }
    }

    pub fn id(&self) -> String {
        match self {
            Self::MostPlayed => "most-played".to_owned(),
            Self::NeverPlayed => "never-played".to_owned(),
            Self::ForgottenFavourites => "forgotten-favourites".to_owned(),
            Self::RecentlyAdded => "recently-added".to_owned(),
            Self::Genre(canonical) => format!("genre:{canonical}"),
            Self::Decade(year) => format!("decade:{year}"),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::MostPlayed => "most_played",
            Self::NeverPlayed => "never_played",
            Self::ForgottenFavourites => "forgotten_favourites",
            Self::RecentlyAdded => "recently_added",
            Self::Genre(_) => "genre",
            Self::Decade(_) => "decade",
        }
    }

    /// Whether the pool's order is the mix's: a ranking rather than a draw.
    fn ranked(&self) -> bool {
        matches!(self, Self::MostPlayed | Self::RecentlyAdded)
    }
}

/// One mix request, validated.
#[derive(Debug, Clone)]
pub struct MixQuery {
    pub library_ids: Vec<Uuid>,
    pub size: i64,
    pub utc_offset_minutes: i32,
}

impl MixQuery {
    pub fn new(
        library_ids: Vec<Uuid>,
        size: Option<i64>,
        utc_offset_minutes: Option<i32>,
    ) -> Result<Self, ServiceError> {
        let size = size.unwrap_or(DEFAULT_MIX_SIZE);
        let utc_offset_minutes = utc_offset_minutes.unwrap_or(0);
        if !(1..=MAX_MIX_SIZE).contains(&size)
            || !(-statistics::MAX_UTC_OFFSET_MINUTES..=statistics::MAX_UTC_OFFSET_MINUTES)
                .contains(&utc_offset_minutes)
        {
            return Err(ServiceError::Invalid);
        }
        Ok(Self {
            library_ids,
            size,
            utc_offset_minutes,
        })
    }
}

/// A mix as the list shows it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MixSummary {
    /// What `GET /api/v2/mixes/{mix_id}` takes: `most-played`,
    /// `never-played`, `forgotten-favourites`, `recently-added`,
    /// `genre:<canonical name>` or `decade:<first year>`.
    pub id: String,
    /// `most_played`, `never_played`, `forgotten_favourites`,
    /// `recently_added`, `genre` or `decade`: what a client names the mix by.
    pub kind: String,
    /// The genre's display name, for a genre mix.
    pub genre: Option<String>,
    /// The decade's first year, for a decade mix.
    pub decade: Option<i64>,
    pub track_count: i64,
    /// Up to four distinct covers from the mix, in its order.
    pub artwork_hashes: Vec<String>,
}

/// A mix and its tracks, in order.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Mix {
    #[serde(flatten)]
    pub summary: MixSummary,
    pub songs: Vec<SongItem>,
}

/// One candidate.
#[derive(Debug, Clone)]
struct Candidate {
    id: Uuid,
    artist: String,
    artwork_hash: Option<String>,
}

impl DomainServices {
    /// `GET /api/v2/mixes`: the mixes the caller has today, those with no
    /// track left out.
    pub async fn list_mixes(
        &self,
        user_id: Uuid,
        query: &MixQuery,
    ) -> Result<Vec<MixSummary>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        let mut kinds = vec![
            (MixKind::MostPlayed, None),
            (MixKind::ForgottenFavourites, None),
            (MixKind::RecentlyAdded, None),
            (MixKind::NeverPlayed, None),
        ];
        for (canonical, name) in self.taste_genres(&mut connection, user_id, query).await? {
            kinds.push((MixKind::Genre(canonical), Some(name)));
        }
        for decade in self.taste_decades(&mut connection, user_id, query).await? {
            kinds.push((MixKind::Decade(decade), None));
        }
        let mut summaries = Vec::new();
        for (kind, genre) in kinds {
            let chosen = self.choose(&mut connection, user_id, &kind, query).await?;
            if chosen.is_empty() {
                continue;
            }
            summaries.push(summary_of(&kind, genre, &chosen));
        }
        Ok(summaries)
    }

    /// `GET /api/v2/mixes/{mix_id}`: one mix and its tracks. A mix id the
    /// server does not know, a genre the caller cannot see and a mix with no
    /// track all answer `NotFound`.
    pub async fn mix(
        &self,
        user_id: Uuid,
        mix_id: &str,
        query: &MixQuery,
    ) -> Result<Mix, ServiceError> {
        let kind = MixKind::parse(mix_id).ok_or(ServiceError::NotFound)?;
        let mut connection = self.db.pool().acquire().await?;
        let genre = match &kind {
            MixKind::Genre(canonical) => Some(
                sqlx::query_scalar::<_, Option<String>>(
                    "SELECT MIN(g.name) FROM genre g \
                     JOIN library_member m ON m.library_id=g.library_id \
                     WHERE m.user_id=? AND g.canonical_name=?",
                )
                .bind(user_id.to_string())
                .bind(canonical)
                .fetch_one(&mut *connection)
                .await?
                .ok_or(ServiceError::NotFound)?,
            ),
            _ => None,
        };
        let chosen = self.choose(&mut connection, user_id, &kind, query).await?;
        if chosen.is_empty() {
            return Err(ServiceError::NotFound);
        }
        let ids: Vec<Uuid> = chosen.iter().map(|candidate| candidate.id).collect();
        let songs = self
            .songs_by_ids_lenient_on(&mut connection, user_id, &ids)
            .await?;
        Ok(Mix {
            summary: summary_of(&kind, genre, &chosen),
            songs,
        })
    }

    /// The tracks of one mix, in order, at most `query.size`.
    async fn choose(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        kind: &MixKind,
        query: &MixQuery,
    ) -> Result<Vec<Candidate>, ServiceError> {
        let now = now_ms();
        let offset_ms = i64::from(query.utc_offset_minutes) * 60_000;
        let seed = seed_of(user_id, kind, local_day(now, offset_ms));
        let pivot = pivot_of(&seed);
        let folders = folder_filter(&query.library_ids);
        let user = user_id.to_string();
        let rows = match kind {
            MixKind::MostPlayed => {
                sqlx::query(concat!(
                    "SELECT * FROM (",
                    mix_pool!(
                        ", (SELECT COUNT(*) FROM play_event pe \
                            WHERE pe.user_id=m.user_id AND pe.submission=1 \
                              AND pe.track_id=t.id) AS plays, \
                           (SELECT MAX(pe.played_at) FROM play_event pe \
                            WHERE pe.user_id=m.user_id AND pe.submission=1 \
                              AND pe.track_id=t.id) AS last_played"
                    ),
                    ") WHERE plays > 0 ORDER BY plays DESC, last_played DESC, id LIMIT ?"
                ))
                .bind(&user)
                .bind(folders.as_deref())
                .bind(folders.as_deref())
                .bind(MIX_POOL)
                .fetch_all(&mut *connection)
                .await?
            }
            MixKind::RecentlyAdded => {
                sqlx::query(concat!(
                    mix_pool!(""),
                    " AND t.created_at >= ? ORDER BY t.created_at DESC, t.id LIMIT ?"
                ))
                .bind(&user)
                .bind(folders.as_deref())
                .bind(folders.as_deref())
                .bind(now - RECENT_DAYS * DAY_MS)
                .bind(MIX_POOL)
                .fetch_all(&mut *connection)
                .await?
            }
            MixKind::NeverPlayed => {
                sqlx::query(concat!(
                    mix_pool!(""),
                    " AND NOT EXISTS (SELECT 1 FROM play_event pe \
                        WHERE pe.user_id=m.user_id AND pe.submission=1 AND pe.track_id=t.id) \
                      ORDER BY (t.id < ?), t.id LIMIT ?"
                ))
                .bind(&user)
                .bind(folders.as_deref())
                .bind(folders.as_deref())
                .bind(&pivot)
                .bind(MIX_POOL)
                .fetch_all(&mut *connection)
                .await?
            }
            MixKind::ForgottenFavourites => {
                sqlx::query(concat!(
                    mix_pool!(""),
                    " AND EXISTS (SELECT 1 FROM user_star us \
                        WHERE us.user_id=m.user_id AND us.entity_type='track' \
                          AND us.entity_id=t.id AND us.starred_at < ?) \
                      AND NOT EXISTS (SELECT 1 FROM play_event pe \
                        WHERE pe.user_id=m.user_id AND pe.submission=1 \
                          AND pe.track_id=t.id AND pe.played_at >= ?) \
                      ORDER BY (t.id < ?), t.id LIMIT ?"
                ))
                .bind(&user)
                .bind(folders.as_deref())
                .bind(folders.as_deref())
                .bind(now - FORGOTTEN_STARRED_DAYS * DAY_MS)
                .bind(now - FORGOTTEN_AFTER_DAYS * DAY_MS)
                .bind(&pivot)
                .bind(MIX_POOL)
                .fetch_all(&mut *connection)
                .await?
            }
            MixKind::Genre(canonical) => {
                sqlx::query(concat!(
                    mix_pool!(""),
                    song_genre_clause!(),
                    " ORDER BY (t.id < ?), t.id LIMIT ?"
                ))
                .bind(&user)
                .bind(folders.as_deref())
                .bind(folders.as_deref())
                .bind(canonical)
                .bind(&pivot)
                .bind(MIX_POOL)
                .fetch_all(&mut *connection)
                .await?
            }
            MixKind::Decade(first) => {
                sqlx::query(concat!(
                    mix_pool!(""),
                    " AND t.year BETWEEN ? AND ? ORDER BY (t.id < ?), t.id LIMIT ?"
                ))
                .bind(&user)
                .bind(folders.as_deref())
                .bind(folders.as_deref())
                .bind(*first)
                .bind(first + 9)
                .bind(&pivot)
                .bind(MIX_POOL)
                .fetch_all(&mut *connection)
                .await?
            }
        };
        let mut pool = rows
            .into_iter()
            .map(|row| {
                Ok(Candidate {
                    id: parse_uuid(row.try_get("id")?)?,
                    artist: row.try_get("artist_key")?,
                    artwork_hash: row.try_get("artwork_hash")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        if !kind.ranked() {
            pool.sort_by_cached_key(|candidate| draw_rank(&seed, candidate.id));
        }
        Ok(vary(pool, query.size as usize))
    }

    /// The caller's most listened genres over [`TASTE_DAYS`], by canonical
    /// name with a display name, topped up with the largest ones.
    async fn taste_genres(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        query: &MixQuery,
    ) -> Result<Vec<(String, String)>, ServiceError> {
        let folders = folder_filter(&query.library_ids);
        let rows = sqlx::query(
            "WITH visible AS (SELECT t.id, m.user_id FROM track t \
                JOIN library_member m ON m.library_id=t.library_id \
               WHERE m.user_id=? AND t.is_available=1 \
                 AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?)))) \
             SELECT g.canonical_name, MIN(g.name) AS name, \
                    COUNT(pe.id) AS plays, COUNT(DISTINCT v.id) AS tracks \
               FROM visible v \
               JOIN track_genre tg ON tg.track_id=v.id JOIN genre g ON g.id=tg.genre_id \
               LEFT JOIN play_event pe ON pe.track_id=v.id AND pe.user_id=v.user_id \
                     AND pe.submission=1 AND pe.played_at >= ? \
              GROUP BY g.canonical_name \
              ORDER BY plays DESC, tracks DESC, name COLLATE NOCASE LIMIT ?",
        )
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(now_ms() - TASTE_DAYS * DAY_MS)
        .bind(GENRE_MIXES)
        .fetch_all(&mut *connection)
        .await?;
        rows.into_iter()
            .map(|row| Ok((row.try_get("canonical_name")?, row.try_get("name")?)))
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// The caller's most listened decades over [`TASTE_DAYS`], topped up with
    /// the largest ones.
    async fn taste_decades(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        query: &MixQuery,
    ) -> Result<Vec<i64>, ServiceError> {
        let folders = folder_filter(&query.library_ids);
        let rows = sqlx::query_scalar::<_, i64>(
            "WITH visible AS (SELECT t.id, t.year, m.user_id FROM track t \
                JOIN library_member m ON m.library_id=t.library_id \
               WHERE m.user_id=? AND t.is_available=1 AND t.year BETWEEN 1000 AND 2999 \
                 AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?)))) \
             SELECT decade FROM (\
               SELECT (v.year / 10) * 10 AS decade, \
                      COUNT(pe.id) AS plays, COUNT(DISTINCT v.id) AS tracks \
                 FROM visible v \
                 LEFT JOIN play_event pe ON pe.track_id=v.id AND pe.user_id=v.user_id \
                       AND pe.submission=1 AND pe.played_at >= ? \
                GROUP BY decade) \
             ORDER BY plays DESC, tracks DESC, decade DESC LIMIT ?",
        )
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(now_ms() - TASTE_DAYS * DAY_MS)
        .bind(DECADE_MIXES)
        .fetch_all(&mut *connection)
        .await?;
        Ok(rows)
    }
}

fn summary_of(kind: &MixKind, genre: Option<String>, chosen: &[Candidate]) -> MixSummary {
    let mut artwork_hashes: Vec<String> = Vec::new();
    for hash in chosen
        .iter()
        .filter_map(|candidate| candidate.artwork_hash.as_ref())
    {
        if artwork_hashes.len() == 4 {
            break;
        }
        if !artwork_hashes.contains(hash) {
            artwork_hashes.push(hash.clone());
        }
    }
    MixSummary {
        id: kind.id(),
        kind: kind.kind().to_owned(),
        genre,
        decade: match kind {
            MixKind::Decade(year) => Some(*year),
            _ => None,
        },
        track_count: chosen.len() as i64,
        artwork_hashes,
    }
}

/// The seed of one mix for one account on one day.
fn seed_of(user_id: Uuid, kind: &MixKind, day: i64) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"waveflow-mix\0");
    hasher.update(user_id.as_bytes());
    hasher.update(kind.id().as_bytes());
    hasher.update(&[0]);
    hasher.update(&day.to_le_bytes());
    *hasher.finalize().as_bytes()
}

/// Where a random pool starts in the track id space: eight hex digits, which
/// compare against the lowercase hyphenated ids the catalogue stores. Ids are
/// drawn at random, so the run of ids from here on is a random sample.
fn pivot_of(seed: &[u8; 32]) -> String {
    hex::encode(&seed[..4])
}

/// A track's place in a seeded draw.
fn draw_rank(seed: &[u8; 32], id: Uuid) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_keyed(seed);
    hasher.update(id.as_bytes());
    *hasher.finalize().as_bytes()
}

/// The pool, in its order, with no artist holding more than a tenth of the
/// mix (two at the least), cut to `size`, then interleaved so one artist's
/// tracks are not adjacent wherever another can stand between them.
fn vary(pool: Vec<Candidate>, size: usize) -> Vec<Candidate> {
    let cap = (size / 10).max(2);
    let mut taken: HashMap<String, usize> = HashMap::new();
    let mut kept = Vec::with_capacity(size);
    for candidate in pool {
        if kept.len() == size {
            break;
        }
        let count = taken.entry(candidate.artist.clone()).or_default();
        if *count >= cap {
            continue;
        }
        *count += 1;
        kept.push(candidate);
    }
    let mut ordered: Vec<Candidate> = Vec::with_capacity(kept.len());
    while !kept.is_empty() {
        let previous = ordered.last().map(|candidate| candidate.artist.as_str());
        let next = kept
            .iter()
            .position(|candidate| Some(candidate.artist.as_str()) != previous)
            .unwrap_or(0);
        ordered.push(kept.remove(next));
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(artist: &str) -> Candidate {
        Candidate {
            id: Uuid::new_v4(),
            artist: artist.to_owned(),
            artwork_hash: None,
        }
    }

    #[test]
    fn no_artist_dominates_and_none_repeats_back_to_back() {
        let mut pool: Vec<Candidate> = (0..30).map(|_| candidate("prolific")).collect();
        pool.extend((0..5).map(|index| candidate(&format!("other-{index}"))));
        let mixed = vary(pool, 20);
        let prolific = mixed.iter().filter(|c| c.artist == "prolific").count();
        // A tenth of twenty is two.
        assert_eq!(prolific, 2);
        assert_eq!(mixed.len(), 7);
        for pair in mixed.windows(2) {
            assert_ne!(pair[0].artist, pair[1].artist);
        }
    }

    #[test]
    fn the_same_day_draws_the_same_mix_and_the_next_day_another() {
        let user = Uuid::new_v4();
        let ids: Vec<Uuid> = (0..50).map(|_| Uuid::new_v4()).collect();
        let order = |day| {
            let seed = seed_of(user, &MixKind::NeverPlayed, day);
            let mut ids = ids.clone();
            ids.sort_by_cached_key(|id| draw_rank(&seed, *id));
            ids
        };
        assert_eq!(order(20_000), order(20_000));
        assert_ne!(order(20_000), order(20_001));
        assert_ne!(
            seed_of(user, &MixKind::NeverPlayed, 1),
            seed_of(Uuid::new_v4(), &MixKind::NeverPlayed, 1)
        );
    }

    #[test]
    fn a_mix_id_reads_back_to_itself() {
        for id in [
            "most-played",
            "never-played",
            "forgotten-favourites",
            "recently-added",
            "decade:1990",
        ] {
            assert_eq!(MixKind::parse(id).unwrap().id(), id);
        }
        assert_eq!(
            MixKind::parse("genre:Hip-Hop").unwrap().id(),
            MixKind::parse("genre:hip hop").unwrap().id()
        );
        assert!(MixKind::parse("decade:1995").is_none());
        assert!(MixKind::parse("genre:").is_none());
        assert!(MixKind::parse("top-secret").is_none());
    }
}
