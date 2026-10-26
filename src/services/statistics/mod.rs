//! Listening statistics — RFC-024 decision 6.
//!
//! **Built from `play_event` and nothing else.** A listen is a row with
//! `submission = 1`, what scrobbling and play counts already count; a "now
//! playing" is not one.
//!
//! **Tenancy is in every statement.** Each one starts from [`listens_cte!`],
//! which binds the caller's id first, joins the track to `library_member` and
//! requires `pe.user_id = m.user_id`: a user sees only their own listens, and
//! only on available tracks in libraries they are still a member of.

use chrono::{Datelike, NaiveDate};

use super::*;

/// The caller's listens in the period, **one row per track**: how often and
/// when last, then the track's length, album and first credited artist.
///
/// Counted first and joined second. The count reads the covering index
/// `(user_id, submission, track_id, played_at)` and never touches a row of
/// `play_event`; the join and the artist lookup then run once per track
/// rather than once per listen. Joined the other way round, a year of
/// listening cost every statement a correlated subquery per listen, and the
/// page made seven of them.
///
/// Binds the user id, the start of the period in milliseconds, then the user
/// id again — for the membership the track must still be reached through.
macro_rules! listened_cte {
    () => {
        "WITH counted AS (\
           SELECT pe.track_id, COUNT(*) AS plays, MAX(pe.played_at) AS last_played \
             FROM play_event pe \
            WHERE pe.user_id=? AND pe.submission=1 AND pe.played_at >= ? \
            GROUP BY pe.track_id), \
         listened AS (\
           SELECT c.track_id, c.plays, c.last_played, t.duration_ms, t.album_id, t.title, \
                  (SELECT tp.artist_id FROM track_participant tp \
                    WHERE tp.track_id=t.id AND tp.role='artist' \
                    ORDER BY tp.position LIMIT 1) AS artist_id \
             FROM counted c JOIN track t ON t.id=c.track_id \
             JOIN library_member m ON m.library_id=t.library_id AND m.user_id=? \
            WHERE t.is_available=1) "
    };
}

/// Every listen in the period on a track the caller still reaches, reduced to
/// when it was and how long the track runs — what the timeline and the first
/// and last listen need, and nothing more. Binds the user id twice, then the
/// start of the period.
macro_rules! each_listen_cte {
    () => {
        "WITH each_listen AS (\
           SELECT pe.track_id, pe.played_at, t.duration_ms \
             FROM play_event pe JOIN track t ON t.id=pe.track_id \
             JOIN library_member m ON m.library_id=t.library_id AND m.user_id=? \
            WHERE pe.user_id=? AND pe.submission=1 AND t.is_available=1 \
              AND pe.played_at >= ?) "
    };
}

/// The most entries a top list may hold.
pub const MAX_STATS_LIMIT: i64 = 50;

/// How far from UTC a client may say it is, in minutes: the widest offsets in
/// use, UTC−14:00 to UTC+14:00.
pub const MAX_UTC_OFFSET_MINUTES: i32 = 14 * 60;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The period a statistics request covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatsPeriod {
    Week,
    #[default]
    Month,
    Year,
    All,
}

impl FromStr for StatsPeriod {
    type Err = ServiceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "7d" => Self::Week,
            "30d" => Self::Month,
            "365d" => Self::Year,
            "all" => Self::All,
            _ => return Err(ServiceError::Invalid),
        })
    }
}

impl StatsPeriod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Week => "7d",
            Self::Month => "30d",
            Self::Year => "365d",
            Self::All => "all",
        }
    }

    fn bucket(self) -> StatsBucketSize {
        match self {
            Self::Week | Self::Month => StatsBucketSize::Day,
            Self::Year | Self::All => StatsBucketSize::Month,
        }
    }
}

/// How the timeline is cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum StatsBucketSize {
    Day,
    Month,
}

/// One statistics request, validated.
#[derive(Debug, Clone, Copy)]
pub struct StatsQuery {
    pub period: StatsPeriod,
    pub utc_offset_minutes: i32,
    pub limit: i64,
}

impl StatsQuery {
    pub fn new(
        period: Option<&str>,
        utc_offset_minutes: Option<i32>,
        limit: Option<i64>,
    ) -> Result<Self, ServiceError> {
        let period = period
            .map(StatsPeriod::from_str)
            .transpose()?
            .unwrap_or_default();
        let utc_offset_minutes = utc_offset_minutes.unwrap_or(0);
        let limit = limit.unwrap_or(10);
        if !(-MAX_UTC_OFFSET_MINUTES..=MAX_UTC_OFFSET_MINUTES).contains(&utc_offset_minutes)
            || !(1..=MAX_STATS_LIMIT).contains(&limit)
        {
            return Err(ServiceError::Invalid);
        }
        Ok(Self {
            period,
            utc_offset_minutes,
            limit,
        })
    }
}

