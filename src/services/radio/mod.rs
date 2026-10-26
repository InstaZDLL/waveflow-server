//! Internet radio stations.
//!
//! One list per account; see [`super`] for the shared projections, which this
//! module does not use — a station is not a catalogue row and has no library to
//! be filtered against. Its tenancy is the `user_id` column, named in every
//! statement here.
//!
//! **The server never fetches a station's address.** It validates it on the way
//! in, stores it, and hands it back to whichever client asked. That is the whole
//! security answer for this feature: a station is a URL supplied by a user, so
//! anything the server fetched on its own behalf would be a request forger
//! reaching whatever the server can reach — the private network it runs in
//! included. Deciding not to proxy is also what makes the feature small.
//!
//! **The image is the one address it does fetch, once.** A station may carry an
//! `image_url`, and the web client cannot load it from its host (RFC-019's
//! `img-src`, and the listener's address that would leak with it). So the
//! server fetches it when an account sets or changes it — never on a timer, and
//! never while anybody listens — through the rules a podcast cover is fetched
//! under ([`crate::remote::portrait::fetch_followed`]): public addresses only, checked
//! and pinned, redirects followed by hand and each hop checked, 10 MiB, and the
//! bytes sniffed as JPEG, PNG or WebP. Only the image comes back, and only into
//! the artwork store; a failure keeps the address and shows no image, and the
//! next save that carries the address tries again.
//!
//! **Nothing is fetched for a write that is refused.** Every check that can
//! refuse a write — the address rules, the per-account ceiling, the station
//! being this account's — runs and the row is committed *before* the image is
//! fetched; the fetched image is then attached by a second write that names the
//! station, its account and the address it was fetched for. An account at its
//! ceiling, or naming a station that is not its own, makes the server contact
//! nobody and write nothing. And an account may start at most
//! [`super::remote_artwork::STATION_IMAGE_FETCHES_PER_HOUR`] image fetches an
//! hour, so editing a station in a loop cannot turn the server into a
//! downloader: past it, the station is saved without an image, and a later
//! save retries it.

use super::remote_artwork::record_artwork;
use super::*;

/// What a write does to a station's image.
#[derive(Debug, Clone, Copy)]
pub enum StationImage<'a> {
    /// Leaves it as it is: what the Subsonic writes do, since the protocol has
    /// no image parameter, and what a native write that does not mention the
    /// field does.
    Keep,
    /// Sets it, or clears it with `None` or a blank value.
    Set(Option<&'a str>),
}

macro_rules! station_select {
    () => {
        "SELECT id, name, stream_url, homepage_url, image_url, artwork_hash, created_at, \
                updated_at FROM radio_station "
    };
}

/// How many stations one account may keep.
///
/// Not a protocol limit — there is none — but a write surface a client drives
/// needs a ceiling, and a list a person scrolls has a useful size far below
/// this. The refusal is `Conflict`: 409 natively, and error code 0 on the
/// facade, which has no code for a full collection. Either way the request was
/// well formed and the list is full.
const MAX_RADIO_STATIONS: i64 = 500;

/// Longest station name, in characters rather than bytes, so the bound a caller
/// is told about is the one it can count.
const MAX_RADIO_NAME_CHARS: usize = 200;

/// Longest URL, in characters. Well above anything a stream address needs, and
/// below what a database row should be asked to carry on a client's word.
const MAX_RADIO_URL_CHARS: usize = 2_048;

impl DomainServices {
    /// The account's stations, by name.
    ///
    /// `name` is `NOT NULL`, so there is no `NULLS LAST` to write here: the
    /// ordering has no missing value to place. `id` breaks the tie, so two
    /// stations sharing a name keep a stable order between calls.
    pub async fn radio_stations(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<RadioStationItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        let rows = sqlx::query(concat!(
            station_select!(),
            "WHERE user_id=? ORDER BY name COLLATE NOCASE, id"
        ))
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        rows.into_iter().map(radio_station_from_row).collect()
    }

