//! Writing a scanned file into the catalogue: the track row and everything it
//! hangs off — artwork, artists, genres, album, lyrics, credits and events.

use sqlx::{Sqlite, Transaction};
use uuid::Uuid;

use crate::{auth::authentication::now_ms, storage::database::Database};

use super::{
    credit_decisions,
    credits::{credit_names, with_album_artist_sorts},
    display_of,
    identity::{album_artist_display, track_credits, TrackIdentity},
    parse_uuid, record_library_event, track_override_lists, track_pid, write_track_credits,
    LibraryChange,
};

#[derive(Debug, Clone)]
pub struct ArtworkInput {
    pub hash: String,
    pub format: String,
    pub source: String,
    pub byte_size: i64,
}

#[derive(Debug, Clone)]
pub struct CatalogTrackInput {
    pub relative_path: String,
    pub file_size: i64,
    pub modified_at: i64,
    pub quick_hash: String,
    pub full_hash: String,
    pub title: String,
    pub artist: Option<String>,
    /// The multi-valued spellings of the two artist tags, when a file used
    /// them. They win over the singular ones and are never cut: a tagger
    /// writing `ARTISTS` has already said where the boundaries are.
    pub artists: Vec<String>,
    pub album_artists: Vec<String>,
    /// Every other role the file names, as it spelled them.
    pub roles: Vec<(crate::library::tags::Role, Vec<String>)>,
    /// Performer credits, already split into instrument and name.
    pub performer_pairs: Vec<(String, String)>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub is_compilation: bool,
    pub genre: Option<String>,
    pub year: Option<i64>,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub duration_ms: i64,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
    pub bit_depth: Option<i64>,
    pub codec: Option<String>,
    pub musical_key: Option<String>,
    pub tag_rating: Option<i64>,
    /// The MusicBrainz recording identifier: the performance, which is what a
    /// song's `musicBrainzId` means. Release and artist are separate entities
    /// and are kept separate.
    pub musicbrainz_recording_id: Option<String>,
    pub musicbrainz_release_id: Option<String>,
    pub musicbrainz_artist_id: Option<String>,
    /// Decibel gains and linear peaks, as tagged.
    pub replay_gain_track_gain: Option<f64>,
    pub replay_gain_track_peak: Option<f64>,
    pub replay_gain_album_gain: Option<f64>,
    pub replay_gain_album_peak: Option<f64>,
    pub bpm: Option<i64>,
    pub sort_title: Option<String>,
    /// Sort forms of the album title and of the two artist credits, as
    /// tagged. Multi-valued credits carry them joined by the same separator.
    pub sort_album: Option<String>,
    pub sort_album_artist: Option<String>,
    pub sort_artist: Option<String>,
    pub comment: Option<String>,
    /// Multi-valued, split like `artist` and `genre`.
    pub isrc: Option<String>,
    pub moods: Option<String>,
    /// Normalised to `explicit` or `clean`; any other tag value is no value.
    pub explicit_status: Option<String>,
    /// The two release dates, as the file spelled them. Kept as written and
    /// taken apart only at the wire, where OpenSubsonic wants year, month and
    /// day as separate numbers: a tag that names only a year must not be
    /// reported as the first of January.
    pub original_release_date: Option<String>,
    pub release_date: Option<String>,
    /// Multi-valued, split like `moods`.
    pub release_types: Option<String>,
    pub record_labels: Option<String>,
    /// The title of the disc this track sits on. Track-level because an album
    /// has one per disc, which is the whole point of the field.
    pub disc_subtitle: Option<String>,
    pub artwork: Option<ArtworkInput>,
    pub lyrics_hash: String,
    pub lyrics: Vec<crate::library::lyrics::LyricsInput>,
}

#[derive(Debug, Clone)]
pub struct CatalogApply {
    pub input: CatalogTrackInput,
    pub existing_id: Option<Uuid>,
    pub moved: bool,
    /// The device that asked for this, when a client did.
    ///
    /// `None` for a scan, which is nobody's request in particular, and also for
    /// a client that named no device — the header is optional, and a client
    /// that would rather hear its own changes back simply omits it.
    pub origin_device_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Added,
    Updated,
    Moved,
}

impl Database {
    pub async fn apply_catalog_track(
        &self,
        library_id: Uuid,
        scan_id: Uuid,
        input: &CatalogTrackInput,
        existing_id: Option<Uuid>,
        moved: bool,
    ) -> Result<ApplyOutcome, sqlx::Error> {
        let mut outcomes = self
            .apply_catalog_tracks(
                library_id,
                scan_id,
                &[CatalogApply {
                    input: input.clone(),
                    existing_id,
                    moved,
                    origin_device_id: None,
                }],
            )
            .await?;
        Ok(outcomes.pop().expect("single-item catalogue batch"))
    }

