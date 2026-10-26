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

/// Where a session's fragments accumulate: the destination directory itself,
/// under a name carrying no audio extension.
///
/// Both halves matter. Same directory means same filesystem, so the rename
/// at commit is atomic rather than a copy with a window during which a
/// truncated file exists. And the walk skips it not because it is hidden —
/// nothing about the walk skips hidden directories — but because it filters
/// on extension, which is a rule that already exists and cannot be broken
/// by someone rewriting the traversal.
///
/// One function for the transfer that writes there and the sweep that cleans
/// up after it: two spellings of the name could drift, and a sweep looking
/// under the wrong one drops the session and leaves the file.
fn staging_path(root: &std::path::Path, session_id: Uuid) -> std::path::PathBuf {
    root.join(MANAGED_DIR).join(format!("{session_id}.part"))
}

/// The name a file keeps, once the bytes are known to be what was promised.
///
/// By hash rather than by tags: tags move — a correction, a retag, a later
/// re-identification — and a path built from them would leave the server
/// choosing between moving the file and keeping a name that lies. The
/// project already answered that question in the other direction, where a
/// correction never rewrites the file. It keeps its extension because the
/// walk recognises a file by nothing else.
fn relative_final(hash: &str, extension: &str) -> String {
    format!("{MANAGED_DIR}/{hash}.{extension}")
}

impl DomainServices {
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
}
