//! What a library's owners and managers are asked to look at (RFC-015
//! decision 8): the joined credits waiting for a decision, and the desktop's
//! track categories that the server can both compute and act on.
//!
//! Every track category reads the **effective** values — the file's tag
//! unless `track_override` corrects it — so a track fixed in the editor leaves
//! its category at once. Only available tracks count. A category offers no
//! action of its own: it lists tracks, and the track editor is where they are
//! fixed, as on the desktop.

use std::collections::{BTreeMap, HashMap};

use super::credit_decisions::curated_library;
use super::*;

/// One list on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttentionCategory {
    /// Joined credits with no decision (`GET …/attention/credits`).
    SplitCredits,
    MissingArtist,
    MissingAlbum,
    MissingYear,
    MissingTrackNumber,
    MissingCover,
    AlbumYearConflict,
    CompilationNoAlbumArtist,
    DuplicateTrackNumber,
    TrackNumberGap,
    ProbableDuplicate,
}

impl AttentionCategory {
    /// Display order, the desktop's.
    pub const ALL: [Self; 11] = [
        Self::SplitCredits,
        Self::MissingArtist,
        Self::MissingAlbum,
        Self::MissingYear,
        Self::MissingTrackNumber,
        Self::MissingCover,
        Self::AlbumYearConflict,
        Self::CompilationNoAlbumArtist,
        Self::DuplicateTrackNumber,
        Self::TrackNumberGap,
        Self::ProbableDuplicate,
    ];

    /// `None` for anything that is not a track category, so a route can answer
    /// it as an invalid parameter.
    pub fn parse_track_category(value: &str) -> Option<Self> {
        let category: Self =
            serde_json::from_value(serde_json::Value::String(value.into())).ok()?;
        (category != Self::SplitCredits).then_some(category)
    }
}

/// How many items one list holds.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AttentionCount {
    pub category: AttentionCategory,
    pub count: i64,
}

/// A track in one category. A probable duplicate carries the group it
/// belongs to, numbered from one, so a client can draw the groups.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AttentionTrack {
    #[serde(flatten)]
    pub song: SongItem,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_group: Option<i64>,
}

// The criteria, over `song_select!`'s aliases: `t` the track, `ovr` its
// correction, `alb` its album. Static, as every query here is, and composed
// with `concat!` so nothing is ever spliced in at run time.

macro_rules! missing_artist {
    () => {
        "NOT EXISTS (SELECT 1 FROM track_participant tp \
           WHERE tp.track_id = t.id AND tp.role = 'artist')"
    };
}
macro_rules! missing_album {
    () => {
        "t.album_id IS NULL"
    };
}
macro_rules! missing_year {
    () => {
        "COALESCE(ovr.year, t.year, 0) = 0"
    };
}
macro_rules! missing_track_number {
    () => {
        "COALESCE(ovr.track_number, t.track_number, 0) <= 0"
    };
}
// The album's cover stands for its tracks, as it does on every page that
// draws one.
macro_rules! missing_cover {
    () => {
        "t.artwork_hash IS NULL AND alb.artwork_hash IS NULL"
    };
}
macro_rules! album_year_conflict {
    () => {
        "t.album_id IS NOT NULL AND (SELECT COUNT(DISTINCT COALESCE(o2.year, t2.year)) \
           FROM track t2 LEFT JOIN track_override o2 ON o2.track_id = t2.id \
           WHERE t2.album_id = t.album_id AND t2.is_available = 1 \
             AND COALESCE(o2.year, t2.year, 0) > 0) > 1"
    };
}
// The server files a compilation that names no album artist under `Various
// Artists`, and cannot tell that fallback from a file that writes those words
// itself — so both are listed; a compilation credited to nobody in particular
// is the case either way.
macro_rules! compilation_no_album_artist {
    () => {
        "alb.is_compilation = 1 AND alb.album_artist_name = 'Various Artists'"
    };
}
macro_rules! duplicate_track_number {
    () => {
        "t.album_id IS NOT NULL AND COALESCE(ovr.track_number, t.track_number, 0) > 0 \
         AND EXISTS (SELECT 1 FROM track t2 LEFT JOIN track_override o2 ON o2.track_id = t2.id \
           WHERE t2.album_id = t.album_id AND t2.id <> t.id AND t2.is_available = 1 \
             AND COALESCE(o2.disc_number, t2.disc_number, 1) \
               = COALESCE(ovr.disc_number, t.disc_number, 1) \
             AND COALESCE(o2.track_number, t2.track_number, 0) \
               = COALESCE(ovr.track_number, t.track_number, 0))"
    };
}
macro_rules! track_number_gap {
    () => {
        "t.album_id IS NOT NULL AND (SELECT \
             MAX(COALESCE(o2.track_number, t2.track_number)) \
           - MIN(COALESCE(o2.track_number, t2.track_number)) + 1 \
           - COUNT(DISTINCT COALESCE(o2.track_number, t2.track_number)) \
           FROM track t2 LEFT JOIN track_override o2 ON o2.track_id = t2.id \
           WHERE t2.album_id = t.album_id AND t2.is_available = 1 \
             AND COALESCE(o2.disc_number, t2.disc_number, 1) \
               = COALESCE(ovr.disc_number, t.disc_number, 1) \
             AND COALESCE(o2.track_number, t2.track_number, 0) > 0) > 0"
    };
}

