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
//! under ([`crate::portrait::fetch_followed`]): public addresses only, checked
//! and pinned, redirects followed by hand and each hop checked, 10 MiB, and the
//! bytes sniffed as JPEG, PNG or WebP. Only the image comes back, and only into
//! the artwork store; a failure keeps the address and shows no image, and the
//! next save that carries the address tries again.

use super::remote_artwork::{record_artwork, FetchedArtwork};
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

/// What a write stores of the image, decided before the writer gate.
enum ImageWrite {
    Keep,
    Store {
        image_url: Option<String>,
        artwork_hash: Option<String>,
        fetched: Option<FetchedArtwork>,
    },
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
    /// An `image_url` is fetched here, before the writer gate, and a station
    /// whose image cannot be fetched is created all the same, with the address
    /// and without the image.
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
        let fetched = match &image_url {
            Some(url) => self.fetch_station_image(url).await,
            None => None,
        };
        let id = Uuid::new_v4();
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        // **Two locks make the count safe**, and they cover different writers.
        // The writer gate serialises this process's mutations, so two creations
        // here cannot both read 499 and both insert. `begin_write` opens with
        // `BEGIN IMMEDIATE`, so SQLite's write lock is held before the
        // `SELECT COUNT(*)` runs: a writer that bypasses the gate — another
        // process on the same file, say — cannot insert between the count and
        // the insert either.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM radio_station WHERE user_id=?")
            .bind(user_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        if count >= MAX_RADIO_STATIONS {
            tx.rollback().await?;
            return Err(ServiceError::Conflict);
        }
        if let Some(fetched) = &fetched {
            record_artwork(&mut tx, fetched, "station", now).await?;
        }
        let artwork_hash = fetched.map(|fetched| fetched.hash);
        sqlx::query(
            "INSERT INTO radio_station \
               (id, user_id, name, stream_url, homepage_url, image_url, artwork_hash, \
                created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(&name)
        .bind(&stream_url)
        .bind(homepage_url.as_deref())
        .bind(image_url.as_deref())
        .bind(artwork_hash.as_deref())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(RadioStationItem {
            id,
            name,
            stream_url,
            homepage_url,
            image_url,
            artwork_hash,
            created_at: now,
            updated_at: now,
        })
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
    /// fetched again; any other is, and a failure stores the address with no
    /// image, so saving the station again is how a fetch is retried.
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
            StationImage::Keep => ImageWrite::Keep,
            StationImage::Set(value) => {
                let image_url = validated_homepage(value)?;
                self.station_image_write(user_id, station_id, image_url)
                    .await?
            }
        };
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        let updated = match &image {
            ImageWrite::Keep => sqlx::query(
                "UPDATE radio_station SET name=?, stream_url=?, homepage_url=?, updated_at=? \
                 WHERE user_id=? AND id=?",
            )
            .bind(&name)
            .bind(&stream_url)
            .bind(homepage_url.as_deref())
            .bind(now)
            .bind(user_id.to_string())
            .bind(station_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected(),
            ImageWrite::Store {
                image_url,
                artwork_hash,
                fetched,
            } => {
                if let Some(fetched) = fetched {
                    record_artwork(&mut tx, fetched, "station", now).await?;
                }
                sqlx::query(
                    "UPDATE radio_station SET name=?, stream_url=?, homepage_url=?, image_url=?, \
                       artwork_hash=?, updated_at=? \
                     WHERE user_id=? AND id=?",
                )
                .bind(&name)
                .bind(&stream_url)
                .bind(homepage_url.as_deref())
                .bind(image_url.as_deref())
                .bind(artwork_hash.as_deref())
                .bind(now)
                .bind(user_id.to_string())
                .bind(station_id.to_string())
                .execute(&mut *tx)
                .await?
                .rows_affected()
            }
        };
        if updated == 0 {
            tx.rollback().await?;
            return Err(ServiceError::NotFound);
        }
        let row = sqlx::query(concat!(station_select!(), "WHERE user_id=? AND id=?"))
            .bind(user_id.to_string())
            .bind(station_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        radio_station_from_row(row)
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

    /// What an edit that sets the image stores, decided before the gate: the
    /// image already held when the address has not changed and its file is in
    /// the store, a fresh fetch otherwise, and no image when that fails.
    ///
    /// Reads the station without the gate, so another edit can land in
    /// between; the worst that does is one fetch more or less, and the
    /// `UPDATE` still names the account, so a station that is not this
    /// account's is 404 whatever was read here.
    async fn station_image_write(
        &self,
        user_id: Uuid,
        station_id: Uuid,
        image_url: Option<String>,
    ) -> Result<ImageWrite, ServiceError> {
        let Some(url) = image_url else {
            return Ok(ImageWrite::Store {
                image_url: None,
                artwork_hash: None,
                fetched: None,
            });
        };
        let held: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT image_url, artwork_hash FROM radio_station WHERE user_id=? AND id=?",
        )
        .bind(user_id.to_string())
        .bind(station_id.to_string())
        .fetch_optional(self.db.pool())
        .await?;
        let Some((held_url, held_hash)) = held else {
            // Nothing is fetched for a station this account cannot see.
            return Err(ServiceError::NotFound);
        };
        if let Some(hash) = held_hash.filter(|_| held_url.as_deref() == Some(url.as_str())) {
            if self.artwork_file_present(&hash).await {
                return Ok(ImageWrite::Store {
                    image_url: Some(url),
                    artwork_hash: Some(hash),
                    fetched: None,
                });
            }
        }
        let fetched = self.fetch_station_image(&url).await;
        Ok(ImageWrite::Store {
            image_url: Some(url),
            artwork_hash: fetched.as_ref().map(|fetched| fetched.hash.clone()),
            fetched,
        })
    }

    /// A station image, fetched into the store. Plain http is allowed: a
    /// station's own logo is as often served that way as its stream is, and
    /// the address rules apply either way.
    async fn fetch_station_image(&self, url: &str) -> Option<FetchedArtwork> {
        self.fetch_remote_artwork(url, true, "station image").await
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
mod tests {
    use super::*;

    #[test]
    fn a_station_address_is_absolute_http_and_nothing_else() {
        assert!(validated_url("http://radio.example.org/stream").is_ok());
        assert!(validated_url("https://radio.example.org:8000/stream.mp3?x=1").is_ok());
        // Trimmed, and the trimming is what is stored.
        assert_eq!(
            validated_url("  https://radio.example.org/s  ").unwrap(),
            "https://radio.example.org/s"
        );
        // Stored as typed: no trailing slash the caller did not write.
        assert_eq!(
            validated_url("https://radio.example.org").unwrap(),
            "https://radio.example.org"
        );

        // Nothing the server could later be made to read off its own disk, and
        // nothing that names no host.
        //
        // **`mms://` is the one that proves the scheme line**, and it is here
        // for that reason: measured by inversion, `file:` and `data:` are
        // already refused for carrying no host, so widening the scheme list to
        // admit them changes no verdict. A scheme with an authority is the only
        // case where the two rules can be told apart.
        assert!(validated_url("mms://radio.example.org/stream").is_err());
        assert!(validated_url("file:///etc/passwd").is_err());
        assert!(validated_url("data:audio/mpeg;base64,AAAA").is_err());
        assert!(validated_url("/stream.mp3").is_err());
        assert!(validated_url("radio.example.org/stream").is_err());
        // An address with no host at all does not reach this function's
        // judgement: the parser refuses it first.
        assert!(validated_url("http://").is_err());
        assert!(validated_url("http://:8000/stream").is_err());
        // And one that looks hostless is not. The WHATWG parser collapses the
        // slashes after a special scheme, so this is a URL whose host is
        // `stream` — surprising enough to be written down rather than
        // discovered by whoever reads the column next.
        assert_eq!(validated_url("http:///stream").unwrap(), "http:///stream");
        assert!(validated_url("").is_err());
        assert!(validated_url(&format!("https://e.org/{}", "a".repeat(2_048))).is_err());
    }

    #[test]
    fn a_blank_homepage_means_none_and_a_blank_address_never_does() {
        assert_eq!(validated_homepage(None).unwrap(), None);
        assert_eq!(validated_homepage(Some("")).unwrap(), None);
        assert_eq!(validated_homepage(Some("   ")).unwrap(), None);
        assert_eq!(
            validated_homepage(Some("https://radio.example.org/")).unwrap(),
            Some("https://radio.example.org/".to_owned())
        );
        // Still judged when it says something.
        assert!(validated_homepage(Some("javascript:alert(1)")).is_err());
        // And the address it sits beside gets no such reading: it is required.
        assert!(validated_url("").is_err());
    }

    #[test]
    fn a_name_is_trimmed_and_cannot_be_blank() {
        assert_eq!(validated_name("  Radio Nova ").unwrap(), "Radio Nova");
        assert!(validated_name("   ").is_err());
        assert!(validated_name("").is_err());
        assert!(validated_name(&"n".repeat(MAX_RADIO_NAME_CHARS)).is_ok());
        assert!(validated_name(&"n".repeat(MAX_RADIO_NAME_CHARS + 1)).is_err());
    }
}
