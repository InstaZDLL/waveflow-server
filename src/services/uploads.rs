//! Deciding whether the server wants a file, before a byte of it moves.
//!
//! Every refusal a received file can earn is cheap exactly once: here, before
//! the transfer. After the last byte, the same refusal has already cost the
//! bandwidth it existed to save. RFC-008 has the reasoning.

use super::*;

/// The one directory in a library the server writes to.
///
/// Inside the library root so the ordinary scan finds what lands there without
/// being taught a second place to look, and named by the server so a client
/// never proposes a path — which is what makes directory traversal, overwriting
/// a file the operator filed themselves, and collisions between two transfers
/// problems that are never posed rather than problems that are solved.
pub(crate) const MANAGED_DIR: &str = ".waveflow-managed";

/// What a session looks like to the code that has to act on it.
struct SessionTarget {
    library_id: Uuid,
    root: std::path::PathBuf,
    declared_hash: String,
    declared_size: i64,
    extension: String,
    next_chunk: i64,
    received_bytes: i64,
    expires_at: i64,
}

impl SessionTarget {
    /// Where the fragments accumulate: the destination directory itself, under
    /// a name carrying no audio extension.
    ///
    /// Both halves matter. Same directory means same filesystem, so the rename
    /// at commit is atomic rather than a copy with a window during which a
    /// truncated file exists. And the walk skips it not because it is hidden —
    /// nothing about the walk skips hidden directories — but because it filters
    /// on extension, which is a rule that already exists and cannot be broken
    /// by someone rewriting the traversal.
    fn staging_path(&self, session_id: Uuid) -> std::path::PathBuf {
        self.root
            .join(MANAGED_DIR)
            .join(format!("{session_id}.part"))
    }

    /// The name the file keeps, once the bytes are known to be what was
    /// promised.
    ///
    /// By hash rather than by tags: tags move — a correction, a retag, a later
    /// re-identification — and a path built from them would leave the server
    /// choosing between moving the file and keeping a name that lies. The
    /// project already answered that question in the other direction, where a
    /// correction never rewrites the file. It keeps its extension because the
    /// walk recognises a file by nothing else.
    fn relative_final(&self, hash: &str) -> String {
        format!("{MANAGED_DIR}/{hash}.{}", self.extension)
    }
}

/// A hash the server will compare against one it computes itself.
///
/// Lowercase hex, sixty-four characters. A client that sends anything else has
/// a bug rather than a file the server declines, which is why this refuses the
/// whole request instead of earning a verdict.
fn clean_hash(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(value)
}

/// The extension the file will keep once it is stored, without its dot.
fn clean_extension(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('.').to_ascii_lowercase();
    waveflow_core::scanner::AUDIO_EXTENSIONS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(&value))
        .then_some(value)
}

impl DomainServices {
    /// Answers a batch of offers, opening a session for each one it accepts.
    ///
    /// A batch because the alternative is five thousand round trips for a
    /// library that is mostly already there. Bounded because the alternative to
    /// five thousand round trips must not be one unbounded body.
    ///
    /// The whole batch is decided under the writer gate, in one transaction.
    /// Two negotiations that each ask for four gigabytes with five gigabytes
    /// free have to be told about each other, and the only thing that can tell
    /// them is the transaction that reserves.
    pub async fn negotiate_uploads(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        offers: Vec<UploadOffer>,
    ) -> Result<Vec<UploadVerdict>, ServiceError> {
        // Shape first, and before the gate. Nothing here can change under a
        // concurrent write, so a malformed request should not queue behind a
        // scan for the right to be told it is malformed.
        if offers.is_empty() || offers.len() > self.uploads.batch_limit {
            return Err(ServiceError::Invalid);
        }
        let mut cleaned = Vec::with_capacity(offers.len());
        for offer in &offers {
            let Some(hash) = clean_hash(&offer.full_hash) else {
                return Err(ServiceError::Invalid);
            };
            if offer.size_bytes <= 0 {
                return Err(ServiceError::Invalid);
            }
            // An unusable extension is a verdict, not a malformed request: the
            // client asked a question and the answer is no. A hash that is not
            // a hash is the other kind, and refused above.
            cleaned.push((hash, offer.size_bytes, clean_extension(&offer.extension)));
        }
        // One offer per file. Two entries for the same hash would each reserve
        // the quota and race for the unique index below, and the second would
        // fail the whole batch over something the client could see itself.
        let mut distinct = std::collections::HashSet::with_capacity(cleaned.len());
        if !cleaned.iter().all(|(hash, _, _)| distinct.insert(hash)) {
            return Err(ServiceError::Invalid);
        }

        // Before the gate: the sweep removes staging files, and file I/O has no
        // business happening while the process-wide writer gate is held.
        let now = now_ms();
        self.sweep_expired_sessions(now).await?;

        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;

        // Membership and the flag are read under the gate, like every other
        // authorisation this server writes behind: a role revoked while this
        // call was deciding must not commit between the check and the write.
        let row = sqlx::query(
            "SELECT m.role, l.accepts_uploads FROM library l \
             JOIN library_member m ON m.library_id=l.id \
             WHERE l.id=? AND m.user_id=?",
        )
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ServiceError::NotFound)?;
        let role = crate::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
            .map_err(|_| ServiceError::Invalid)?;
        if !role.may_upload() {
            // Blurred onto 404 by the surface above. A caller who may not
            // upload learns nothing about whether the library would have.
            return Err(ServiceError::Forbidden);
        }
        if row.try_get::<i64, _>("accepts_uploads")? == 0 {
            // Every offer, one answer. The caller is entitled to be here, so
            // the closed door is something they may be told about — unlike the
            // 404 above, which is for someone who is not.
            return Ok(cleaned
                .into_iter()
                .map(|(hash, _, _)| UploadVerdict {
                    full_hash: hash,
                    decision: UploadDecision::LibraryClosed,
                    track_id: None,
                    session: None,
                })
                .collect());
        }