macro_rules! count_head {
    () => {
        "SELECT COUNT(*) FROM track t \
         LEFT JOIN album alb ON alb.id = t.album_id \
         LEFT JOIN track_override ovr ON ovr.track_id = t.id \
         WHERE t.library_id = ? AND t.is_available = 1 AND ("
    };
}

macro_rules! by_sleeve {
    () => {
        " ORDER BY t.album_title COLLATE NOCASE NULLS LAST, \
           COALESCE(ovr.disc_number, t.disc_number) NULLS LAST, \
           COALESCE(ovr.track_number, t.track_number) NULLS LAST, \
           COALESCE(ovr.title, t.title) COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
    };
}

/// The count query and the listing query of one criterion.
macro_rules! category_sql {
    ($criterion:ident) => {
        (
            concat!(count_head!(), $criterion!(), ")"),
            concat!(
                song_select!(),
                " AND t.library_id = ? AND (",
                $criterion!(),
                ")",
                by_sleeve!()
            ),
        )
    };
}

impl AttentionCategory {
    /// Its two queries, for the categories SQL can answer alone.
    fn sql(self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Self::MissingArtist => category_sql!(missing_artist),
            Self::MissingAlbum => category_sql!(missing_album),
            Self::MissingYear => category_sql!(missing_year),
            Self::MissingTrackNumber => category_sql!(missing_track_number),
            Self::MissingCover => category_sql!(missing_cover),
            Self::AlbumYearConflict => category_sql!(album_year_conflict),
            Self::CompilationNoAlbumArtist => category_sql!(compilation_no_album_artist),
            Self::DuplicateTrackNumber => category_sql!(duplicate_track_number),
            Self::TrackNumberGap => category_sql!(track_number_gap),
            Self::SplitCredits | Self::ProbableDuplicate => return None,
        })
    }
}