/// One account's listening over a period.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListeningStats {
    /// `7d`, `30d`, `365d` or `all`.
    pub period: String,
    /// Where the period starts, in epoch milliseconds: the listener's
    /// midnight, or the first of their month for `365d`. `None` for `all`.
    pub from: Option<i64>,
    /// How the timeline is cut: per day for `7d` and `30d`, per month for the
    /// others.
    pub bucket: StatsBucketSize,
    pub totals: StatsTotals,
    pub top_tracks: Vec<TopTrack>,
    pub top_albums: Vec<TopAlbum>,
    pub top_artists: Vec<TopArtist>,
    pub top_genres: Vec<TopGenre>,
    /// In order. Every day or month of a bounded period is present, empty
    /// ones included; `all` lists only the months that hold a listen.
    pub timeline: Vec<StatsBucket>,
    pub first_play: Option<PlayMoment>,
    pub last_play: Option<PlayMoment>,
}

/// The period's totals.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct StatsTotals {
    pub plays: i64,
    /// **An estimate**: the sum of the listened tracks' durations. No client
    /// reports how much of a track was heard.
    pub listening_ms: i64,
    pub tracks: i64,
    pub albums: i64,
    pub artists: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TopTrack {
    pub song: SongItem,
    pub plays: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TopAlbum {
    pub album: AlbumItem,
    pub plays: i64,
}

/// An artist, grouped by canonical name across the caller's libraries, and
/// named by one of its rows they can see.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TopArtist {
    pub artist: ArtistRef,
    pub artwork_hash: Option<String>,
    pub plays: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TopGenre {
    pub name: String,
    pub plays: i64,
}

/// One day or one month of the timeline.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct StatsBucket {
    /// `YYYY-MM-DD` or `YYYY-MM`, in the listener's time.
    pub label: String,
    pub plays: i64,
    pub listening_ms: i64,
}

/// A listen, and the track it was.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PlayMoment {
    pub song: SongItem,
    pub played_at: i64,
}

impl DomainServices {
    /// `GET /api/v2/stats`: the caller's listening over `query.period`.
    pub async fn listening_stats(
        &self,
        user_id: Uuid,
        query: StatsQuery,
    ) -> Result<ListeningStats, ServiceError> {
        let offset_ms = i64::from(query.utc_offset_minutes) * 60_000;
        let now = now_ms();
        let from = period_start(query.period, now, offset_ms);
        let since = from.unwrap_or(i64::MIN);
        let user = user_id.to_string();
        let mut connection = self.db.pool().acquire().await?;

        let totals = sqlx::query(concat!(
            listened_cte!(),
            "SELECT COALESCE(SUM(l.plays), 0) AS plays, \
                    COALESCE(SUM(l.plays * l.duration_ms), 0) AS listening_ms, \
                    COUNT(*) AS tracks, COUNT(DISTINCT l.album_id) AS albums, \
                    COUNT(DISTINCT a.canonical_name) AS artists \
               FROM listened l LEFT JOIN artist a ON a.id=l.artist_id"
        ))
        .bind(&user)
        .bind(since)
        .bind(&user)
        .fetch_one(&mut *connection)
        .await?;
        let totals = StatsTotals {
            plays: totals.try_get("plays")?,
            listening_ms: totals.try_get("listening_ms")?,
            tracks: totals.try_get("tracks")?,
            albums: totals.try_get("albums")?,
            artists: totals.try_get("artists")?,
        };

        // Ties go to the most recent listen, then to the name, so two
        // requests over the same history answer the same list.
        let ranked_tracks = sqlx::query(concat!(
            listened_cte!(),
            "SELECT l.track_id, l.plays FROM listened l \
              ORDER BY l.plays DESC, l.last_played DESC, l.title COLLATE NOCASE, l.track_id \
              LIMIT ?"
        ))
        .bind(&user)
        .bind(since)
        .bind(&user)
        .bind(query.limit)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| Ok((parse_uuid(row.try_get("track_id")?)?, row.try_get("plays")?)))
        .collect::<Result<Vec<(Uuid, i64)>, sqlx::Error>>()?;
        let ids: Vec<Uuid> = ranked_tracks.iter().map(|(id, _)| *id).collect();
        let mut songs: HashMap<Uuid, SongItem> = self
            .songs_by_ids_lenient_on(&mut connection, user_id, &ids)
            .await?
            .into_iter()
            .map(|song| (song.id, song))
            .collect();
        let top_tracks = ranked_tracks
            .into_iter()
            .filter_map(|(id, plays)| songs.remove(&id).map(|song| TopTrack { song, plays }))
            .collect();

