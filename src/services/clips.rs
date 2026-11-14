//! A track's music video, for whoever may play the track (RFC-029 decision 7).

use super::*;

/// What serving a clip needs: where the file is, under the track's library and
/// its link policy, and what to call it. Never sent to a client.
#[derive(Debug, Clone)]
pub struct ClipSource {
    pub track_id: Uuid,
    pub library_id: Uuid,
    pub library_root: std::path::PathBuf,
    pub link_policy: crate::library::link_roots::LinkPolicy,
    /// Relative to `library_root`, as the scan recorded it.
    pub relative_path: String,
    /// `mp4`, `m4v`, `mov`, `webm` or `mkv`.
    pub format: String,
}

impl DomainServices {
    /// The clip a track has now, for somebody who may play the track.
    ///
    /// The tenancy is the audio's, and read through the same query: membership
    /// in the track's library, of an account that is not disabled, re-read on
    /// every request, so a ticket cannot outlive what justified it. The track
    /// must be available too: an unavailable track's folder is usually its
    /// clip's, and the client cannot play the track anyway. Every refusal is
    /// `None`, so a route answers all of them with one 404.
    pub async fn clip_for_track(
        &self,
        user_id: Uuid,
        track_id: Uuid,
    ) -> Result<Option<ClipSource>, ServiceError> {
        let Some(track) = self.db.stream_track_for_user(user_id, track_id).await? else {
            return Ok(None);
        };
        if !track.available {
            return Ok(None);
        }
        let Some(clip) = self.db.clip_of_track(track_id).await? else {
            return Ok(None);
        };
        Ok(Some(ClipSource {
            track_id,
            library_id: track.library_id,
            library_root: track.library_root,
            link_policy: track.link_policy,
            relative_path: clip.relative_path,
            format: clip.format,
        }))
    }
}