impl DomainServices {
    /// How many items each list holds, in display order.
    pub async fn attention_summary(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<Vec<AttentionCount>, ServiceError> {
        let candidates = self.credit_candidates(user_id, library_id).await?;
        let duplicates = self.probable_duplicates(library_id).await?;
        let mut counts = Vec::with_capacity(AttentionCategory::ALL.len());
        for category in AttentionCategory::ALL {
            let count = match category.sql() {
                Some((count, _)) => {
                    sqlx::query_scalar::<_, i64>(count)
                        .bind(library_id.to_string())
                        .fetch_one(self.db.pool())
                        .await?
                }
                None if category == AttentionCategory::SplitCredits => candidates.len() as i64,
                None => duplicates.len() as i64,
            };
            counts.push(AttentionCount { category, count });
        }
        Ok(counts)
    }

    /// One page of the tracks in one category.
    pub async fn attention_tracks(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        category: AttentionCategory,
        page: BrowsePage,
    ) -> Result<Vec<AttentionTrack>, ServiceError> {
        {
            let mut tx = self.db.pool().begin().await?;
            curated_library(&mut tx, user_id, library_id).await?;
            tx.commit().await?;
        }
        if let Some((_, listing)) = category.sql() {
            let rows = sqlx::query(listing)
                .bind(user_id.to_string())
                .bind(library_id.to_string())
                .bind(page.limit)
                .bind(page.offset)
                .fetch_all(self.db.pool())
                .await?;
            let mut songs = rows
                .into_iter()
                .map(song_from_row)
                .collect::<Result<Vec<_>, _>>()?;
            attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs)
                .await?;
            return Ok(songs
                .into_iter()
                .map(|song| AttentionTrack {
                    song,
                    duplicate_group: None,
                })
                .collect());
        }
        if category != AttentionCategory::ProbableDuplicate {
            return Err(ServiceError::Invalid);
        }
        let all = self.probable_duplicates(library_id).await?;
        let window: Vec<(Uuid, i64)> = all
            .into_iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .collect();
        let ids: Vec<Uuid> = window.iter().map(|(id, _)| *id).collect();
        let mut connection = self.db.pool().acquire().await?;
        let songs = self
            .songs_by_ids_lenient_on(&mut connection, user_id, &ids)
            .await?;
        let mut by_id: HashMap<Uuid, SongItem> =
            songs.into_iter().map(|song| (song.id, song)).collect();
        Ok(window
            .into_iter()
            .filter_map(|(id, group)| {
                by_id.remove(&id).map(|song| AttentionTrack {
                    song,
                    duplicate_group: Some(group),
                })
            })
            .collect())
    }

    /// Every available track of the library that is probably another one,
    /// with its group: the same normalised title, the same credited artists,
    /// and durations that chain within two seconds — the desktop's rule,
    /// through the same two functions of `waveflow-core`.
    ///
    /// In a fixed order — groups by title and artists, members by duration —
    /// so a page means the same thing on every request.
    async fn probable_duplicates(
        &self,
        library_id: Uuid,
    ) -> Result<Vec<(Uuid, i64)>, ServiceError> {
        let tracks: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT t.id, COALESCE(ovr.title, t.title), t.duration_ms FROM track t \
             LEFT JOIN track_override ovr ON ovr.track_id = t.id \
             WHERE t.library_id = ? AND t.is_available = 1",
        )
        .bind(library_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        let credited: Vec<(String, String)> = sqlx::query_as(
            "SELECT tp.track_id, tp.artist_id FROM track_participant tp \
             JOIN track t ON t.id = tp.track_id \
             WHERE t.library_id = ? AND t.is_available = 1 AND tp.role = 'artist'",
        )
        .bind(library_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        let mut artists: HashMap<String, Vec<String>> = HashMap::new();
        for (track, artist) in credited {
            artists.entry(track).or_default().push(artist);
        }
        let mut buckets: BTreeMap<(String, String), Vec<(usize, i64)>> = BTreeMap::new();
        for (index, (id, title, duration)) in tracks.iter().enumerate() {
            let mut names = artists.remove(id).unwrap_or_default();
            names.sort();
            let key = (
                waveflow_core::metadata::name_match::normalize_name(title),
                names.join("|"),
            );
            buckets.entry(key).or_default().push((index, *duration));
        }
        let mut grouped = Vec::new();
        let mut group = 0;
        for items in buckets.into_values() {
            let items: Vec<(i64, i64)> = items
                .into_iter()
                .map(|(index, duration)| (index as i64, duration))
                .collect();
            for members in waveflow_core::inventory::chain_by_duration(
                &items,
                waveflow_core::inventory::DURATION_TOLERANCE_MS,
            ) {
                group += 1;
                for member in members {
                    let id = Uuid::parse_str(&tracks[member as usize].0)
                        .map_err(|_| ServiceError::Invalid)?;
                    grouped.push((id, group));
                }
            }
        }
        Ok(grouped)
    }
}