        // Unavailable tracks are left out: the file a scan stopped finding is
        // not occupying the disk this quota measures.
        let used: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(file_size), 0) FROM track \
             WHERE library_id=? AND is_available=1",
        )
        .bind(library_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        let reserved: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(declared_size), 0) FROM upload_session WHERE library_id=?",
        )
        .bind(library_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        let mut committed = used.saturating_add(reserved);
        let mut held: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE user_id=?")
                .bind(user_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        let sessions_per_user = i64::try_from(self.uploads.sessions_per_user).unwrap_or(i64::MAX);
        let expires_at = now.saturating_add(
            i64::try_from(self.uploads.session_ttl.as_millis()).unwrap_or(i64::MAX),
        );

        // Every session this account already holds here, read once. An account
        // holds a handful at most, and the loop below runs under the
        // process-wide writer gate: a query per offer would hold the gate that
        // much longer for a set small enough to keep in hand.
        let mut open: std::collections::HashMap<String, UploadSessionState> =
            std::collections::HashMap::new();
        let rows = sqlx::query(
            "SELECT id, declared_hash, next_chunk, received_bytes, expires_at \
             FROM upload_session WHERE library_id=? AND user_id=?",
        )
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(&mut *tx)
        .await?;
        for row in rows {
            open.insert(
                row.try_get::<String, _>("declared_hash")?,
                UploadSessionState {
                    session_id: parse_uuid(row.try_get("id")?)?,
                    next_chunk: row.try_get("next_chunk")?,
                    received_bytes: row.try_get("received_bytes")?,
                    chunk_bytes: self.uploads.chunk_bytes,
                    expires_at: row.try_get("expires_at")?,
                },
            );
        }

        let mut verdicts = Vec::with_capacity(cleaned.len());
        for (hash, size, extension) in cleaned {
            // Already here, and that ends it — including when the same file is
            // half-transferred under an open session, because the session is
            // then about bytes the library already has.
            let present: Option<String> = sqlx::query_scalar(
                "SELECT id FROM track WHERE library_id=? AND full_hash=? AND is_available=1 \
                 ORDER BY id LIMIT 1",
            )
            .bind(library_id.to_string())
            .bind(&hash)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(track_id) = present {
                verdicts.push(UploadVerdict {
                    full_hash: hash,
                    decision: UploadDecision::Present,
                    track_id: Some(parse_uuid(track_id)?),
                    session: None,
                });
                continue;
            }

            // A session already open for this file is returned rather than
            // replaced. A client that restarts mid-transfer re-offers what it
            // was sending; opening a second session would strand the first
            // one's reservation for as long as its expiry, and collide with the
            // unique index that exists to stop exactly that.
            if let Some(existing) = open.get(&hash) {
                verdicts.push(UploadVerdict {
                    full_hash: hash,
                    decision: UploadDecision::Accepted,
                    track_id: None,
                    session: Some(existing.clone()),
                });
                continue;
            }

            let Some(extension) = extension else {
                verdicts.push(refused(hash, UploadDecision::UnsupportedFormat));
                continue;
            };
            if size > self.uploads.max_file_bytes {
                verdicts.push(refused(hash, UploadDecision::TooLarge));
                continue;
            }
            if held >= sessions_per_user {
                verdicts.push(refused(hash, UploadDecision::TooManySessions));
                continue;
            }
            // The running total, not the total this batch started with. Two
            // offers of four gigabytes with five free must not both be told
            // yes, and inside one batch the earlier one is what says so.
            let Some(after) = committed.checked_add(size) else {
                verdicts.push(refused(hash, UploadDecision::QuotaExceeded));
                continue;
            };
            if after > self.uploads.library_quota_bytes {
                verdicts.push(refused(hash, UploadDecision::QuotaExceeded));
                continue;
            }

            let session_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO upload_session \
                   (id, library_id, user_id, declared_hash, declared_size, extension, \
                    created_at, expires_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(session_id.to_string())
            .bind(library_id.to_string())
            .bind(user_id.to_string())
            .bind(&hash)
            .bind(size)
            .bind(&extension)
            .bind(now)
            .bind(expires_at)
            .execute(&mut *tx)
            .await?;
            committed = after;
            held += 1;
            verdicts.push(UploadVerdict {
                full_hash: hash,
                decision: UploadDecision::Accepted,
                track_id: None,
                session: Some(UploadSessionState {
                    session_id,
                    next_chunk: 0,
                    received_bytes: 0,
                    chunk_bytes: self.uploads.chunk_bytes,
                    expires_at,
                }),
            });
        }

        tx.commit().await?;
        drop(_writer);
        Ok(verdicts)
    }
}

fn refused(full_hash: String, decision: UploadDecision) -> UploadVerdict {
    UploadVerdict {
        full_hash,
        decision,
        track_id: None,
        session: None,
    }
}

impl DomainServices {
    /// Sweeps abandoned sessions on a schedule, so cleanup does not wait for
    /// somebody to offer another file.
    ///
    /// Without this the sweep only runs inside a negotiation. Nothing breaks —
    /// the quota is decided by that same negotiation, which sweeps first — but
    /// a client that abandons a batch of transfers and never returns leaves its
    /// staging files holding the operator's disk with nothing to signal it.
    ///
    /// Boot first, then one pass per session lifetime: an abandoned file then
    /// lives at most twice the lifetime a session was promised, which is the
    /// same order of magnitude that promise already makes. The shape is the
    /// scanner's `spawn_background`, deliberately.
    pub fn spawn_upload_sweeper(&self) {
        let services = self.clone();
        let interval = services.uploads.session_ttl;
        tokio::spawn(async move {
            services.sweep_now().await;
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                services.sweep_now().await;
            }
        });
    }

    async fn sweep_now(&self) {
        if let Err(error) = self.sweep_expired_sessions(now_ms()).await {
            tracing::warn!(%error, "could not sweep expired upload sessions");
        }
    }

    /// Removes what expired sessions left behind, before anything counts them.
    ///
    /// Each session is swept under its own lock, and that is not tidiness. A
    /// fragment that read its session just before it expired can be writing
    /// while this runs: delete the file and the row without the lock, and that
    /// write recreates the staging file a moment later with no row left to
    /// remember it — a file on the operator's disk nothing will ever sweep
    /// again. The lock makes the two mutually exclusive.
    ///
    /// File first, row second, and that order is deliberate too: dying between
    /// the two leaves a row whose file is already gone, which the next sweep
    /// repeats harmlessly. The other order leaves the orphan.
    ///
    /// Run before the writer gate is taken. File I/O has no business happening
    /// while the process-wide gate is held, and a session that expires during
    /// this sweep is one the next sweep gets.
    async fn sweep_expired_sessions(&self, now: i64) -> Result<(), ServiceError> {
        let expired = sqlx::query(
            "SELECT s.id, l.root_path FROM upload_session s \
             JOIN library l ON l.id = s.library_id \
             WHERE s.expires_at <= ?",
        )
        .bind(now)
        .fetch_all(self.db.pool())
        .await?;
        for row in &expired {
            let session_id = parse_uuid(row.try_get::<String, _>("id")?)?;
            let root: String = row.try_get("root_path")?;
            let path = std::path::Path::new(&root)
                .join(MANAGED_DIR)
                .join(format!("{session_id}.part"));

            let lock = self.session_lock(session_id);
            let held = lock.lock().await;
            // Already gone is the ordinary case — this sweep repeats after a
            // crash, and a session that never sent a fragment has no file.
            if let Err(error) = tokio::fs::remove_file(&path).await {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(%error, session = %session_id, "cannot remove an expired staging file");
                }
            }
            {
                let _writer = self.db.writer_guard().await;
                sqlx::query("DELETE FROM upload_session WHERE id=? AND expires_at <= ?")
                    .bind(session_id.to_string())
                    .bind(now)
                    .execute(self.db.pool())
                    .await?;
            }
            // Released before the entry goes, so a caller arriving between the
            // two waits on a mutex nobody holds rather than on one this sweep
            // is still using.
            drop(held);
            self.upload_locks.remove(&session_id);
        }
        Ok(())
    }

    /// Reads a session and re-decides, from scratch, whether it may continue.
    ///
    /// Every step re-asks: a session is not an authorisation a client carries
    /// away with it. The server already holds this rule for playback, where a
    /// ticket re-checks membership on each redemption so revoking access takes
    /// effect immediately rather than when the ticket expires. A transfer lasts
    /// far longer than a ticket and costs far more, so a member removed or a
    /// library closed mid-transfer must stop writing at the next request.
    async fn session_target(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        now: i64,
    ) -> Result<SessionTarget, ServiceError> {
        let row = sqlx::query(
            "SELECT s.library_id, s.declared_hash, s.declared_size, s.extension, \
                    s.next_chunk, s.received_bytes, s.expires_at, \
                    l.root_path, l.accepts_uploads, m.role \
             FROM upload_session s \
             JOIN library l ON l.id = s.library_id \
             JOIN library_member m ON m.library_id = s.library_id AND m.user_id = s.user_id \
             WHERE s.id = ? AND s.user_id = ?",
        )
        .bind(session_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        let role = crate::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
            .map_err(|_| ServiceError::Invalid)?;
        if !role.may_upload() {
            // Blurred onto 404: someone who may no longer upload learns nothing
            // about a session they may no longer act on.
            return Err(ServiceError::Forbidden);
        }
        if row.try_get::<i64, _>("expires_at")? <= now {
            return Err(ServiceError::NotFound);
        }
        if row.try_get::<i64, _>("accepts_uploads")? == 0 {
            // Not a 404. The caller is still entitled to be here — it is the
            // library that stopped taking files, and saying so is what lets a
            // client stop rather than retry.
            return Err(ServiceError::Conflict);
        }
        Ok(SessionTarget {
            library_id: parse_uuid(row.try_get("library_id")?)?,
            root: std::path::PathBuf::from(row.try_get::<String, _>("root_path")?),
            declared_hash: row.try_get("declared_hash")?,
            declared_size: row.try_get("declared_size")?,
            extension: row.try_get("extension")?,
            next_chunk: row.try_get("next_chunk")?,
            received_bytes: row.try_get("received_bytes")?,
            expires_at: row.try_get("expires_at")?,
        })
    }

    /// How many sessions this process is currently coordinating.
    ///
    /// The locks are created on lookup, so this is also the count of sessions
    /// that have been looked up and not yet finished. It is exposed because the
    /// rule that no lock exists for a session that is not real and not the
    /// caller's has no other visible consequence — an entry that should not be
    /// there costs memory and nothing else, which is precisely the kind of
    /// invariant that rots unwatched.
    pub fn tracked_upload_locks(&self) -> usize {
        self.upload_locks.len()
    }

    fn session_lock(&self, session_id: Uuid) -> Arc<tokio::sync::Mutex<()>> {
        self.upload_locks
            .entry(session_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    fn state_of(&self, session_id: Uuid, target: &SessionTarget) -> UploadSessionState {
        UploadSessionState {
            session_id,
            next_chunk: target.next_chunk,
            received_bytes: target.received_bytes,
            chunk_bytes: self.uploads.chunk_bytes,
            expires_at: target.expires_at,
        }
    }

    /// Where a transfer stands, so a client that restarted can ask instead of
    /// assuming.
    pub async fn upload_session_state(
        &self,
        user_id: Uuid,
        session_id: Uuid,
    ) -> Result<UploadSessionState, ServiceError> {
        let target = self.session_target(user_id, session_id, now_ms()).await?;
        Ok(self.state_of(session_id, &target))
    }

    /// Takes one fragment.
    ///
    /// Three cases, and only one is an error. A fragment already written is
    /// answered idempotently rather than rejected: an acknowledgement lost
    /// after the write is what a dropped link ordinarily produces, and treating
    /// the client's honest retry as a fault would make the protocol fragile
    /// exactly where it exists to absorb interruptions. A fragment from the
    /// future is refused, because the gap it would leave is one only the final
    /// hash could reveal, far too late.
    pub async fn receive_chunk(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        index: i64,
        bytes: &[u8],
    ) -> Result<UploadSessionState, ServiceError> {
        // Resolved before any lock exists for it. `session_lock` inserts on
        // lookup, so locking first would let a caller mint a map entry for
        // every uuid they can type; resolving first bounds the entries to
        // sessions that are real and theirs.
        self.session_target(user_id, session_id, now_ms()).await?;
        let lock = self.session_lock(session_id);
        let _held = lock.lock().await;
        // Read again under the lock. The check above decided whether to take
        // the lock; this one is what the rest of the call acts on, and only it
        // is protected from a sweep or another fragment moving underneath.
        let now = now_ms();
        let target = self.session_target(user_id, session_id, now).await?;
        if index < 0 {
            return Err(ServiceError::Invalid);
        }
        if index < target.next_chunk {
            return Ok(self.state_of(session_id, &target));
        }
        if index > target.next_chunk {
            return Err(ServiceError::Conflict);
        }
        // Exactly what this position calls for, not merely no more than it. A
        // short fragment anywhere but the end would put every later one at the
        // wrong offset, and nothing would notice until the hash did.
        let remaining = target
            .declared_size
            .saturating_sub(target.received_bytes)
            .max(0);
        let expected = remaining.min(self.uploads.chunk_bytes);
        if expected == 0 || i64::try_from(bytes.len()).unwrap_or(i64::MAX) != expected {
            return Err(ServiceError::Invalid);
        }

        let staging = target.staging_path(session_id);
        if let Some(parent) = staging.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                tracing::error!(%error, "cannot create the managed directory");
                ServiceError::Unavailable
            })?;
        }
        // Written before the row moves. A fragment written twice at the same
        // offset is the same file; a row that moved without its bytes is not.
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&staging)
            .await
            .map_err(|error| {
                tracing::error!(%error, "cannot open a staging file");
                ServiceError::Unavailable
            })?;
        {
            use tokio::io::AsyncSeekExt;
            use tokio::io::AsyncWriteExt;
            file.seek(std::io::SeekFrom::Start(
                u64::try_from(target.received_bytes).unwrap_or(0),
            ))
            .await
            .map_err(|_| ServiceError::Unavailable)?;
            file.write_all(bytes)
                .await
                .map_err(|_| ServiceError::Unavailable)?;
            file.sync_data()
                .await
                .map_err(|_| ServiceError::Unavailable)?;
        }

        let _writer = self.db.writer_guard().await;
        let received = target.received_bytes + expected;
        // Guarded on the position this call claimed: if anything moved the
        // session while the bytes were being written, this updates nothing and
        // the client is told where the session actually stands.
        let moved = sqlx::query(
            "UPDATE upload_session SET next_chunk = ?, received_bytes = ? \
             WHERE id = ? AND next_chunk = ?",
        )
        .bind(index + 1)
        .bind(received)
        .bind(session_id.to_string())
        .bind(target.next_chunk)
        .execute(self.db.pool())
        .await?
        .rows_affected()
            == 1;
        drop(_writer);
        if !moved {
            return Err(ServiceError::Conflict);
        }
        Ok(UploadSessionState {
            session_id,
            next_chunk: index + 1,
            received_bytes: received,
            chunk_bytes: self.uploads.chunk_bytes,
            expires_at: target.expires_at,
        })
    }

    /// Turns a completed transfer into a track.
    ///
    /// SQLite and the filesystem share no transaction, so the order below is a
    /// decision rather than a detail. A crash between the rename and the
    /// transaction leaves a file with no catalogue row — which is precisely
    /// what the next scan collects, since the file already carries its final
    /// name. The other order leaves a row pointing at nothing, a state nothing
    /// repairs on its own and every read runs into.
    pub async fn commit_upload(
        &self,
        user_id: Uuid,
        session_id: Uuid,
    ) -> Result<CommittedUpload, ServiceError> {
        // Resolved before the lock, for the same reason as above.
        self.session_target(user_id, session_id, now_ms()).await?;
        let lock = self.session_lock(session_id);
        let _held = lock.lock().await;
        let now = now_ms();
        let target = self.session_target(user_id, session_id, now).await?;
        let staging = target.staging_path(session_id);
        if target.received_bytes != target.declared_size {
            return Err(ServiceError::Invalid);
        }

        // Recomputed, never taken on trust. The declared hash exists to avoid a
        // transfer; letting it establish an identity would let any authorised
        // member pass one file off as another, and turn the deduplication into
        // a means of substitution.
        let hashed = {
            let path = staging.clone();
            tokio::task::spawn_blocking(move || waveflow_core::scanner::hash_file_full(&path))
                .await
                .map_err(|_| ServiceError::Unavailable)?
        };
        let Ok(hash) = hashed else {
            self.abandon(session_id, &staging).await;
            return Err(ServiceError::Unavailable);
        };
        if hash != target.declared_hash {
            // What arrived is not what was promised, and nothing about it is
            // worth keeping.
            self.abandon(session_id, &staging).await;
            return Err(ServiceError::Invalid);
        }

        let relative = target.relative_final(&hash);
        let final_path = target.root.join(&relative);
        tokio::fs::rename(&staging, &final_path)
            .await
            .map_err(|error| {
                tracing::error!(%error, "cannot move a received file into place");
                ServiceError::Unavailable
            })?;

        // The extension was never proof — anything at all can be called .flac.
        // Reading the file is, and reading it through the scan's own extractor
        // is what makes what lands here identical to what a scan would have
        // written. A file that will not open is removed rather than left to
        // occupy a disk the catalogue can never show.
        let input = match self.scanner.read_track_input(&target.root, &relative).await {
            Ok(input) => input,
            Err(error) => {
                tracing::warn!(%error, "a received file is not one the catalogue can read");
                let _ = tokio::fs::remove_file(&final_path).await;
                self.abandon(session_id, &staging).await;
                return Err(ServiceError::Invalid);
            }
        };

        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        // A track may already sit at this name — the same bytes uploaded again
        // after their track went unavailable. Reviving it is what keeps a
        // second row from claiming a path the schema makes unique anyway.
        let existing_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM track WHERE library_id=? AND relative_path=?")
                .bind(target.library_id.to_string())
                .bind(&relative)
                .fetch_optional(&mut *tx)
                .await?;
        crate::database::Database::apply_catalog_track_in_transaction(
            &mut tx,
            self.db.pid(),
            target.library_id,
            // No scan walked for this file. The column says so rather than
            // borrowing an unrelated job's identity, and the end-of-scan sweep
            // knows what that means.
            None,
            &crate::catalog::CatalogApply {
                input,
                existing_id: existing_id.map(parse_uuid).transpose()?,
                moved: false,
            },
            now,
        )
        .await?;
        // Deleted in the same transaction that inserts the track: the space the
        // session was holding becomes the space the file occupies, with no
        // interval during which it is neither. Releasing it and recounting
        // would leave exactly that interval, and two commits would race through
        // it.
        // Read back rather than derived: the apply owns whether this was a new
        // row or a revived one, and the path is what identifies it either way.
        let track_id: String =
            sqlx::query_scalar("SELECT id FROM track WHERE library_id=? AND relative_path=?")
                .bind(target.library_id.to_string())
                .bind(&relative)
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query("DELETE FROM upload_session WHERE id=?")
            .bind(session_id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.upload_locks.remove(&session_id);

        // The apply announced the track on the library feed itself, which is
        // the point of going through it: a received file is a catalogue change
        // like any other, and nothing here had to remember to say so.
        Ok(CommittedUpload {
            track_id: parse_uuid(track_id)?,
            full_hash: hash,
        })
    }

    /// Drops a session and whatever it had accumulated.
    ///
    /// File first, row second, like the sweep: dying in between leaves a row
    /// whose file is gone, which the next sweep repeats harmlessly.
    async fn abandon(&self, session_id: Uuid, staging: &std::path::Path) {
        if let Err(error) = tokio::fs::remove_file(staging).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%error, session = %session_id, "cannot remove a staging file");
            }
        }
        let _writer = self.db.writer_guard().await;
        if let Err(error) = sqlx::query("DELETE FROM upload_session WHERE id=?")
            .bind(session_id.to_string())
            .execute(self.db.pool())
            .await
        {
            tracing::warn!(%error, session = %session_id, "cannot close an abandoned session");
        }
        drop(_writer);
        self.upload_locks.remove(&session_id);
    }
}