    pub async fn apply_catalog_tracks(
        &self,
        library_id: Uuid,
        scan_id: Uuid,
        applies: &[CatalogApply],
    ) -> Result<Vec<ApplyOutcome>, sqlx::Error> {
        if applies.is_empty() {
            return Ok(Vec::new());
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        // Read once per batch, inside the transaction that writes it (RFC-015):
        // a decision taken while a scan runs is either before this batch,
        // which applies it, or after it, in which case the decision's own pass
        // finds what this batch wrote.
        let decisions = credit_decisions(&mut tx, library_id).await?;
        let now = now_ms();
        let mut outcomes = Vec::with_capacity(applies.len());
        for apply in applies {
            outcomes.push(
                Self::apply_catalog_track_in_transaction(
                    &mut tx,
                    self.pid(),
                    library_id,
                    Some(scan_id),
                    apply,
                    &decisions,
                    now,
                )
                .await?,
            );
        }
        tx.commit().await?;
        Ok(outcomes)
    }

    #[allow(clippy::too_many_arguments)]
    /// Reachable from the domain services because clearing a correction has to
    /// land in the same transaction that removes it: re-deriving afterwards
    /// would leave a window where the correction is gone and the rows it wrote
    /// are not.
    /// Writes one track into the catalogue.
    ///
    /// `scan_id` is optional, and `None` has a precise meaning: **this row did
    /// not come from a scan**. It is not "the scan is unknown". A received file
    /// is applied through this same path so that what lands is by construction
    /// what a scan would have written — but no scan walked for it, and saying
    /// otherwise would put a fact in the scan history that never happened.
    ///
    /// The helpers above take a real id because they are the scanner's, and a
    /// scan always has one. This primitive does not: applying to the catalogue
    /// is not intrinsically a scanner operation, and the signature is where
    /// that stops being implied.
    ///
    /// [`Database::mark_unseen_unavailable`] is the other half of this
    /// meaning: a row that never came from a scan must not be swept as one a
    /// scan failed to find.
    pub(crate) async fn apply_catalog_track_in_transaction(
        tx: &mut Transaction<'_, Sqlite>,
        pid: &crate::library::pid::PidSpecs,
        library_id: Uuid,
        scan_id: Option<Uuid>,
        apply: &CatalogApply,
        decisions: &crate::library::credit_split::Decisions,
        now: i64,
    ) -> Result<ApplyOutcome, sqlx::Error> {
        let input = &apply.input;
        let existing_id = apply.existing_id;
        // Credits are derived here rather than by the scanner because this is
        // where identity is written: the test suite applies catalogue rows
        // directly, and reading the tag values one way there and another way
        // in production is how the two drift apart.
        //
        // These are the first step's credits (RFC-015): what the tags say,
        // before any decision. They are stored as they are, and everything an
        // identity hangs off — the album's display string, the relocation
        // hint — reads them rather than what the decisions make of them, so a
        // split never moves an album id.
        let file_credits = with_album_artist_sorts(track_credits(input));
        let moved = apply.moved;
        let track_id = existing_id.unwrap_or_else(Uuid::new_v4);
        // A track that already exists may carry corrections, and a scan must
        // not undo them. Only its own `artist` credits are displaced: a
        // composer or a conductor comes from the file and nobody corrected it.
        let overrides = track_override_lists(tx, track_id).await?;
        // The library's decisions, read by the caller inside this transaction.
        let credits = crate::library::credit_split::resolve(
            &file_credits,
            decisions,
            overrides.artists.as_deref(),
            |name| pid.artist_id(library_id, name.trim()),
        );
        // The display strings follow the correction too, or a track would read
        // one way in a listing and another in its own row.
        // An empty correction says the track credits nobody, which is absence
        // rather than an empty string: `Some("")` would render as a blank name
        // where `None` renders as none at all.
        let artist_display = display_of(overrides.artists.as_deref(), input.artist.as_deref());
        let genre_display = display_of(overrides.genres.as_deref(), input.genre.as_deref());
        let artwork_hash = upsert_artwork(tx, input.artwork.as_ref(), now).await?;
        // The credits arrive already split, already paired with their sort
        // forms and already carrying the album-artist fallback — that is the
        // tag mapper's whole job, and it applies the reference's rules rather
        // than this file's old `;`-only cut. What is left here is turning
        // names into rows.
        let artist_names: Vec<String> = credit_names(&credits, crate::library::tags::Role::Artist);
        let mut artist_ids = Vec::with_capacity(artist_names.len());
        for artist in &artist_names {
            artist_ids.push(upsert_artist(tx, pid, library_id, artist, now).await?);
        }
        let album_artist_names: Vec<String> =
            credit_names(&credits, crate::library::tags::Role::AlbumArtist);
        // The first step's names, not the decisions': the display string is
        // what the album's identity hashes (RFC-015 decision 4).
        let album_artist = album_artist_display(input, &file_credits);
        let mut album_artist_ids = Vec::with_capacity(album_artist_names.len());
        for name in &album_artist_names {
            album_artist_ids.push(upsert_artist(tx, pid, library_id, name, now).await?);
        }
        // The album still names one artist in the frozen `artistId` field, and
        // that is the first credit. The rest reach clients through the album's
        // participants rather than through this column.
        let album_artist_id = album_artist_ids.first().copied();
        let album = upsert_album(
            tx,
            pid,
            library_id,
            input,
            album_artist.as_deref(),
            album_artist_id,
            artwork_hash.as_deref(),
            now,
        )
        .await?;
        let album_id = album.map(|(id, _)| id);
        // RFC-007 left open whether an album needs its own event or whether a
        // client can derive one from the tracks. It cannot, and the desktop
        // named the case: its incremental mirror skips an album whose
        // `song_count` has not moved, so an album that was merely *retagged* —
        // a release date learned, a cover found, an album artist corrected —
        // is invisible to that walk. The only way to catch it without an event
        // is to refetch every album, which is exactly the saving the walk
        // exists for.
        //
        // Emitted only when the row was really written, so a twelve-track album
        // produces one event and not twelve. It carries the origin device of
        // the track that caused it, for the same reason the track's own event
        // does: the client that just wrote must not read its own change back as
        // a discovery.
        if let Some((id, _)) = album.filter(|(_, changed)| *changed) {
            record_library_event(
                tx,
                LibraryChange {
                    library_id,
                    entity_type: "album",
                    entity_id: id,
                    action: "upsert",
                    // Nothing to carry. The track's event carries `full_hash`
                    // because no other surface exposes it; everything about an
                    // album is on the album, which the client refetches on
                    // seeing this.
                    payload: serde_json::json!({}),
                    changed_at: now,
                    origin_device_id: apply.origin_device_id,
                },
            )
            .await?;
        }
        let genres = overrides
            .genres
            .clone()
            .unwrap_or_else(|| split_values(genre_display.as_deref()));
        let mut genre_ids = Vec::with_capacity(genres.len());
        for genre in &genres {
            genre_ids.push(upsert_genre(tx, library_id, genre, now).await?);
        }

        sqlx::query(
            "INSERT INTO track (id, library_id, album_id, artwork_hash, relative_path, file_size, \
               file_modified_at, quick_hash, full_hash, title, album_title, artist_display, genre_display, \
               year, track_number, disc_number, duration_ms, bitrate, sample_rate, channels, bit_depth, \
               codec, musical_key, tag_rating, musicbrainz_recording_id, musicbrainz_release_id, \
               musicbrainz_artist_id, replay_gain_track_gain, replay_gain_track_peak, \
               replay_gain_album_gain, replay_gain_album_peak, bpm, sort_title, sort_album, \
               comment, isrc, \
               moods, explicit_status, \
               lyrics_hash, disc_subtitle, pid, is_available, last_seen_scan_id, \
               created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
                     ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
                     ?, ?, 1, ?, ?, ?) \
             ON CONFLICT (id) DO UPDATE SET album_id=excluded.album_id, artwork_hash=excluded.artwork_hash, \
               relative_path=excluded.relative_path, file_size=excluded.file_size, \
               file_modified_at=excluded.file_modified_at, quick_hash=excluded.quick_hash, \
               full_hash=excluded.full_hash, title=excluded.title, album_title=excluded.album_title, \
               artist_display=excluded.artist_display, genre_display=excluded.genre_display, year=excluded.year, \
               track_number=excluded.track_number, disc_number=excluded.disc_number, duration_ms=excluded.duration_ms, \
               bitrate=excluded.bitrate, sample_rate=excluded.sample_rate, channels=excluded.channels, \
               bit_depth=excluded.bit_depth, codec=excluded.codec, musical_key=excluded.musical_key, \
               tag_rating=excluded.tag_rating, \
               musicbrainz_recording_id=excluded.musicbrainz_recording_id, \
               musicbrainz_release_id=excluded.musicbrainz_release_id, \
               musicbrainz_artist_id=excluded.musicbrainz_artist_id, \
               replay_gain_track_gain=excluded.replay_gain_track_gain, \
               replay_gain_track_peak=excluded.replay_gain_track_peak, \
               replay_gain_album_gain=excluded.replay_gain_album_gain, \
               replay_gain_album_peak=excluded.replay_gain_album_peak, \
               bpm=excluded.bpm, sort_title=excluded.sort_title, sort_album=excluded.sort_album, \
               comment=excluded.comment, \
               isrc=excluded.isrc, moods=excluded.moods, \
               explicit_status=excluded.explicit_status, \
               lyrics_hash=excluded.lyrics_hash, disc_subtitle=excluded.disc_subtitle, \
               pid=excluded.pid, \
               is_available=1, unavailable_since=NULL, purged_at=NULL, \
               last_seen_scan_id=excluded.last_seen_scan_id, \
               updated_at=excluded.updated_at",
        )
        .bind(track_id.to_string()).bind(library_id.to_string())
        .bind(album_id.map(|id| id.to_string())).bind(artwork_hash.as_deref())
        .bind(&input.relative_path).bind(input.file_size).bind(input.modified_at)
        .bind(&input.quick_hash).bind(&input.full_hash).bind(&input.title)
        .bind(input.album.as_deref()).bind(artist_display.as_deref()).bind(genre_display.as_deref())
        .bind(input.year).bind(input.track_number).bind(input.disc_number).bind(input.duration_ms)
        .bind(input.bitrate).bind(input.sample_rate).bind(input.channels).bind(input.bit_depth)
        .bind(input.codec.as_deref()).bind(input.musical_key.as_deref()).bind(input.tag_rating)
        .bind(input.musicbrainz_recording_id.as_deref())
        .bind(input.musicbrainz_release_id.as_deref())
        .bind(input.musicbrainz_artist_id.as_deref())
        .bind(input.replay_gain_track_gain).bind(input.replay_gain_track_peak)
        .bind(input.replay_gain_album_gain).bind(input.replay_gain_album_peak)
        .bind(input.bpm).bind(input.sort_title.as_deref()).bind(input.sort_album.as_deref())
        .bind(input.comment.as_deref())
        .bind(input.isrc.as_deref())
        .bind(input.moods.as_deref()).bind(input.explicit_status.as_deref())
        .bind(&input.lyrics_hash)
        .bind(input.disc_subtitle.as_deref())
        .bind(track_pid(pid, library_id, input).to_string())
        .bind(scan_id.map(|id| id.to_string())).bind(now).bind(now)
        .execute(&mut **tx).await?;

        sqlx::query("DELETE FROM track_lyrics WHERE track_id = ?")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        for (position, lyrics) in input.lyrics.iter().enumerate() {
            sqlx::query(
                "INSERT INTO track_lyrics \
                 (track_id, library_id, position, source, lang, synced, content) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(track_id.to_string())
            .bind(library_id.to_string())
            .bind(position as i64)
            .bind(lyrics.source)
            .bind(&lyrics.lang)
            .bind(i64::from(lyrics.synced))
            .bind(&lyrics.content)
            .execute(&mut **tx)
            .await?;
        }

        write_track_credits(tx, library_id, track_id, &file_credits).await?;
        // Every credit the file names, not just its artists. The delete comes
        // first for the reason the reference gives: an artist who was both an
        // album artist and a composer, and is now only a composer, has to lose
        // the row that said otherwise.
        sqlx::query("DELETE FROM track_participant WHERE track_id = ?")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        for credit in &credits {
            let artist_id = match credit.role {
                crate::library::tags::Role::Artist => artist_ids
                    .get(credit.position)
                    .copied()
                    .expect("every artist credit was upserted above"),
                crate::library::tags::Role::AlbumArtist => album_artist_ids
                    .get(credit.position)
                    .copied()
                    .expect("every album-artist credit was upserted above"),
                _ => upsert_artist(tx, pid, library_id, &credit.name, now).await?,
            };
            let sort_name = credit.sort_name.clone();
            // A file can credit the same artist twice under one role — two
            // spellings folding onto one row — and the second insert would
            // then collide on the key. The first credit wins, as it does in
            // tag order.
            sqlx::query(
                "INSERT INTO track_participant \
                   (track_id, artist_id, library_id, role, sub_role, position, sort_name) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (track_id, artist_id, role, sub_role) DO NOTHING",
            )
            .bind(track_id.to_string())
            .bind(artist_id.to_string())
            .bind(library_id.to_string())
            .bind(credit.role.as_str())
            .bind(&credit.sub_role)
            .bind(credit.position as i64)
            .bind(sort_name)
            .execute(&mut **tx)
            .await?;
        }
        sqlx::query("DELETE FROM track_genre WHERE track_id = ?")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        // A file may name one genre twice, or in two spellings canonicalisation
        // folds together; the pair is the key, so the repeat is not an error.
        for genre_id in &genre_ids {
            sqlx::query(
                "INSERT INTO track_genre (track_id, genre_id, library_id) VALUES (?, ?, ?) \
                 ON CONFLICT (track_id, genre_id) DO NOTHING",
            )
            .bind(track_id.to_string())
            .bind(genre_id.to_string())
            .bind(library_id.to_string())
            .execute(&mut **tx)
            .await?;
        }
        sqlx::query("DELETE FROM track_fts WHERE track_id = ?")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        // Every participant's name, not just the track artist's. The artist
        // index lists only artists an album is credited to, and the argument
        // for that is that a composer stays reachable by search — which is
        // only true if the search can see the composer's name.
        let searchable = {
            let mut names: Vec<&str> = credits.iter().map(|credit| credit.name.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            // The effective display string, not the file's. Pushing
            // `input.artist` here would leave a corrected track findable under
            // the very name it was corrected away from — the same failure the
            // title had, one column over.
            if let Some(display) = artist_display.as_deref() {
                names.push(display);
            }
            names.join(" ")
        };
        sqlx::query("INSERT INTO track_fts (track_id, library_id, title, album, artists, genres) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(track_id.to_string()).bind(library_id.to_string())
            .bind(overrides.title.as_deref().unwrap_or(input.title.as_str()))
            .bind(input.album.as_deref()).bind(&searchable).bind(genre_display.as_deref())
            .execute(&mut **tx).await?;
        // The hash travels with the event because nothing else carries it. A
        // client that confirmed a link against this track has no other way to
        // learn the file was retagged outside the API — the track keeps its id
        // while its bytes move, and the scan is the only witness.
        record_library_event(
            tx,
            LibraryChange {
                library_id,
                entity_type: "track",
                entity_id: track_id,
                action: "upsert",
                payload: serde_json::json!({ "full_hash": input.full_hash }),
                changed_at: now,
                origin_device_id: apply.origin_device_id,
            },
        )
        .await?;
        Ok(if existing_id.is_none() {
            ApplyOutcome::Added
        } else if moved {
            ApplyOutcome::Moved
        } else {
            ApplyOutcome::Updated
        })
    }
}

async fn upsert_artwork(
    tx: &mut Transaction<'_, Sqlite>,
    artwork: Option<&ArtworkInput>,
    now: i64,
) -> Result<Option<String>, sqlx::Error> {
    let Some(artwork) = artwork else {
        return Ok(None);
    };
    sqlx::query("INSERT INTO artwork (hash, format, source, byte_size, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (hash) DO NOTHING")
        .bind(&artwork.hash).bind(&artwork.format).bind(&artwork.source).bind(artwork.byte_size).bind(now)
        .execute(&mut **tx).await?;
    Ok(Some(artwork.hash.clone()))
}

/// Upserts one artist row, whose id is the identity of its name.
///
/// The name reaches the row twice: as written, which is what a client renders,
/// and folded through `canonical_name`, which is what search matches on. The
/// id is neither — it is the identity hash, so two spellings that differ by a
/// real character are two artists, as they are in the reference.
pub(super) async fn upsert_artist(
    tx: &mut Transaction<'_, Sqlite>,
    pid: &crate::library::pid::PidSpecs,
    library: Uuid,
    name: &str,
    now: i64,
) -> Result<Uuid, sqlx::Error> {
    let name = name.trim();
    let id = pid.artist_id(library, name);
    let canonical = waveflow_core::scanner::canonical_name(name);
    sqlx::query(
        "INSERT INTO artist (id, library_id, name, canonical_name, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT (id) DO UPDATE SET name=excluded.name, \
         canonical_name=excluded.canonical_name, updated_at=excluded.updated_at",
    )
    .bind(id.to_string())
    .bind(library.to_string())
    .bind(name)
    .bind(canonical)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

pub(super) async fn upsert_genre(
    tx: &mut Transaction<'_, Sqlite>,
    library: Uuid,
    name: &str,
    now: i64,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    let canonical = waveflow_core::scanner::canonical_name(name);
    let value: String = sqlx::query_scalar("INSERT INTO genre (id, library_id, name, canonical_name, created_at) VALUES (?, ?, ?, ?, ?) \
        ON CONFLICT (library_id, canonical_name) DO UPDATE SET name=excluded.name RETURNING id")
        .bind(id.to_string()).bind(library.to_string()).bind(name.trim()).bind(canonical).bind(now)
        .fetch_one(&mut **tx).await?;
    parse_uuid(value)
}

#[allow(clippy::too_many_arguments)]
/// Upserts one album row, whose id is the identity of the tags that name it.
///
/// The identity key this replaces embedded the row id of the album's first
/// credited artist, which is what made "Live" by `A; B` and "Live" by `A; C`
/// answer the same key — and what needed a special case to tell them apart.
/// The engine hashes the credit as written, so the two are simply different
/// and nothing has to notice.
async fn upsert_album(
    tx: &mut Transaction<'_, Sqlite>,
    pid: &crate::library::pid::PidSpecs,
    library: Uuid,
    input: &CatalogTrackInput,
    album_artist: Option<&str>,
    album_artist_id: Option<Uuid>,
    artwork: Option<&str>,
    now: i64,
) -> Result<Option<(Uuid, bool)>, sqlx::Error> {
    let Some(title) = input.album.as_deref().filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    let canonical = waveflow_core::scanner::canonical_name(title);
    let id = pid.album_id(
        library,
        &TrackIdentity::new(input, album_artist.unwrap_or_default()),
    );
    let written = sqlx::query(
        "INSERT INTO album (id, library_id, title, canonical_title, album_artist_id, \
           album_artist_name, is_compilation, year, artwork_hash, \
           original_release_date, release_date, release_types, record_labels, \
           created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (id) DO UPDATE SET title=excluded.title, \
           canonical_title=excluded.canonical_title, \
           album_artist_id=excluded.album_artist_id, \
           album_artist_name=excluded.album_artist_name, \
           is_compilation=excluded.is_compilation, \
           year=COALESCE(excluded.year, album.year), \
           artwork_hash=COALESCE(excluded.artwork_hash, album.artwork_hash), \
           original_release_date=COALESCE(excluded.original_release_date, album.original_release_date), \
           release_date=COALESCE(excluded.release_date, album.release_date), \
           release_types=COALESCE(excluded.release_types, album.release_types), \
           record_labels=COALESCE(excluded.record_labels, album.record_labels), \
           updated_at=excluded.updated_at \
         WHERE album.title IS NOT excluded.title \
            OR album.canonical_title IS NOT excluded.canonical_title \
            OR album.album_artist_id IS NOT excluded.album_artist_id \
            OR album.album_artist_name IS NOT excluded.album_artist_name \
            OR album.is_compilation IS NOT excluded.is_compilation \
            OR album.year IS NOT COALESCE(excluded.year, album.year) \
            OR album.artwork_hash IS NOT COALESCE(excluded.artwork_hash, album.artwork_hash) \
            OR album.original_release_date \
                 IS NOT COALESCE(excluded.original_release_date, album.original_release_date) \
            OR album.release_date IS NOT COALESCE(excluded.release_date, album.release_date) \
            OR album.release_types IS NOT COALESCE(excluded.release_types, album.release_types) \
            OR album.record_labels IS NOT COALESCE(excluded.record_labels, album.record_labels) \
         RETURNING id",
    )
    .bind(id.to_string())
    .bind(library.to_string())
    .bind(title.trim())
    .bind(canonical)
    .bind(album_artist_id.map(|id| id.to_string()))
    .bind(album_artist)
    .bind(i64::from(input.is_compilation))
    .bind(input.year)
    .bind(artwork)
    .bind(input.original_release_date.as_deref())
    .bind(input.release_date.as_deref())
    .bind(input.release_types.as_deref())
    .bind(input.record_labels.as_deref())
    .bind(now)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await?;
    // A row came back only if one was written: inserted, or updated because the
    // `WHERE` above found a difference. Twelve tracks of one album run this
    // twelve times and the first is the only one that writes, which is what
    // makes the event below one per album rather than one per track.
    Ok(Some((id, written.is_some())))
}

fn split_values(raw: Option<&str>) -> Vec<String> {
    raw.into_iter()
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}
