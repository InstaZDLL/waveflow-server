//! Deciding whether the server wants a file, before a byte of it moves.
//!
//! Every refusal a received file can earn is cheap exactly once: here, before
//! the transfer. After the last byte, the same refusal has already cost the
//! bandwidth it existed to save. RFC-008 has the reasoning.

use super::*;

mod negotiate;
mod sweep;
mod transfer;

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

fn refused(full_hash: String, decision: UploadDecision) -> UploadVerdict {
    UploadVerdict {
        full_hash,
        decision,
        track_id: None,
        session: None,
    }
}

impl DomainServices {
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
}
