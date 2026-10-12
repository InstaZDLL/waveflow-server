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

use super::*;

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
        let rows = sqlx::query(
            "SELECT id, name, stream_url, homepage_url, created_at, updated_at \
             FROM radio_station WHERE user_id=? ORDER BY name COLLATE NOCASE, id",
        )
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
        let row = sqlx::query(
            "SELECT id, name, stream_url, homepage_url, created_at, updated_at \
             FROM radio_station WHERE user_id=? AND id=?",
        )
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
    pub async fn create_radio_station(
        &self,
        user_id: Uuid,
        name: &str,
        stream_url: &str,
        homepage_url: Option<&str>,
    ) -> Result<RadioStationItem, ServiceError> {
        let name = validated_name(name)?;
        let stream_url = validated_url(stream_url)?;
        let homepage_url = validated_homepage(homepage_url)?;
        let id = Uuid::new_v4();
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        // **The writer gate is what makes the count safe**, not the
        // transaction: a deferred SQLite transaction takes no write lock until
        // its first write, so two creations reading 499 inside their own
        // transactions would both insert. They cannot run at once because
        // every mutation in this server holds that one gate. The transaction
        // keeps the count and the insert a single unit for the reader of this
        // code and for whatever bypasses the gate later.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM radio_station WHERE user_id=?")
            .bind(user_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        if count >= MAX_RADIO_STATIONS {
            tx.rollback().await?;
            return Err(ServiceError::Conflict);
        }
        sqlx::query(
            "INSERT INTO radio_station \
               (id, user_id, name, stream_url, homepage_url, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(&name)
        .bind(&stream_url)
        .bind(homepage_url.as_deref())
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
            created_at: now,
            updated_at: now,
        })
    }

    /// Replaces a station's three fields.
    ///
    /// A replacement and not a patch, on both surfaces: the protocol's
    /// `updateInternetRadioStation` requires the name and the address every
    /// time, so omitting the homepage there clears it. A native `PATCH` that
    /// kept the old value instead would make the same call mean two different
    /// things depending on which door it came through.
    pub async fn update_radio_station(
        &self,
        user_id: Uuid,
        station_id: Uuid,
        name: &str,
        stream_url: &str,
        homepage_url: Option<&str>,
    ) -> Result<RadioStationItem, ServiceError> {
        let name = validated_name(name)?;
        let stream_url = validated_url(stream_url)?;
        let homepage_url = validated_homepage(homepage_url)?;
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut connection = self.db.pool().acquire().await?;
        let updated = sqlx::query(
            "UPDATE radio_station SET name=?, stream_url=?, homepage_url=?, updated_at=? \
             WHERE user_id=? AND id=?",
        )
        .bind(&name)
        .bind(&stream_url)
        .bind(homepage_url.as_deref())
        .bind(now)
        .bind(user_id.to_string())
        .bind(station_id.to_string())
        .execute(&mut *connection)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(ServiceError::NotFound);
        }
        // `created_at` is the only field the caller did not supply, so it is the
        // only reason to read the row back.
        let created_at: i64 =
            sqlx::query_scalar("SELECT created_at FROM radio_station WHERE user_id=? AND id=?")
                .bind(user_id.to_string())
                .bind(station_id.to_string())
                .fetch_one(&mut *connection)
                .await?;
        Ok(RadioStationItem {
            id: station_id,
            name,
            stream_url,
            homepage_url,
            created_at,
            updated_at: now,
        })
    }

    /// Removes a station.
    ///
    /// Unlike a bookmark, a station is named by an identifier rather than by
    /// the track it sits on, so removing one that is not there is not the same
    /// request answered twice — it names something that does not exist, and the
    /// account cannot tell that from one belonging to somebody else.
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
}

fn radio_station_from_row(row: sqlx::sqlite::SqliteRow) -> Result<RadioStationItem, ServiceError> {
    Ok(RadioStationItem {
        id: parse_uuid(row.try_get("id")?)?,
        name: row.try_get("name")?,
        stream_url: row.try_get("stream_url")?,
        homepage_url: row.try_get("homepage_url")?,
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
