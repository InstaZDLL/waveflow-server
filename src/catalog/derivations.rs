//! What the catalogue derives from its own contents after a scan: sort names,
//! MusicBrainz identifiers, album credits, artist counts and the artist index.

use sqlx::{Sqlite, Transaction};
use uuid::Uuid;

use crate::database::Database;

impl Database {
    /// Re-derives `album.sort_name` and `artist.sort_name` from the tags the
    /// library's available tracks still carry.
    ///
    /// Sibling of [`Self::consolidate_catalog_derivations`] and run beside it, for
    /// the same reason and by the same rule: a value derived from tags has to
    /// follow the tags, including when they go away. Writing the sort name
    /// during the per-track upsert could not do that — the row is rewritten
    /// once per track of every album the artist appears on, so a file carrying
    /// no tag had to be prevented from erasing what a sibling supplied, and
    /// that preservation outlived the tag itself.
    ///
    /// The majority wins, ties broken deterministically, so two scans of
    /// unchanged files answer the same thing. An album takes the sort title
    /// its tracks agree on; an artist takes the sort form its credits agree
    /// on, from `track_artist` where the joined tag was already paired with
    /// the joined credit. An album artist credited on none of the tracks
    /// therefore has no sort form of its own — the catalogue holds no tag
    /// that is about it alone.
    pub async fn consolidate_sort_names(&self, library_id: Uuid) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        Self::consolidate_sort_names_in(&mut tx, library_id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// [`Self::consolidate_sort_names`] inside a transaction the caller holds,
    /// under the writer gate it already took: a credit decision re-derives in
    /// the transaction that records it, so the two land or fail together.
    pub(crate) async fn consolidate_sort_names_in(
        tx: &mut Transaction<'_, Sqlite>,
        library_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE album SET sort_name = ( \
               SELECT t.sort_album FROM track t \
               WHERE t.album_id = album.id AND t.is_available = 1 \
                 AND t.sort_album IS NOT NULL \
               GROUP BY t.sort_album \
               ORDER BY COUNT(*) DESC, t.sort_album \
               LIMIT 1) \
             WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE artist SET sort_name = ( \
               SELECT tp.sort_name FROM track_participant tp \
               JOIN track t ON t.id = tp.track_id \
               WHERE tp.artist_id = artist.id AND t.is_available = 1 \
                 AND tp.sort_name IS NOT NULL \
               GROUP BY tp.sort_name \
               ORDER BY COUNT(*) DESC, tp.sort_name \
               LIMIT 1) \
             WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Everything the catalogue derives from its own contents, after a scan.
    ///
    /// It was named for the MusicBrainz identifiers alone, which stopped being
    /// true: it also rebuilds each album's credits from its tracks', recounts
    /// what every artist has to show for each capacity, and drops the artist
    /// rows nothing credits any more. All of it is a majority or a union over
    /// the tracks that are still available, so all of it has to run after
    /// `mark_unseen_unavailable` — a file that vanished must stop voting first.
    ///
    /// A test that builds a catalogue by applying tracks directly runs this
    /// too, for the same reason the scanner does: without it the album has no
    /// credits and no artist has a count.
    ///
    /// Recomputes the derived MusicBrainz identifier of every album and artist
    /// in one library from the tracks that currently back them.
    ///
    /// Tracks of one album routinely disagree: a release id is written per file
    /// by whatever tagger touched it, and a library assembled over years holds
    /// files tagged against different releases of the same record. The album's
    /// identifier is therefore the one most of its available tracks agree on,
    /// not the one the last file scanned happened to carry.
    ///
    /// Ties are broken by the earliest disc and track number, zero-padded into
    /// a single sortable key so `MIN` compares the pair rather than each half
    /// independently, and finally by the identifier itself. A tie means the
    /// tags genuinely split the album down the middle; what matters is that two
    /// scans of unchanged files answer the same thing, so nothing flaps.
    ///
    /// An artist takes the identifier from the tracks it is the *primary*
    /// credit of (`track_artist.position = 0`), because `musicbrainz_artist_id`
    /// is one value on a track that may credit several artists, and only the
    /// first credit is the one the tag is about.
    ///
    /// Albums and artists whose tracks carry no identifier are set back to
    /// `NULL` rather than left alone: this runs after every scan, so a tag
    /// removed from the files has to disappear from the catalogue too.
    pub async fn consolidate_catalog_derivations(
        &self,
        library_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        Self::consolidate_catalog_derivations_in(&mut tx, library_id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// [`Self::consolidate_catalog_derivations`] inside a transaction the caller
    /// holds, under the writer gate it already took, for the same reason as
    /// [`Self::consolidate_sort_names_in`].
    pub(crate) async fn consolidate_catalog_derivations_in(
        tx: &mut Transaction<'_, Sqlite>,
        library_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE album SET musicbrainz_id = ( \
               SELECT t.musicbrainz_release_id FROM track t \
               WHERE t.album_id = album.id AND t.is_available = 1 \
                 AND t.musicbrainz_release_id IS NOT NULL \
               GROUP BY t.musicbrainz_release_id \
               ORDER BY COUNT(*) DESC, \
                        MIN(printf('%06d%06d', COALESCE(t.disc_number, 1), \
                                   COALESCE(t.track_number, 0))), \
                        t.musicbrainz_release_id \
               LIMIT 1) \
             WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE artist SET musicbrainz_id = ( \
               SELECT t.musicbrainz_artist_id FROM track t \
               JOIN track_participant tp ON tp.track_id = t.id AND tp.role = 'artist' AND tp.position = 0 \
               WHERE tp.artist_id = artist.id AND t.is_available = 1 \
                 AND t.musicbrainz_artist_id IS NOT NULL \
               GROUP BY t.musicbrainz_artist_id \
               ORDER BY COUNT(*) DESC, t.musicbrainz_artist_id \
               LIMIT 1) \
             WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .execute(&mut **tx)
        .await?;
        // The album's own credits, derived here rather than written per track.
        // A track-by-track write would be a last-writer-wins race between
        // files of one album that disagree; deriving after availability is
        // settled makes the answer the same whatever order the scan ran in.
        sqlx::query("DELETE FROM album_participant WHERE library_id = ?")
            .bind(library_id.to_string())
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "INSERT INTO album_participant \
               (album_id, artist_id, library_id, role, sub_role, position) \
             SELECT t.album_id, tp.artist_id, t.library_id, tp.role, tp.sub_role, \
                    ROW_NUMBER() OVER ( \
                      PARTITION BY t.album_id, tp.role \
                      ORDER BY MIN(tp.position), tp.sub_role, tp.artist_id) - 1 \
               FROM track t \
               JOIN track_participant tp ON tp.track_id = t.id \
              WHERE t.library_id = ? AND t.is_available = 1 AND t.album_id IS NOT NULL \
              GROUP BY t.album_id, tp.artist_id, tp.role, tp.sub_role",
        )
        .bind(library_id.to_string())
        .execute(&mut **tx)
        .await?;
        // What each artist has to show for each capacity. `albumCount` used to
        // be a count over the album's single artist column, which could only
        // ever see the first credit; the roles an artist appears in are now
        // simply the keys of this table.
        sqlx::query("DELETE FROM artist_role_stats WHERE library_id = ?")
            .bind(library_id.to_string())
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "INSERT INTO artist_role_stats \
               (artist_id, library_id, role, album_count, song_count) \
             SELECT tp.artist_id, t.library_id, tp.role, \
                    COUNT(DISTINCT t.album_id), COUNT(DISTINCT t.id) \
               FROM track_participant tp \
               JOIN track t ON t.id = tp.track_id \
              WHERE t.library_id = ? AND t.is_available = 1 \
              GROUP BY tp.artist_id, t.library_id, tp.role",
        )
        .bind(library_id.to_string())
        .execute(&mut **tx)
        .await?;
        // An album no track depends on is no longer in the catalogue. This
        // matters most on the scan that follows the identity migration: the
        // rows kept by the migration hold their old identifiers, the rescan
        // mints derived ones and moves every track across, and without this
        // the library would answer with each album twice — once full, once
        // empty. It runs before the artist sweep because `album_artist_id`
        // restricts on delete, so a ghost album would hold its artist hostage.
        sqlx::query(
            "DELETE FROM album WHERE library_id = ? \
               AND NOT EXISTS (SELECT 1 FROM track t WHERE t.album_id = album.id)",
        )
        .bind(library_id.to_string())
        .execute(&mut **tx)
        .await?;
        // An artist row outlives the tag that created it: rename a credit and
        // the old name stays in the index forever. An artist credited on
        // nothing is no longer part of the catalogue, so it goes with the same
        // scan that made it unreferenced. Both relations are checked, because
        // an album artist need not be credited on any of the album's tracks.
        sqlx::query(
            "DELETE FROM artist WHERE library_id = ? \
               AND NOT EXISTS (SELECT 1 FROM track_participant tp WHERE tp.artist_id = artist.id) \
               AND NOT EXISTS (SELECT 1 FROM album_participant ap WHERE ap.artist_id = artist.id) \
               AND NOT EXISTS (SELECT 1 FROM album al WHERE al.album_artist_id = artist.id)",
        )
        .bind(library_id.to_string())
        .execute(&mut **tx)
        .await?;
        // The artist search index, rewritten whole rather than kept in step
        // row by row. An artist row is touched once per track of every album
        // it appears on, and a rename would leave the old name searchable
        // forever unless something swept it — the same reasoning that made
        // the sort names a pass instead of a write. Last in this
        // transaction because it indexes the artists the sweep above left
        // standing, under the sort names settled before this pass ran.
        sqlx::query("DELETE FROM artist_fts WHERE library_id = ?")
            .bind(library_id.to_string())
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "INSERT INTO artist_fts (artist_id, library_id, name, sort_name) \
             SELECT id, library_id, name, COALESCE(sort_name, '') \
               FROM artist WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}
