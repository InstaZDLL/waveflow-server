//! Playlists and their track lists.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    pub async fn playlists(&self, user_id: Uuid) -> Result<Vec<PlaylistItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.playlists_on(&mut connection, user_id).await
    }

    pub(super) async fn playlists_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<PlaylistItem>, ServiceError> {
        let rows = sqlx::query(
            "SELECT id, name, comment, public, created_at, updated_at FROM playlist \
             WHERE owner_user_id=? ORDER BY updated_at DESC, id",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        // One query for every playlist's track list and one for the songs they
        // name, rather than two per playlist: an account with fifty playlists
        // was costing a hundred round trips to answer `getPlaylists`.
        let playlist_ids = rows
            .iter()
            .map(|row| parse_uuid(row.try_get("id")?))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ordered: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        if !playlist_ids.is_empty() {
            let ids_json =
                serde_json::to_string(&playlist_ids).map_err(|_| ServiceError::Invalid)?;
            for row in sqlx::query(
                "SELECT pt.playlist_id, pt.track_id FROM playlist_track pt                  JOIN playlist p ON p.id=pt.playlist_id                  WHERE p.owner_user_id=? AND p.id IN (SELECT value FROM json_each(?))                  ORDER BY pt.playlist_id, pt.position",
            )
            .bind(user_id.to_string())
            .bind(ids_json)
            .fetch_all(&mut *connection)
            .await?
            {
                ordered
                    .entry(parse_uuid(row.try_get("playlist_id")?)?)
                    .or_default()
                    .push(parse_uuid(row.try_get("track_id")?)?);
            }
        }
        let mut union: Vec<Uuid> = ordered.values().flatten().copied().collect();
        union.sort_unstable();
        union.dedup();
        // Resolved once for every playlist at once. The per-playlist order and
        // the dropping of a track this account cannot see are reapplied below,
        // exactly as `songs_by_ids_lenient_on` would have applied them.
        let visible = self
            .songs_by_ids_lenient_on(connection, user_id, &union)
            .await?
            .into_iter()
            .map(|song| (song.id, song))
            .collect::<HashMap<_, _>>();
        let mut result = Vec::with_capacity(rows.len());
        for (row, id) in rows.into_iter().zip(playlist_ids) {
            result.push(PlaylistItem {
                id,
                name: row.try_get("name")?,
                comment: row.try_get("comment")?,
                public: row.try_get::<i64, _>("public")? != 0,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
                songs: ordered
                    .get(&id)
                    .into_iter()
                    .flatten()
                    .filter_map(|track| visible.get(track).cloned())
                    .collect(),
            });
        }
        Ok(result)
    }

    pub async fn playlist(&self, user_id: Uuid, id: Uuid) -> Result<PlaylistItem, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.playlist_on(&mut connection, user_id, id).await
    }

    async fn playlist_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<PlaylistItem, ServiceError> {
        let row = sqlx::query(
            "SELECT id, name, comment, public, created_at, updated_at FROM playlist \
             WHERE id=? AND owner_user_id=?",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(ServiceError::NotFound)?;
        Ok(PlaylistItem {
            id,
            name: row.try_get("name")?,
            comment: row.try_get("comment")?,
            public: row.try_get::<i64, _>("public")? != 0,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            songs: self.playlist_songs_on(connection, user_id, id).await?,
        })
    }

    async fn playlist_songs_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<Vec<SongItem>, ServiceError> {
        let ids = self
            .playlist_track_ids_on(connection, user_id, playlist_id)
            .await?;
        self.songs_by_ids_lenient_on(connection, user_id, &ids)
            .await
    }

    async fn playlist_track_ids_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<Vec<Uuid>, ServiceError> {
        sqlx::query_scalar::<_, String>(
            "SELECT pt.track_id FROM playlist_track pt JOIN playlist p ON p.id=pt.playlist_id \
             WHERE p.id=? AND p.owner_user_id=? ORDER BY pt.position",
        )
        .bind(playlist_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
    }

    pub async fn create_playlist(
        &self,
        user_id: Uuid,
        name: &str,
        track_ids: &[Uuid],
    ) -> Result<PlaylistItem, ServiceError> {
        self.create_playlist_with_context(
            user_id,
            name,
            track_ids,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn create_playlist_with_context(
        &self,
        user_id: Uuid,
        name: &str,
        track_ids: &[Uuid],
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let intent = MutationIntent::new(
            "create",
            "playlist",
            &serde_json::json!({ "name": name.trim(), "track_ids": track_ids }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            let id = receipt.result_entity_id.ok_or(ServiceError::Conflict)?;
            drop(_writer);
            return self.playlist(user_id, id).await;
        }
        // Checked after the replay branch and not before it. A replay owes the
        // caller the outcome the original call had, and this ceiling is a
        // policy number rather than a fact of the domain: lower it in a later
        // release and an operation that was valid when it ran would start
        // answering an error to its own retry. Still ahead of every write.
        if track_ids.len() > MAX_PLAYLIST_TRACKS {
            return Err(ServiceError::Invalid);
        }
        validate_name(name)?;
        self.songs_by_ids_on(&mut tx, user_id, track_ids).await?;
        let id = Uuid::new_v4();
        let now = now_ms();
        sqlx::query("INSERT INTO playlist (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(id.to_string()).bind(user_id.to_string()).bind(name.trim()).bind(now).bind(now)
            .execute(&mut *tx).await?;
        replace_playlist_tracks(&mut tx, id, track_ids, now).await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "playlist",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "name": name.trim(),
                    "track_ids": track_ids,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.playlist(user_id, id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_playlist(
        &self,
        user_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
        add: &[Uuid],
        remove_indexes: &[usize],
        clear: PlaylistClear,
    ) -> Result<PlaylistItem, ServiceError> {
        self.update_playlist_with_context(
            user_id,
            id,
            name,
            comment,
            public,
            add,
            remove_indexes,
            clear,
            MutationContext::server_generated(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_playlist_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
        add: &[Uuid],
        remove_indexes: &[usize],
        clear: PlaylistClear,
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let mut removes = remove_indexes.to_vec();
        removes.sort_unstable_by(|a, b| b.cmp(a));
        removes.dedup();
        let mut intent_payload = serde_json::json!({
            "name": name.map(str::trim),
            "comment": comment,
            "public": public,
            "add": add,
            "remove_indexes": &removes,
            "clear_comment": clear.comment,
        });
        // Added to the payload only when set. The intent is hashed and compared
        // on replay, so naming a new field unconditionally would change the
        // hash of every update this server version ever saw before, and turn a
        // client's retry across an upgrade into a conflict.
        if clear.tracks {
            intent_payload["clear_tracks"] = serde_json::Value::Bool(true);
        }
        let intent = MutationIntent::new("update", &format!("playlist:{id}"), &intent_payload);
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            drop(_writer);
            return self.playlist(user_id, id).await;
        }
        // A request naming more tracks than a playlist may hold can never be
        // valid, whatever this playlist currently holds, so it is refused
        // before anything is read. After the replay branch for the same reason
        // the create path is: a retry is owed its original outcome.
        if add.len() > MAX_PLAYLIST_TRACKS {
            return Err(ServiceError::Invalid);
        }
        let current = self.playlist_on(&mut tx, user_id, id).await?;
        if let Some(name) = name {
            validate_name(name)?;
        }
        self.songs_by_ids_on(&mut tx, user_id, add).await?;
        let mut ids = if clear.tracks {
            Vec::new()
        } else {
            self.playlist_track_ids_on(&mut tx, user_id, id).await?
        };
        for index in removes {
            if index >= ids.len() {
                return Err(ServiceError::Invalid);
            }
            ids.remove(index);
        }
        ids.extend_from_slice(add);
        // Checked on the result rather than on `add`: an update that adds one
        // track to a playlist already at the ceiling is what has to be refused,
        // and the request that gets there is small.
        if ids.len() > MAX_PLAYLIST_TRACKS {
            return Err(ServiceError::Invalid);
        }
        let changed_at = now_ms();
        sqlx::query(
            "UPDATE playlist SET name=COALESCE(?, name), \
             comment=CASE WHEN ? THEN NULL ELSE COALESCE(?, comment) END, \
             public=COALESCE(?, public), updated_at=? WHERE id=? AND owner_user_id=?",
        )
        .bind(name.map(str::trim))
        .bind(clear.comment)
        .bind(comment)
        .bind(public.map(i64::from))
        .bind(changed_at)
        .bind(id.to_string())
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM playlist_track WHERE playlist_id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        replace_playlist_tracks(&mut tx, id, &ids, changed_at).await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "playlist",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "name": name.map(str::trim).unwrap_or(&current.name),
                    "comment": comment.or(current.comment.as_deref()),
                    "public": public.unwrap_or(current.public),
                    "track_ids": ids,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.playlist(user_id, id).await
    }

    pub async fn delete_playlist(&self, user_id: Uuid, id: Uuid) -> Result<(), ServiceError> {
        self.delete_playlist_with_context(user_id, id, MutationContext::server_generated())
            .await
    }

    pub async fn delete_playlist_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent =
            MutationIntent::new("delete", &format!("playlist:{id}"), &serde_json::json!({}));
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            return Ok(());
        }
        let changed = sqlx::query("DELETE FROM playlist WHERE id=? AND owner_user_id=?")
            .bind(id.to_string())
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if changed == 0 {
            tx.rollback().await?;
            Err(ServiceError::NotFound)
        } else {
            let receipt = self
                .sync
                .complete_operation(
                    &mut tx,
                    user_id,
                    context,
                    "playlist",
                    id,
                    "delete",
                    &serde_json::json!({}),
                    Some(id),
                )
                .await?;
            tx.commit().await?;
            self.sync.publish(user_id, receipt);
            Ok(())
        }
    }

    /// Imports an `.m3u` or `.m3u8` file as a playlist of this account's.
    ///
    /// The file is read by [`crate::playlist_file`]; what is left here is the
    /// part that needs the catalogue — turning what an entry *says* into a
    /// track this account can actually reach.
    ///
    /// **The ladder.** A playlist entry is a path relative to wherever the file
    /// lived, or absolute on a filesystem that is not this server's, so what
    /// the two sides share is a trailing run of segments. The longest suffix is
    /// tried first and the rungs go down to the bare file name; several matches
    /// at any rung is a refusal rather than a guess, because a shorter suffix
    /// can only match more. An entry no path matched gets one last rung: its
    /// `#EXTINF` credit, which is the only other thing a playlist file carries.
    ///
    /// **The issue asked for the scanner's ladder — path, then content hash —
    /// and the second rung cannot exist here.** The scanner holds the file and
    /// can hash it; a playlist entry hands over a path and, at best, a display
    /// title. So the fallback is the credit, and an entry whose path moved
    /// *and* whose credit was never written is reported rather than dropped.
    ///
    /// Resolution runs tenant-scoped in SQL, so a track in a library this
    /// account cannot see is reported as unmatched, exactly like one that does
    /// not exist — the report says nothing about which of the two it was.
    pub async fn import_playlist(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
        name: Option<&str>,
        bytes: &[u8],
        dry_run: bool,
        context: MutationContext,
    ) -> Result<PlaylistImport, ServiceError> {
        let file = playlist_file::parse(bytes);
        // The playlist ceiling, applied to the file rather than discovered
        // after the work: every entry costs a resolution, and a caller that
        // sent five thousand of them is owed the refusal before the reads.
        if file.entries.len() > MAX_PLAYLIST_TRACKS {
            return Err(ServiceError::Invalid);
        }
        let name = name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| file.name.clone())
            .ok_or(ServiceError::Invalid)?;
        validate_name(&name)?;

        let mut connection = self.db.pool().acquire().await?;
        let by_file_name = self
            .tracks_by_file_name_on(&mut connection, user_id, library_ids, &file.entries)
            .await?;
        // The path rung for every entry, then one batch for the leftovers: the
        // credit lookup is only worth making for the entries that reached it,
        // and making it per entry would be a query per line of a file that may
        // hold ten thousand.
        let climbed = file
            .entries
            .iter()
            .map(|entry| {
                // ASCII, like the comparison that selected the candidates
                // and like every other catalogue match. `to_lowercase` folds
                // more than SQLite's `NOCASE` does, and a key wider than the
                // filter that produced it is a disagreement: an entry reading
                // `ÉTÉ.flac` against a stored `été.flac` would be folded here
                // and never returned there. The residual limit is that such an
                // entry is reported rather than placed, which the report says
                // out loud.
                let segments = entry
                    .segments
                    .iter()
                    .map(|segment| segment.to_ascii_lowercase())
                    .collect::<Vec<_>>();
                (entry, climb(&segments, &by_file_name))
            })
            .collect::<Vec<_>>();
        let leftovers = climbed
            .iter()
            .filter(|(_, rung)| matches!(rung, Rung::Unmatched))
            .map(|(entry, _)| *entry)
            .collect::<Vec<_>>();
        let by_credit = self
            .tracks_by_credit_on(&mut connection, user_id, library_ids, &leftovers)
            .await?;
        drop(connection);

        let mut track_ids: Vec<Uuid> = Vec::new();
        // Beside the list rather than in place of it: the order is the file's
        // and has to be kept, while the question asked of it — "already in
        // here?" — is asked once per entry. Scanning the list instead was
        // quadratic in a length the caller chooses, up to ten thousand.
        let mut placed: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        let mut unresolved: Vec<UnresolvedEntry> = Vec::new();
        for (entry, rung) in climbed {
            let report = |reason| UnresolvedEntry {
                line: entry.line,
                entry: entry.raw.clone(),
                title: entry.title.clone(),
                reason,
            };
            let outcome = match rung {
                Rung::Found(id) => Ok(id),
                Rung::Ambiguous => Err(UnresolvedReason::Ambiguous),
                Rung::NotAPath => Err(UnresolvedReason::NotAPath),
                Rung::Unmatched => match entry
                    .artist_and_title()
                    .map(|(artist, title)| format!("{artist}\n{title}").to_ascii_lowercase())
                    .and_then(|key| by_credit.get(&key))
                    .map(Vec::as_slice)
                {
                    Some([only]) => Ok(*only),
                    Some([_, ..]) => Err(UnresolvedReason::Ambiguous),
                    Some([]) | None => Err(UnresolvedReason::NoMatch),
                },
            };
            match outcome {
                // A playlist carries a track once, so a file naming one twice
                // has a second copy with nowhere to go. Reported rather than
                // dropped, because a shorter playlist with no explanation is
                // the failure this whole report exists to prevent.
                Ok(id) if placed.contains(&id) => {
                    unresolved.push(report(UnresolvedReason::Duplicate));
                }
                Ok(id) => {
                    placed.insert(id);
                    track_ids.push(id);
                }
                Err(reason) => unresolved.push(report(reason)),
            }
        }

        // Created through the ordinary verb rather than beside it: an import is
        // a playlist creation whose track list came out of a file, so it claims
        // a sync operation, lands in the journal and reaches other devices like
        // any other. A second insert here would be a second implementation of
        // what a playlist is.
        let playlist = match dry_run {
            true => None,
            false => Some(
                self.create_playlist_with_context(user_id, &name, &track_ids, context)
                    .await?,
            ),
        };
        Ok(PlaylistImport {
            playlist,
            name,
            encoding: file.encoding,
            entries: file.entries.len(),
            resolved: track_ids.len(),
            unresolved,
        })
    }

    /// Every track whose file name one of the entries could be naming, with
    /// its own path, keyed on that name.
    ///
    /// **The file name is the key because the comparison is symmetric, and the
    /// first version of this was not.** An entry can be *longer* than the
    /// stored path — absolute on someone else's filesystem — and it can just
    /// as easily be *shorter*: a `.m3u` written inside an album folder names
    /// `01 Hidden Place.flac` while the catalogue stores
    /// `Björk/Vespertine/01 Hidden Place.flac`. Asking SQL for the stored path
    /// to equal one of the entry's own suffixes answers the first case and
    /// never the second, which is the common one. What the two shapes always
    /// share is the file name, so that is what is asked for; how much of the
    /// rest agrees is settled in memory by [`climb`], where both sides are
    /// whole.
    ///
    /// The name is extracted in SQL — SQLite has no `basename`, hence the
    /// `rtrim` of everything after the last separator — so this is a scan of
    /// the visible tracks. It is one scan per import, which is a rare
    /// user-initiated act, and the alternative is a stored or generated column
    /// for a path shape that only this feature reads.
    ///
    /// The value is a list on purpose: a file name two tracks carry is what
    /// [`UnresolvedReason::Ambiguous`] is about, and a map keyed to one id
    /// would have hidden the second.
    async fn tracks_by_file_name_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        library_ids: &[Uuid],
        entries: &[PlaylistFileEntry],
    ) -> Result<HashMap<String, Vec<(Uuid, Vec<String>)>>, ServiceError> {
        let mut candidates: Vec<String> = entries
            .iter()
            .filter_map(|entry| entry.segments.last().cloned())
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        let folders = folder_filter(library_ids);
        let mut found: HashMap<String, Vec<(Uuid, Vec<String>)>> = HashMap::new();
        for batch in candidates.chunks(RESOLUTION_BATCH) {
            let wanted = serde_json::to_string(batch).map_err(|_| ServiceError::Invalid)?;
            for row in sqlx::query(
                "SELECT t.id, t.relative_path, \
                        substr(t.relative_path, \
                               length(rtrim(t.relative_path, \
                                            replace(t.relative_path, '/', ''))) + 1) AS file_name \
                 FROM track t \
                 JOIN library_member m ON m.library_id=t.library_id \
                 WHERE m.user_id=? AND t.is_available=1 \
                   AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?))) \
                   AND file_name COLLATE NOCASE IN (SELECT value FROM json_each(?))",
            )
            .bind(user_id.to_string())
            .bind(folders.as_deref())
            .bind(folders.as_deref())
            .bind(&wanted)
            .fetch_all(&mut *connection)
            .await?
            {
                let path: String = row.try_get("relative_path")?;
                let file_name: String = row.try_get("file_name")?;
                found
                    .entry(file_name.to_ascii_lowercase())
                    .or_default()
                    .push((parse_uuid(row.try_get("id")?)?, lowercased_segments(&path)));
            }
        }
        Ok(found)
    }

    /// The last rung: the `#EXTINF` credit, matched exactly.
    ///
    /// Asked only for the entries no path matched, and only for those whose
    /// title splits into an artist and a song — the convention every player
    /// writes and none guarantees. The comparison is against the title the
    /// account sees, correction included, because a corrected title is the
    /// title.
    ///
    /// Batched like the path rung rather than asked per entry: a file whose
    /// paths are all foreign would otherwise be one query per line, and the
    /// ceiling on a file is ten thousand lines.
    ///
    /// The key is the two fields joined by a newline, which is a separator
    /// neither of them can contain — `track.title` and `artist_display` come
    /// from tags read line by line. The folding is ASCII on both sides: SQLite
    /// matches `NOCASE`, and the map is keyed through `to_ascii_lowercase`
    /// rather than `to_lowercase` so the key cannot be wider than the filter
    /// that produced it.
    async fn tracks_by_credit_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        library_ids: &[Uuid],
        entries: &[&PlaylistFileEntry],
    ) -> Result<HashMap<String, Vec<Uuid>>, ServiceError> {
        let mut wanted: Vec<String> = entries
            .iter()
            .filter_map(|entry| entry.artist_and_title())
            .map(|(artist, title)| format!("{artist}\n{title}"))
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        let folders = folder_filter(library_ids);
        let mut found: HashMap<String, Vec<Uuid>> = HashMap::new();
        for batch in wanted.chunks(RESOLUTION_BATCH) {
            let keys = serde_json::to_string(batch).map_err(|_| ServiceError::Invalid)?;
            for row in sqlx::query(
                "SELECT t.id, \
                        t.artist_display || char(10) || COALESCE(ovr.title, t.title) AS credit \
                 FROM track t \
                 JOIN library_member m ON m.library_id=t.library_id \
                 LEFT JOIN track_override ovr ON ovr.track_id=t.id \
                 WHERE m.user_id=? AND t.is_available=1 \
                   AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?))) \
                   AND (t.artist_display || char(10) || COALESCE(ovr.title, t.title)) \
                       COLLATE NOCASE IN (SELECT value FROM json_each(?))",
            )
            .bind(user_id.to_string())
            .bind(folders.as_deref())
            .bind(folders.as_deref())
            .bind(&keys)
            .fetch_all(&mut *connection)
            .await?
            {
                let credit: String = row.try_get("credit")?;
                found
                    .entry(credit.to_ascii_lowercase())
                    .or_default()
                    .push(parse_uuid(row.try_get("id")?)?);
            }
        }
        Ok(found)
    }

    /// Renders one of this account's playlists back as an extended `.m3u8`.
    ///
    /// The paths are library-relative, which is what the import reads: a
    /// playlist exported here and imported again resolves by the same ladder,
    /// and that round trip is the strongest evidence the reader is right — a
    /// test asserts it rather than a reviewer believing it.
    ///
    /// It reads `relative_path` itself rather than through `SongItem`, which
    /// does not carry it. Putting a server-side path on every song of every
    /// listing to serve one export would widen a payload the whole catalogue
    /// answers with.
    ///
    /// UTF-8, hence `.m3u8`, with no cleverness about it: the only honest
    /// reason the other encoding exists is that old files were written in it,
    /// which is a reading problem rather than a writing one.
    pub async fn export_playlist(
        &self,
        user_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<String, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        // First, and for its refusal: a playlist of another account answers as
        // one that does not exist.
        let playlist = self
            .playlist_on(&mut connection, user_id, playlist_id)
            .await?;
        let rows = sqlx::query(
            "SELECT t.relative_path, COALESCE(ovr.title, t.title) AS title, \
                    t.artist_display, t.duration_ms \
             FROM playlist_track pt \
             JOIN playlist p ON p.id=pt.playlist_id \
             JOIN track t ON t.id=pt.track_id \
             JOIN library_member m ON m.library_id=t.library_id \
             LEFT JOIN track_override ovr ON ovr.track_id=t.id \
             WHERE p.id=? AND p.owner_user_id=? AND m.user_id=? AND t.is_available=1 \
             ORDER BY pt.position",
        )
        .bind(playlist_id.to_string())
        .bind(user_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut out = String::from("#EXTM3U\n");
        out.push_str(&format!("#PLAYLIST:{}\n", one_line(&playlist.name)));
        for row in rows {
            let path: String = row.try_get("relative_path")?;
            let title: String = row.try_get("title")?;
            let artist: Option<String> = row.try_get("artist_display")?;
            let duration_ms: i64 = row.try_get("duration_ms")?;
            let credit = match artist.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
                Some(artist) => format!("{} - {}", one_line(artist), one_line(&title)),
                None => one_line(&title),
            };
            out.push_str(&format!(
                "#EXTINF:{},{}\n{}\n",
                duration_ms / 1000,
                credit,
                one_line(&path)
            ));
        }
        Ok(out)
    }
}

/// How many candidates one batch asks for.
///
/// A file may name ten thousand targets, so the whole set in one bind
/// parameter would be a statement of some hundreds of kilobytes. Slicing keeps
/// every statement a normal size and the number of statements small: at most
/// ten for the file names of the largest file the route accepts, and ten more
/// for the credits of the entries that reach that rung.
const RESOLUTION_BATCH: usize = 1_000;

/// What the path rung made of one entry.
///
/// `Unmatched` is deliberately not `Ambiguous`: only the first has a rung left
/// to try, and collapsing the two into an `Option` would have sent an entry
/// the catalogue already answered twice on to a credit lookup that cannot
/// improve on it.
enum Rung {
    Found(Uuid),
    Ambiguous,
    NotAPath,
    Unmatched,
}

/// The path ladder: among the tracks sharing this entry's file name, the ones
/// that agree with it for as many trailing segments as possible.
///
/// Climbed from the longest tail down, and both sides are bounded by
/// themselves rather than by a constant — the entry may name four parents and
/// the catalogue two, or the other way round, and the deepest agreement either
/// one can express is the strongest answer available.
///
/// The first depth at which exactly one candidate agrees resolves the entry. A
/// depth where several agree ends the climb as ambiguous: a shorter tail is a
/// weaker question, so it cannot separate what this one could not. A depth
/// where none agree is simply too specific, and the next one down is tried.
fn climb(segments: &[String], by_file_name: &HashMap<String, Vec<(Uuid, Vec<String>)>>) -> Rung {
    let Some(file_name) = segments.last() else {
        return Rung::NotAPath;
    };
    let Some(candidates) = by_file_name.get(file_name) else {
        return Rung::Unmatched;
    };
    let deepest = candidates
        .iter()
        .map(|(_, stored)| stored.len())
        .max()
        .unwrap_or(1)
        .min(segments.len());
    for depth in (1..=deepest).rev() {
        let wanted = &segments[segments.len() - depth..];
        let agreeing = candidates
            .iter()
            .filter(|(_, stored)| {
                stored.len() >= depth && &stored[stored.len() - depth..] == wanted
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        match agreeing.as_slice() {
            [only] => return Rung::Found(*only),
            [_, ..] => return Rung::Ambiguous,
            [] => {}
        }
    }
    Rung::Unmatched
}

/// A stored path, split and folded the way an entry's segments are, so the two
/// can be compared without a second spelling of either rule.
fn lowercased_segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// A field flattened onto one line.
///
/// A playlist file is line-structured, so a newline inside a title would end
/// the line early and make the next one look like a path — a track called
/// "x\n/etc/passwd" would otherwise write an entry of its own. Tabs go too:
/// they are invisible in the output and some readers split on them.
fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '\r' | '\n' | '\t' => ' ',
            other => other,
        })
        .collect::<String>()
        .trim()
        .to_owned()
}
