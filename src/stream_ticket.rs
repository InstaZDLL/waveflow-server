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

use crate::security::{SecretBox, SecurityError};

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
}

impl TicketKind {
    const AUDIO: u8 = 0x01;
    const CANVAS: u8 = 0x02;

    const fn as_byte(self) -> u8 {
        match self {
            Self::Audio => Self::AUDIO,
            Self::Canvas => Self::CANVAS,
        }
    }

    /// `None` for any byte that is not one of the two fixed values — including
    /// `0x00`, and including whatever a later version might add.
    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            Self::AUDIO => Some(Self::Audio),
            Self::CANVAS => Some(Self::Canvas),
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

/// Only the two kinds can be minted: the parameter is an enum, so an unknown
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
mod tests {
    use super::*;

    fn secret() -> SecretBox {
        SecretBox::from_key_bytes(&[7u8; 32]).unwrap()
    }

    /// Seals a payload the public API cannot express, so the test can present a
    /// ticket that is genuine in every respect except its kind.
    fn seal(secret: &SecretBox, payload: &[u8]) -> String {
        let sealed = secret.encrypt(payload).unwrap();
        let mut raw = Vec::with_capacity(NONCE_LEN + sealed.ciphertext.len());
        raw.extend_from_slice(&sealed.nonce);
        raw.extend_from_slice(&sealed.ciphertext);
        URL_SAFE_NO_PAD.encode(raw)
    }

    fn payload_with_kind(byte: u8, user: Uuid, track: Uuid, expires_at: i64) -> Vec<u8> {
        let mut payload = Vec::with_capacity(PAYLOAD_LEN);
        payload.push(byte);
        payload.extend_from_slice(user.as_bytes());
        payload.extend_from_slice(track.as_bytes());
        payload.extend_from_slice(&expires_at.to_be_bytes());
        payload
    }

    #[test]
    fn round_trip_returns_the_authorised_triple() {
        let secret = secret();
        for kind in [TicketKind::Audio, TicketKind::Canvas] {
            let user = Uuid::new_v4();
            let track = Uuid::new_v4();
            let ticket = mint(&secret, kind, user, track, 1_000).unwrap();
            assert_eq!(verify(&secret, &ticket, 999), Some((kind, user, track)));
        }
    }

    #[test]
    fn the_two_kinds_do_not_answer_for_each_other() {
        let secret = secret();
        let user = Uuid::new_v4();
        let track = Uuid::new_v4();
        let audio = mint(&secret, TicketKind::Audio, user, track, 1_000).unwrap();
        let canvas = mint(&secret, TicketKind::Canvas, user, track, 1_000).unwrap();
        assert_ne!(audio, canvas, "the kind is inside the sealed payload");
        assert_eq!(
            verify(&secret, &audio, 999).map(|(kind, ..)| kind),
            Some(TicketKind::Audio)
        );
        assert_eq!(
            verify(&secret, &canvas, 999).map(|(kind, ..)| kind),
            Some(TicketKind::Canvas)
        );
    }

    #[test]
    fn an_unknown_kind_is_refused_even_under_the_right_key() {
        let secret = secret();
        let user = Uuid::new_v4();
        let track = Uuid::new_v4();
        // Sealed under the real key, correct length, live expiry: the kind byte
        // is the only thing wrong with it. A forged ticket would prove nothing
        // about this contract, since it already fails to decrypt.
        for byte in [0x00, 0x03, 0xff] {
            let ticket = seal(&secret, &payload_with_kind(byte, user, track, 1_000));
            assert_eq!(
                verify(&secret, &ticket, 999),
                None,
                "kind {byte:#04x} is not one of the two"
            );
        }
    }

    #[test]
    fn a_payload_of_the_previous_length_is_refused() {
        let secret = secret();
        // What `mint` produced before the kind byte existed: user || track ||
        // expiry, forty bytes, sealed under the current key.
        let mut payload = Vec::with_capacity(40);
        payload.extend_from_slice(Uuid::new_v4().as_bytes());
        payload.extend_from_slice(Uuid::new_v4().as_bytes());
        payload.extend_from_slice(&1_000i64.to_be_bytes());
        let ticket = seal(&secret, &payload);
        assert_eq!(verify(&secret, &ticket, 999), None);
    }

    #[test]
    fn expired_forged_and_malformed_tickets_are_all_rejected() {
        let secret = secret();
        let ticket = mint(
            &secret,
            TicketKind::Audio,
            Uuid::new_v4(),
            Uuid::new_v4(),
            1_000,
        )
        .unwrap();
        assert_eq!(verify(&secret, &ticket, 1_000), None, "expiry is exclusive");
        assert_eq!(verify(&secret, &ticket, 1_001), None);

        let other = SecretBox::from_key_bytes(&[9u8; 32]).unwrap();
        assert_eq!(
            verify(&other, &ticket, 0),
            None,
            "a ticket minted elsewhere must not validate"
        );

        assert_eq!(verify(&secret, "not-base64!", 0), None);
        assert_eq!(verify(&secret, "", 0), None);
        let mut tampered = URL_SAFE_NO_PAD.decode(&ticket).unwrap();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xff;
        assert_eq!(
            verify(&secret, &URL_SAFE_NO_PAD.encode(tampered), 0),
            None,
            "AEAD must reject a flipped bit"
        );
    }
}
