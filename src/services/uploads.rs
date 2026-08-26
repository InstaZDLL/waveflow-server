//! Deciding whether the server wants a file, before a byte of it moves.
//!
//! Every refusal a received file can earn is cheap exactly once: here, before
//! the transfer. After the last byte, the same refusal has already cost the
//! bandwidth it existed to save. RFC-008 has the reasoning.

use super::*;

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

        let _writer = self.db.writer_guard().await;
        let now = now_ms();
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

        // Expired sessions are swept before anything is counted, or they would
        // hold quota and session slots that nothing returns. Their staging
        // areas go with them once there are staging areas to remove.
        sqlx::query("DELETE FROM upload_session WHERE expires_at <= ?")
            .bind(now)
            .execute(&mut *tx)
            .await?;

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