    /// One station, by id. A station belonging to another account answers like
    /// one that does not exist.
    pub async fn radio_station(
        &self,
        user_id: Uuid,
        station_id: Uuid,
    ) -> Result<RadioStationItem, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        let row = sqlx::query(concat!(station_select!(), "WHERE user_id=? AND id=?"))
            .bind(user_id.to_string())
            .bind(station_id.to_string())
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(ServiceError::NotFound)?;
        radio_station_from_row(row)
    }

    /// Adds a station to the account's list.
    ///
    /// Nothing is deduplicated. Two rows may carry the same address under two
    /// names, because `updateInternetRadioStation` has no conflict code to
    /// report with and a client told "failed" would have nothing to act on.
    ///
    /// An `image_url` is fetched once the station is stored, never before: a
    /// write refused for the ceiling has contacted nobody. A station whose
    /// image cannot be fetched is kept, with the address and without the
    /// image.
    pub async fn create_radio_station(
        &self,
        user_id: Uuid,
        name: &str,
        stream_url: &str,
        homepage_url: Option<&str>,
        image_url: Option<&str>,
    ) -> Result<RadioStationItem, ServiceError> {
        let name = validated_name(name)?;
        let stream_url = validated_url(stream_url)?;
        let homepage_url = validated_homepage(homepage_url)?;
        let image_url = validated_homepage(image_url)?;
        let id = Uuid::new_v4();
        let now = now_ms();
        {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            // **Two locks make the count safe**, and they cover different
            // writers. The writer gate serialises this process's mutations, so
            // two creations here cannot both read 499 and both insert.
            // `begin_write` opens with `BEGIN IMMEDIATE`, so SQLite's write
            // lock is held before the `SELECT COUNT(*)` runs: a writer that
            // bypasses the gate — another process on the same file, say —
            // cannot insert between the count and the insert either.
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM radio_station WHERE user_id=?")
                    .bind(user_id.to_string())
                    .fetch_one(&mut *tx)
                    .await?;
            if count >= MAX_RADIO_STATIONS {
                tx.rollback().await?;
                return Err(ServiceError::Conflict);
            }
            sqlx::query(
                "INSERT INTO radio_station \
                   (id, user_id, name, stream_url, homepage_url, image_url, artwork_hash, \
                    created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?)",
            )
            .bind(id.to_string())
            .bind(user_id.to_string())
            .bind(&name)
            .bind(&stream_url)
            .bind(homepage_url.as_deref())
            .bind(image_url.as_deref())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        if let Some(url) = &image_url {
            self.attach_station_image(user_id, id, url).await?;
        }
        self.radio_station(user_id, id).await
    }

    /// Replaces a station's three fields, and its image when told to.
    ///
    /// A replacement and not a patch, on both surfaces: the protocol's
    /// `updateInternetRadioStation` requires the name and the address every
    /// time, so omitting the homepage there clears it. A native `PATCH` that
    /// kept the old value instead would make the same call mean two different
    /// things depending on which door it came through.
    ///
    /// The image is the exception, and for the same reason turned around: the
    /// protocol has no image parameter, so a Subsonic edit says nothing about
    /// it and [`StationImage::Keep`] leaves it alone — otherwise renaming a
    /// station in any Subsonic client would take its image away. An address
    /// that is the one already stored, with its image in the store, is not
    /// fetched again; any other is, once the replacement is committed — so a
    /// station that is not this account's is 404 before anything is
    /// contacted. A failure stores the address with no image, and saving the
    /// station again is how a fetch is retried.
    pub async fn update_radio_station(
        &self,
        user_id: Uuid,
        station_id: Uuid,
        name: &str,
        stream_url: &str,
        homepage_url: Option<&str>,
        image: StationImage<'_>,
    ) -> Result<RadioStationItem, ServiceError> {
        let name = validated_name(name)?;
        let stream_url = validated_url(stream_url)?;
        let homepage_url = validated_homepage(homepage_url)?;
        let image = match image {
            StationImage::Keep => None,
            StationImage::Set(value) => Some(validated_homepage(value)?),
        };
        let now = now_ms();
        // The address to fetch once the replacement is in, and the hash kept
        // from before when the address did not change.
        let (due, kept) = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let held: Option<(Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT image_url, artwork_hash FROM radio_station WHERE user_id=? AND id=?",
            )
            .bind(user_id.to_string())
            .bind(station_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
            let Some((held_url, held_hash)) = held else {
                tx.rollback().await?;
                return Err(ServiceError::NotFound);
            };
            let plan = match image {
                None => {
                    sqlx::query(
                        "UPDATE radio_station SET name=?, stream_url=?, homepage_url=?, \
                           updated_at=? WHERE user_id=? AND id=?",
                    )
                    .bind(&name)
                    .bind(&stream_url)
                    .bind(homepage_url.as_deref())
                    .bind(now)
                    .bind(user_id.to_string())
                    .bind(station_id.to_string())
                    .execute(&mut *tx)
                    .await?;
                    (None, None)
                }
                Some(image_url) => {
                    let kept = held_hash.filter(|_| image_url.is_some() && image_url == held_url);
                    sqlx::query(
                        "UPDATE radio_station SET name=?, stream_url=?, homepage_url=?, \
                           image_url=?, artwork_hash=?, updated_at=? \
                         WHERE user_id=? AND id=?",
                    )
                    .bind(&name)
                    .bind(&stream_url)
                    .bind(homepage_url.as_deref())
                    .bind(image_url.as_deref())
                    .bind(kept.as_deref())
                    .bind(now)
                    .bind(user_id.to_string())
                    .bind(station_id.to_string())
                    .execute(&mut *tx)
                    .await?;
                    (image_url, kept)
                }
            };
            tx.commit().await?;
            plan
        };
        if let Some(url) = due {
            // Kept from before only while its file is still in the store; a
            // file the sweep's window carried off is fetched again.
            let held = match &kept {
                Some(hash) => self.artwork_file_present(hash).await,
                None => false,
            };
            if !held {
                self.attach_station_image(user_id, station_id, &url).await?;
            }
        }
        self.radio_station(user_id, station_id).await
    }

    /// Removes a station.
    ///
    /// Unlike a bookmark, a station is named by an identifier rather than by
    /// the track it sits on, so removing one that is not there is not the same
    /// request answered twice — it names something that does not exist, and the
    /// account cannot tell that from one belonging to somebody else.
    ///
    /// Its image goes with it: nothing names the hash any more, and the
    /// artwork sweep collects it.
    pub async fn delete_radio_station(
        &self,
        user_id: Uuid,
        station_id: Uuid,
    ) -> Result<(), ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut connection = self.db.pool().acquire().await?;
        let deleted = sqlx::query("DELETE FROM radio_station WHERE user_id=? AND id=?")
            .bind(user_id.to_string())
            .bind(station_id.to_string())
            .execute(&mut *connection)
            .await?;
        if deleted.rows_affected() == 0 {
            return Err(ServiceError::NotFound);
        }
        Ok(())
    }

    /// Fetches the image of a station already stored, and attaches it.
    ///
    /// Called only after the write that set the address has committed. The
    /// attaching write names the station, its account and the address the
    /// image was fetched for, so an image that arrives after the station was
    /// deleted, or after its address changed again, attaches to nothing — its
    /// file is then named by no row, and the sweep collects it.
    ///
    /// Within the account's hourly budget only: past it, nothing is fetched and
    /// the station stays without an image until a later save. Plain http is
    /// allowed: a station's own logo is as often served that way as its
    /// stream is, and the address rules apply either way.
    async fn attach_station_image(
        &self,
        user_id: Uuid,
        station_id: Uuid,
        url: &str,
    ) -> Result<(), ServiceError> {
        if !self.remote_artwork.enabled {
            return Ok(());
        }
        if !self.station_images.take(user_id, now_ms()) {
            tracing::warn!(
                subject = "station image",
                reason = "hourly budget spent",
                "remote artwork not fetched"
            );
            return Ok(());
        }
        let Some(fetched) = self.fetch_remote_artwork(url, true, "station image").await else {
            return Ok(());
        };
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        record_artwork(&mut tx, &fetched, "station", now).await?;
        sqlx::query(
            "UPDATE radio_station SET artwork_hash=? WHERE user_id=? AND id=? AND image_url=?",
        )
        .bind(&fetched.hash)
        .bind(user_id.to_string())
        .bind(station_id.to_string())
        .bind(url)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

fn radio_station_from_row(row: sqlx::sqlite::SqliteRow) -> Result<RadioStationItem, ServiceError> {
    Ok(RadioStationItem {
        id: parse_uuid(row.try_get("id")?)?,
        name: row.try_get("name")?,
        stream_url: row.try_get("stream_url")?,
        homepage_url: row.try_get("homepage_url")?,
        image_url: row.try_get("image_url")?,
        artwork_hash: row.try_get("artwork_hash")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Trimmed, non-empty, bounded.
///
/// A name made only of spaces is refused rather than stored: it would list as a
/// blank row the account could not tell apart from another blank row.
fn validated_name(name: &str) -> Result<String, ServiceError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_RADIO_NAME_CHARS {
        return Err(ServiceError::Invalid);
    }
    Ok(name.to_owned())
}

/// The optional homepage, where a blank value means "none".
///
/// A field left empty in a form arrives as `homepageUrl=` on the facade and as
/// `"homepage_url": ""` natively, and in both cases the sender means the
/// station has no page — not that it has one spelled with zero characters.
/// Refusing it would make clearing the field an error on the only surface
/// where a station is edited, and the two doors would disagree about it as
/// well: the web client already sends `null` for an empty box, so a Subsonic
/// client would have been the only one refused. The address itself is required
/// and has no such reading, so an empty `streamUrl` stays invalid.
fn validated_homepage(value: Option<&str>) -> Result<Option<String>, ServiceError> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(validated_url)
        .transpose()
}

/// Absolute, and `http` or `https`.
///
/// Stored as typed rather than as `Url` re-serialises it: a client that sends
/// `https://example.org` reads back what it sent instead of a trailing slash it
/// did not write, and a station address is handed to a player rather than
/// compared against anything.
///
/// The scheme restriction is the point. Without it the column would accept
/// `file:///etc/passwd` or a `data:` payload, and the only reason that is
/// harmless today is that nothing here dereferences it — a feature added later
/// that did would inherit a table full of addresses nobody validated.
///
/// **No separate host check, measured rather than assumed.** `Url::parse`
/// refuses `http://` and `http://:8000/x` outright with "empty host", and an
/// `http` URL cannot be cannot-be-a-base, so once the scheme is one of these
/// two there is no hostless URL left to reject. If this list ever admits a
/// non-special scheme — `mms://`, say, which parses with no host at all — the
/// check has to come back with it.
fn validated_url(value: &str) -> Result<String, ServiceError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_RADIO_URL_CHARS {
        return Err(ServiceError::Invalid);
    }
    let url = url::Url::parse(value).map_err(|_| ServiceError::Invalid)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ServiceError::Invalid);
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests;
