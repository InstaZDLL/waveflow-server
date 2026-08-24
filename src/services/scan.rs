//! Library scan jobs.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    /// Queues a rescan of one library the user can reach.
    ///
    /// The single implementation behind `POST /api/v2/libraries/{id}/scans`
    /// and the Subsonic `startScan`. Both surfaces have to answer the same
    /// question about who may scan what, so the membership check cannot sit
    /// in a handler where the two copies can drift apart.
    pub async fn start_library_scan(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<Uuid, ServiceError> {
        let library = self
            .db
            .library_for_user(user_id, library_id)
            .await?
            .ok_or(ServiceError::NotFound)?;
        // The lookup above reads the root path; it is not what authorises the
        // job. The insert tests `library_member` itself — membership and role
        // together — so an access revoked or downgraded between the two refuses
        // the job instead of queuing work the requester may no longer ask for.
        let scan_id = self
            .db
            .create_scan_job_for_user(user_id, library_id, "manual")
            .await?
            .ok_or(ServiceError::NotFound)?;
        self.scanner.spawn(scan_id, library);
        Ok(scan_id)
    }

    /// Queues a rescan of every library the user may scan, for the Subsonic
    /// `startScan`, which takes no library parameter.
    ///
    /// Libraries the account only listens to are skipped rather than attempted
    /// and reported: `startScan` names no library, so refusing the whole call
    /// because one of the account's libraries is read-only would put the
    /// scannable ones out of reach from Subsonic entirely.
    ///
    /// An account that may scan nothing therefore queues nothing and succeeds,
    /// like an account that reaches no library at all: there is no missing
    /// resource to report, and every other catalogue-wide method answers such
    /// an account with an empty result rather than an error.
    ///
    /// Best effort by design: a library whose job cannot be queued does not
    /// cancel the ones that can. Aborting on the first failure would leave
    /// the caller reading an error while half the catalogue is already
    /// rescanning, which is the worst of both answers. The error surfaces
    /// only when nothing at all could be queued.
    ///
    /// Re-queuing a library that is already scanning is deliberately allowed,
    /// exactly as calling the native endpoint twice is: [`crate::scanner::ScanManager`]
    /// serialises jobs per library and a scan converges on file content, so a
    /// redundant pass costs time and changes nothing.
    pub async fn start_visible_scans(&self, user_id: Uuid) -> Result<Vec<Uuid>, ServiceError> {
        let libraries = self.db.libraries_for_user(user_id).await?;
        let mut queued = Vec::new();
        let mut failure = None;
        for access in libraries
            .into_iter()
            .filter(|access| access.role.may_scan())
        {
            match self.start_library_scan(user_id, access.id).await {
                Ok(scan_id) => queued.push(scan_id),
                Err(error) => failure = Some(error),
            }
        }
        match failure {
            Some(error) if queued.is_empty() => Err(error),
            _ => Ok(queued),
        }
    }
}