        let ranked_albums = sqlx::query(concat!(
            listened_cte!(),
            "SELECT l.album_id, SUM(l.plays) AS plays, MAX(l.last_played) AS last_played \
               FROM listened l JOIN album al ON al.id=l.album_id \
              GROUP BY l.album_id \
              ORDER BY plays DESC, last_played DESC, MIN(al.title) COLLATE NOCASE, l.album_id \
              LIMIT ?"
        ))
        .bind(&user)
        .bind(since)
        .bind(&user)
        .bind(query.limit)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| Ok((parse_uuid(row.try_get("album_id")?)?, row.try_get("plays")?)))
        .collect::<Result<Vec<(Uuid, i64)>, sqlx::Error>>()?;
        let top_albums = self
            .albums_in_order(&mut connection, user_id, &ranked_albums)
            .await?;

        let top_artists = sqlx::query(concat!(
            listened_cte!(),
            "SELECT g.id, g.name, g.plays, \
                    COALESCE((SELECT c.artwork_hash FROM artist_image_choice c \
                               WHERE c.artist_id=g.id AND c.kind='portrait'), \
                             (SELECT ar.artwork_hash FROM artist ar WHERE ar.id=g.id)) \
                      AS artwork_hash \
               FROM (SELECT MIN(a.id) AS id, MIN(a.name) AS name, SUM(l.plays) AS plays, \
                            MAX(l.last_played) AS last_played \
                       FROM listened l JOIN artist a ON a.id=l.artist_id \
                      GROUP BY a.canonical_name) g \
              ORDER BY g.plays DESC, g.last_played DESC, g.name COLLATE NOCASE, g.id \
              LIMIT ?"
        ))
        .bind(&user)
        .bind(since)
        .bind(&user)
        .bind(query.limit)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| {
            Ok(TopArtist {
                artist: ArtistRef {
                    id: parse_uuid(row.try_get("id")?)?,
                    name: row.try_get("name")?,
                },
                artwork_hash: row.try_get("artwork_hash")?,
                plays: row.try_get("plays")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;

        let top_genres = sqlx::query(concat!(
            listened_cte!(),
            "SELECT MIN(g.name) AS name, SUM(l.plays) AS plays, \
                    MAX(l.last_played) AS last_played \
               FROM listened l JOIN track_genre tg ON tg.track_id=l.track_id \
               JOIN genre g ON g.id=tg.genre_id \
              GROUP BY g.canonical_name \
              ORDER BY plays DESC, last_played DESC, name COLLATE NOCASE \
              LIMIT ?"
        ))
        .bind(&user)
        .bind(since)
        .bind(&user)
        .bind(query.limit)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| {
            Ok(TopGenre {
                name: row.try_get("name")?,
                plays: row.try_get("plays")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;

        // Counted per listener's day in SQL, an integer division; named and,
        // for the long periods, folded into months here. A date format per
        // listen was the costliest expression on the page.
        let bucket = query.period.bucket();
        let days = sqlx::query(concat!(
            each_listen_cte!(),
            "SELECT (e.played_at + ?) / 86400000 AS day, COUNT(*) AS plays, \
                    COALESCE(SUM(e.duration_ms), 0) AS listening_ms \
               FROM each_listen e GROUP BY day ORDER BY day"
        ))
        .bind(&user)
        .bind(&user)
        .bind(since)
        .bind(offset_ms)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get::<i64, _>("day")?,
                row.try_get::<i64, _>("plays")?,
                row.try_get::<i64, _>("listening_ms")?,
            ))
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
        let counted = buckets_of(bucket, &days);
        let timeline = match from {
            Some(_) => fill_timeline(query.period, now, offset_ms, counted),
            None => counted,
        };

        let mut moments = Vec::with_capacity(2);
        for ordering in [
            concat!(
                each_listen_cte!(),
                "SELECT e.track_id, e.played_at FROM each_listen e \
                 ORDER BY e.played_at, e.track_id LIMIT 1"
            ),
            concat!(
                each_listen_cte!(),
                "SELECT e.track_id, e.played_at FROM each_listen e \
                 ORDER BY e.played_at DESC, e.track_id DESC LIMIT 1"
            ),
        ] {
            let moment = match sqlx::query(ordering)
                .bind(&user)
                .bind(&user)
                .bind(since)
                .fetch_optional(&mut *connection)
                .await?
            {
                Some(row) => {
                    let track = parse_uuid(row.try_get("track_id")?)?;
                    let played_at: i64 = row.try_get("played_at")?;
                    self.songs_by_ids_lenient_on(&mut connection, user_id, &[track])
                        .await?
                        .into_iter()
                        .next()
                        .map(|song| PlayMoment { song, played_at })
                }
                None => None,
            };
            moments.push(moment);
        }
        let last_play = moments.pop().flatten();
        let first_play = moments.pop().flatten();

        Ok(ListeningStats {
            period: query.period.as_str().to_owned(),
            from,
            bucket,
            totals,
            top_tracks,
            top_albums,
            top_artists,
            top_genres,
            timeline,
            first_play,
            last_play,
        })
    }

    /// The albums named in `ranked`, in its order, each with its count.
    async fn albums_in_order(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        ranked: &[(Uuid, i64)],
    ) -> Result<Vec<TopAlbum>, ServiceError> {
        if ranked.is_empty() {
            return Ok(Vec::new());
        }
        let ids = serde_json::to_string(&ranked.iter().map(|(id, _)| *id).collect::<Vec<_>>())
            .map_err(|_| ServiceError::Invalid)?;
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND al.id IN (SELECT value FROM json_each(?))"
        ))
        .bind(user_id.to_string())
        .bind(ids)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(connection, user_id, &mut albums).await?;
        let mut by_id: HashMap<Uuid, AlbumItem> =
            albums.into_iter().map(|album| (album.id, album)).collect();
        Ok(ranked
            .iter()
            .filter_map(|(id, plays)| {
                by_id.remove(id).map(|album| TopAlbum {
                    album,
                    plays: *plays,
                })
            })
            .collect())
    }
}

