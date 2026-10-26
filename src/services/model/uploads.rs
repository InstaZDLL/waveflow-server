//! What a client sends in: upload offers, sessions and verdicts, and the
//! canvas blobs the media surface serves.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// One file a client is offering, before any of its bytes have moved.
///
/// `full_hash` is what the client computed over the whole file. It is used to
/// avoid a transfer and never to establish an identity: the server recomputes
/// it from the bytes it actually received, and an identity founded on what a
/// client asserts would let any authorised member pass one file off as another.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UploadOffer {
    pub full_hash: String,
    pub size_bytes: i64,
    /// Without its dot, and matched case-insensitively against the extensions
    /// the scanner can index. It is not proof of anything — the file is read
    /// for real at commit — but it is what makes a refusal cheap here rather
    /// than after the last byte.
    pub extension: String,
}

/// What the server decided about one offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UploadDecision {
    /// This library already holds these bytes. Scoped to this library on
    /// purpose: a server-wide lookup would tell a member of one library that
    /// some library they cannot see holds exactly this file, and would leave
    /// them believing they had a track they do not have.
    Present,
    /// A session is open and the bytes are wanted.
    Accepted,
    /// The extension is not one the scanner can index, so storing the file
    /// would spend disk on something the catalogue could never show.
    UnsupportedFormat,
    /// Above the per-file ceiling.
    TooLarge,
    /// The library has no room, counting what open sessions have reserved.
    QuotaExceeded,
    /// The caller is entitled to be here and the library does not accept
    /// files. Distinct from a 404, which is what someone not entitled gets.
    LibraryClosed,
    /// The account already holds as many sessions as it may.
    TooManySessions,
}

/// An open session, as the client needs to see it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UploadSessionState {
    pub session_id: Uuid,
    /// The fragment the server wants next. A client that restarts asks rather
    /// than assumes, so a lost acknowledgement costs one request instead of a
    /// transfer.
    pub next_chunk: i64,
    pub received_bytes: i64,
    /// The size to send each fragment at, advertised rather than assumed.
    pub chunk_bytes: i64,
    pub expires_at: i64,
}

/// One blob of the canvas store.
///
/// Cross-cutting rather than private to the canvas service: the media surface
/// serves it, and the hash it carries is both the file name and the `ETag`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub struct CanvasBlob {
    pub hash: String,
    /// `mp4` or `webm`, decided by reading the bytes rather than by trusting
    /// what the request called them.
    pub format: String,
    pub byte_size: i64,
}

impl CanvasBlob {
    /// The name the bytes carry on disk. The hash is the name, so this is a
    /// pure function of the row rather than something stored twice.
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.hash, self.format)
    }
}

/// One verdict, carrying back the hash it answers.
///
/// The hash rather than a position: a client matching verdicts to offers by
/// index is one reordering away from filing a session under the wrong file.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UploadVerdict {
    pub full_hash: String,
    pub decision: UploadDecision,
    /// Set when the decision is `present` — the track that already holds these
    /// bytes, so the client can reconcile without a catalogue sweep.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<Uuid>,
    /// Set when the decision is `accepted`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<UploadSessionState>,
}

/// What a committed upload became.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CommittedUpload {
    /// The track the file became, available immediately rather than at the next
    /// scan.
    pub track_id: Uuid,
    /// Recomputed from the bytes that arrived, never the one the client
    /// declared. At this moment, and only at this moment, both sides know their
    /// file and this track are the same bytes — which is the link a client
    /// would otherwise re-read the whole file to establish.
    pub full_hash: String,
}
