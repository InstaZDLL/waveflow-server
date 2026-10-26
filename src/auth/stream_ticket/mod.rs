//! Short-lived stream authorisation for browser playback.
//!
//! `GET /api/v2/tracks/{id}/stream` requires an `Authorization` header, which an
//! `<audio src>` cannot carry. The web client therefore exchanges its bearer
//! token for a ticket and plays from a URL that authorises itself, the same
//! shape `/share/{token}` already uses.
//!
//! The ticket is the AEAD-sealed quadruple `(kind, user, track, expiry)`:
//! forging one requires the instance key, and it authorises exactly one
//! resource of one kind for one account. Playback still re-checks library
//! membership when the ticket is redeemed, so revoking access takes effect
//! immediately instead of lasting until the ticket expires.
//!
//! The kind is sealed with the rest rather than carried beside it, because a
//! discriminator a caller could choose would discriminate nothing. Every route
//! asserts the kind it expects; see [`TicketKind`] for why `0x00` is not one.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use uuid::Uuid;

use crate::auth::security::{SecretBox, SecurityError};

/// What a ticket opens.
///
/// Without it, a ticket minted for one resource opens the other: audio and a
/// canvas cost three orders of magnitude apart in bandwidth and sit behind the
/// same authorisation, so the client would gain a privilege it never asked for
/// and the operator never granted. RFC-009 decision 3.
///
/// `0x00` is deliberately not a kind. A null byte is what a truncated or
/// half-filled payload produces most readily, and the kind must not be what the
/// error picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketKind {
    Audio,
    Canvas,
    /// One visit to a public share (RFC-020 decision 4). Its first identifier
    /// is the **share**, not an account: an anonymous visitor has none, and a
    /// ticket naming the owner would open the owner's whole catalogue on the
    /// routes that redeem the other two kinds.
    ShareVisit,
}

impl TicketKind {
    const AUDIO: u8 = 0x01;
    const CANVAS: u8 = 0x02;
    const SHARE_VISIT: u8 = 0x03;

    const fn as_byte(self) -> u8 {
        match self {
            Self::Audio => Self::AUDIO,
            Self::Canvas => Self::CANVAS,
            Self::ShareVisit => Self::SHARE_VISIT,
        }
    }

    /// `None` for any byte that is not one of the fixed values — including
    /// `0x00`, and including whatever a later version might add.
    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            Self::AUDIO => Some(Self::Audio),
            Self::CANVAS => Some(Self::Canvas),
            Self::SHARE_VISIT => Some(Self::ShareVisit),
            _ => None,
        }
    }
}

/// `kind (1) || user (16) || track (16) || expiry (8, big-endian epoch millis)`.
///
/// This grew by the leading byte, so tickets minted before that change no
/// longer validate — `verify` checks the exact length. The damage is bounded by
/// the TTL: a browser mid-seek gets a 404 and mints again. It needs no version
/// field, since the payload is sealed under the instance key and never
/// persisted.
const PAYLOAD_LEN: usize = 41;
const NONCE_LEN: usize = 12;

/// Only the known kinds can be minted: the parameter is an enum, so an unknown
/// discriminator is not something a caller can express.
pub fn mint(
    secret: &SecretBox,
    kind: TicketKind,
    user_id: Uuid,
    track_id: Uuid,
    expires_at: i64,
) -> Result<String, SecurityError> {
    let mut payload = Vec::with_capacity(PAYLOAD_LEN);
    payload.push(kind.as_byte());
    payload.extend_from_slice(user_id.as_bytes());
    payload.extend_from_slice(track_id.as_bytes());
    payload.extend_from_slice(&expires_at.to_be_bytes());
    let sealed = secret.encrypt(&payload)?;
    let mut raw = Vec::with_capacity(NONCE_LEN + sealed.ciphertext.len());
    raw.extend_from_slice(&sealed.nonce);
    raw.extend_from_slice(&sealed.ciphertext);
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

/// Returns the authorised `(kind, user, track)`, or `None` when the ticket is
/// malformed, forged, expired or of an unknown kind. Every failure looks the
/// same to the caller so a probe cannot tell "wrong key" from "expired".
///
/// The kind comes back rather than being asserted here: the caller is the route,
/// and the route is the only thing that knows which resource it serves.
pub fn verify(secret: &SecretBox, ticket: &str, now_ms: i64) -> Option<(TicketKind, Uuid, Uuid)> {
    let raw = URL_SAFE_NO_PAD.decode(ticket).ok()?;
    if raw.len() <= NONCE_LEN {
        return None;
    }
    let (nonce, ciphertext) = raw.split_at(NONCE_LEN);
    let payload = secret.decrypt(nonce, ciphertext).ok()?;
    if payload.len() != PAYLOAD_LEN {
        return None;
    }
    let kind = TicketKind::from_byte(payload[0])?;
    let user_id = Uuid::from_slice(&payload[1..17]).ok()?;
    let track_id = Uuid::from_slice(&payload[17..33]).ok()?;
    let expires_at = i64::from_be_bytes(payload[33..41].try_into().ok()?);
    (expires_at > now_ms).then_some((kind, user_id, track_id))
}

#[cfg(test)]
mod tests;