/// The listener's calendar date at `ms`.
fn local_date(ms: i64, offset_ms: i64) -> NaiveDate {
    chrono::DateTime::from_timestamp_millis(ms.saturating_add(offset_ms))
        .unwrap_or_default()
        .date_naive()
}

/// The instant the listener's `date` begins, in epoch milliseconds.
fn local_midnight(date: NaiveDate, offset_ms: i64) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight exists")
        .and_utc()
        .timestamp_millis()
        - offset_ms
}

/// Where a period starts: today's midnight minus six or twenty-nine days, the
/// first of the month eleven months back, or nothing for `all`.
fn period_start(period: StatsPeriod, now: i64, offset_ms: i64) -> Option<i64> {
    let today = local_date(now, offset_ms);
    let start = match period {
        StatsPeriod::Week => today - chrono::Days::new(6),
        StatsPeriod::Month => today - chrono::Days::new(29),
        StatsPeriod::Year => first_of_month(today) - chrono::Months::new(11),
        StatsPeriod::All => return None,
    };
    Some(local_midnight(start, offset_ms))
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("every month has a first")
}

/// Days counted in SQL — the listener's day number, plays, listening time, in
/// order — named and, for the long periods, folded into their months.
fn buckets_of(size: StatsBucketSize, days: &[(i64, i64, i64)]) -> Vec<StatsBucket> {
    let mut buckets: Vec<StatsBucket> = Vec::new();
    for &(day, plays, listening_ms) in days {
        let date = chrono::DateTime::from_timestamp(day.saturating_mul(86_400), 0)
            .unwrap_or_default()
            .date_naive();
        let label = match size {
            StatsBucketSize::Day => date.format("%Y-%m-%d"),
            StatsBucketSize::Month => date.format("%Y-%m"),
        }
        .to_string();
        match buckets.last_mut() {
            Some(last) if last.label == label => {
                last.plays += plays;
                last.listening_ms += listening_ms;
            }
            _ => buckets.push(StatsBucket {
                label,
                plays,
                listening_ms,
            }),
        }
    }
    buckets
}

/// Every bucket of a bounded period, the counted ones in their place.
fn fill_timeline(
    period: StatsPeriod,
    now: i64,
    offset_ms: i64,
    counted: Vec<StatsBucket>,
) -> Vec<StatsBucket> {
    let today = local_date(now, offset_ms);
    let labels: Vec<String> = match period {
        StatsPeriod::Week | StatsPeriod::Month => {
            let days = if period == StatsPeriod::Week { 7 } else { 30 };
            (0..days)
                .rev()
                .map(|back| {
                    (today - chrono::Days::new(back))
                        .format("%Y-%m-%d")
                        .to_string()
                })
                .collect()
        }
        StatsPeriod::Year => (0..12)
            .rev()
            .map(|back| {
                (first_of_month(today) - chrono::Months::new(back))
                    .format("%Y-%m")
                    .to_string()
            })
            .collect(),
        StatsPeriod::All => return counted,
    };
    let mut by_label: HashMap<String, StatsBucket> = counted
        .into_iter()
        .map(|bucket| (bucket.label.clone(), bucket))
        .collect();
    labels
        .into_iter()
        .map(|label| {
            by_label.remove(&label).unwrap_or(StatsBucket {
                label,
                plays: 0,
                listening_ms: 0,
            })
        })
        .collect()
}

/// The day number a mix is seeded with: the listener's day, counted from the
/// epoch.
pub(super) fn local_day(now: i64, offset_ms: i64) -> i64 {
    now.saturating_add(offset_ms).div_euclid(DAY_MS)
}

#[cfg(test)]
mod tests;
