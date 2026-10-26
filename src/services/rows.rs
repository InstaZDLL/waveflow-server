//! Decoding a projected row into the record the services answer with.

use super::*;

pub(super) fn credential_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<SubsonicCredentialRecord, sqlx::Error> {
    let nonce = row.try_get::<Vec<u8>, _>("password_nonce")?;
    let nonce: [u8; 12] = nonce.try_into().map_err(|value: Vec<u8>| {
        sqlx::Error::Decode(format!("invalid credential nonce length: {}", value.len()).into())
    })?;
    Ok(SubsonicCredentialRecord {
        account: AccountRecord {
            id: parse_uuid(row.try_get("id")?)?,
            username: row.try_get("username")?,
            password_hash: row.try_get("password_hash")?,
            role: AccountRole::from_str(row.try_get::<&str, _>("role")?)
                .map_err(|error| sqlx::Error::Decode(error.into()))?,
            disabled: row.try_get::<i64, _>("disabled")? != 0,
        },
        encrypted_password: EncryptedSecret {
            nonce,
            ciphertext: row.try_get("password_ciphertext")?,
        },
    })
}

pub(super) fn artist_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ArtistItem, sqlx::Error> {
    Ok(ArtistItem {
        // Sorted here rather than trusted from `group_concat`, whose order
        // SQLite does not guarantee even when the subquery feeding it is
        // ordered. Two responses for one artist have to be byte-identical.
        roles: {
            let mut roles: Vec<String> = row
                .try_get::<Option<String>, _>("roles")?
                .map(|roles| roles.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            roles.sort_unstable();
            roles
        },
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        name: row.try_get("name")?,
        artwork_hash: row.try_get("artwork_hash")?,
        musicbrainz_id: row.try_get("musicbrainz_id")?,
        sort_name: row.try_get("sort_name")?,
        starred_at: row.try_get("starred_at")?,
        user_rating: row.try_get("user_rating")?,
    })
}

pub(super) fn artist_summary_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<ArtistSummary, sqlx::Error> {
    let album_count = row.try_get("album_count")?;
    Ok(ArtistSummary {
        artist: artist_from_row(row)?,
        album_count,
    })
}

pub(super) fn album_from_row(row: sqlx::sqlite::SqliteRow) -> Result<AlbumItem, sqlx::Error> {
    let is_compilation = row.try_get::<i64, _>("is_compilation")? != 0;
    let release_types = split_tag_values(
        row.try_get::<Option<String>, _>("release_types")?
            .as_deref(),
    );
    Ok(AlbumItem {
        release_group: ReleaseGroup::classify(&release_types, is_compilation),
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        title: row.try_get("title")?,
        artist: row.try_get("album_artist_name")?,
        artist_id: row
            .try_get::<Option<String>, _>("album_artist_id")?
            .map(parse_uuid)
            .transpose()?,
        artwork_hash: row.try_get("artwork_hash")?,
        year: row.try_get("year")?,
        is_compilation,
        sort_name: row.try_get("sort_name")?,
        musicbrainz_id: row.try_get("musicbrainz_id")?,
        original_release_date: row.try_get("original_release_date")?,
        release_date: row.try_get("release_date")?,
        release_types,
        record_labels: split_tag_values(
            row.try_get::<Option<String>, _>("record_labels")?
                .as_deref(),
        ),
        // Loaded in a batch by `attach_album_relations`, never row by row.
        artists: Vec::new(),
        genres: Vec::new(),
        disc_titles: Vec::new(),
        created_at: row.try_get("created_at")?,
        starred_at: row.try_get("starred_at")?,
        user_rating: row.try_get("user_rating")?,
        play_count: row.try_get("play_count")?,
        last_played_at: row.try_get("last_played_at")?,
        song_count: row.try_get("song_count")?,
        duration_ms: row.try_get("duration_ms")?,
    })
}

pub(super) fn lyrics_list_from_rows(
    track_id: Uuid,
    rows: Vec<sqlx::sqlite::SqliteRow>,
) -> Result<LyricsList, ServiceError> {
    let first = rows.first().ok_or(ServiceError::NotFound)?;
    let display_title: String = first.try_get("title")?;
    let display_artist: Option<String> = first.try_get("artist_display")?;
    let mut structured_lyrics = Vec::new();
    for row in rows {
        let Some(content) = row.try_get::<Option<String>, _>("content")? else {
            continue;
        };
        let synced = row.try_get::<Option<i64>, _>("synced")?.unwrap_or(0) != 0;
        structured_lyrics.push(StructuredLyrics {
            display_artist: display_artist.clone(),
            display_title: display_title.clone(),
            lang: row
                .try_get::<Option<String>, _>("lang")?
                .unwrap_or_else(|| "xxx".into()),
            synced,
            lines: lyrics::lines(&content, synced),
            // The façade's sheets name no source: its answer is frozen.
            source: None,
        });
    }
    Ok(LyricsList {
        track_id,
        structured_lyrics,
    })
}

/// Splits a multi-valued tag string the way the scanner stored it.
///
/// The scanner writes these joined on `;`, so a reader that did not split
/// them would hand a client one value that is really several.
pub(super) fn split_tag_values(raw: Option<&str>) -> Vec<String> {
    raw.into_iter()
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn song_from_row(row: sqlx::sqlite::SqliteRow) -> Result<SongItem, sqlx::Error> {
    let relative: String = row.try_get("relative_path")?;
    Ok(SongItem {
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        album_id: row
            .try_get::<Option<String>, _>("album_id")?
            .map(parse_uuid)
            .transpose()?,
        title: row.try_get("title")?,
        album: row.try_get("album_title")?,
        artist: row.try_get("artist_display")?,
        artist_id: row
            .try_get::<Option<String>, _>("artist_id")?
            .map(parse_uuid)
            .transpose()?,
        genre: row.try_get("genre_display")?,
        year: row.try_get("year")?,
        track: row.try_get("track_number")?,
        disc: row.try_get("disc_number")?,
        duration_ms: row.try_get("duration_ms")?,
        bitrate: row.try_get("bitrate")?,
        codec: row.try_get("codec")?,
        suffix: PathBuf::from(&relative)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase(),
        size: row.try_get("file_size")?,
        artwork_hash: row.try_get("artwork_hash")?,
        full_hash: row.try_get("full_hash")?,
        created_at: row.try_get("created_at")?,
        starred_at: row.try_get("starred_at")?,
        user_rating: row.try_get("user_rating")?,
        sample_rate: row.try_get("sample_rate")?,
        channels: row.try_get("channels")?,
        bit_depth: row.try_get("bit_depth")?,
        play_count: row.try_get("play_count")?,
        last_played_at: row.try_get("last_played_at")?,
        // Filled in by `attach_song_relations`: one row cannot carry them.
        artists: Vec::new(),
        album_artists: Vec::new(),
        contributors: Vec::new(),
        genres: Vec::new(),
        album_artist: row.try_get("album_artist_name")?,
        album_artist_id: row
            .try_get::<Option<String>, _>("album_artist_id")?
            .map(parse_uuid)
            .transpose()?,
        musicbrainz_id: row.try_get("musicbrainz_recording_id")?,
        replay_gain_track_gain: row.try_get("replay_gain_track_gain")?,
        replay_gain_track_peak: row.try_get("replay_gain_track_peak")?,
        replay_gain_album_gain: row.try_get("replay_gain_album_gain")?,
        replay_gain_album_peak: row.try_get("replay_gain_album_peak")?,
        bpm: row.try_get("bpm")?,
        sort_name: row.try_get("sort_title")?,
        comment: row.try_get("comment")?,
        isrc: split_tag_values(row.try_get::<Option<String>, _>("isrc")?.as_deref()),
        moods: split_tag_values(row.try_get::<Option<String>, _>("moods")?.as_deref()),
        explicit_status: row.try_get("explicit_status")?,
        relative_path: relative,
    })
}

pub(super) fn parse_uuid(value: String) -> Result<Uuid, sqlx::Error> {
    Uuid::from_str(&value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}
