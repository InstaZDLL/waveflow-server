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
