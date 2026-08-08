//! Short-lived stream authorisation for browser playback.
//!
//! `GET /api/v2/tracks/{id}/stream` requires an `Authorization` header, which an
//! `<audio src>` cannot carry. The web client therefore exchanges its bearer
//! token for a ticket and plays from a URL that authorises itself, the same
//! shape `/share/{token}` already uses.
//!
//! The ticket is the AEAD-sealed triple `(user, track, expiry)`: forging one
//! requires the instance key, and it authorises exactly one track for one
//! account. Playback still re-checks library membership when the ticket is
//! redeemed, so revoking access takes effect immediately instead of lasting
//! until the ticket expires.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use uuid::Uuid;

use crate::security::{SecretBox, SecurityError};

/// `user (16) || track (16) || expiry (8, big-endian epoch millis)`.
const PAYLOAD_LEN: usize = 40;
const NONCE_LEN: usize = 12;

pub fn mint(
    secret: &SecretBox,
    user_id: Uuid,
    track_id: Uuid,
    expires_at: i64,
) -> Result<String, SecurityError> {
    let mut payload = Vec::with_capacity(PAYLOAD_LEN);
    payload.extend_from_slice(user_id.as_bytes());
    payload.extend_from_slice(track_id.as_bytes());
    payload.extend_from_slice(&expires_at.to_be_bytes());
    let sealed = secret.encrypt(&payload)?;
    let mut raw = Vec::with_capacity(NONCE_LEN + sealed.ciphertext.len());
    raw.extend_from_slice(&sealed.nonce);
    raw.extend_from_slice(&sealed.ciphertext);
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

/// Returns the authorised `(user, track)` pair, or `None` when the ticket is
/// malformed, forged or expired. Every failure looks the same to the caller so
/// a probe cannot tell "wrong key" from "expired".
pub fn verify(secret: &SecretBox, ticket: &str, now_ms: i64) -> Option<(Uuid, Uuid)> {
    let raw = URL_SAFE_NO_PAD.decode(ticket).ok()?;
    if raw.len() <= NONCE_LEN {
        return None;
    }
    let (nonce, ciphertext) = raw.split_at(NONCE_LEN);
    let payload = secret.decrypt(nonce, ciphertext).ok()?;
    if payload.len() != PAYLOAD_LEN {
        return None;
    }
    let user_id = Uuid::from_slice(&payload[..16]).ok()?;
    let track_id = Uuid::from_slice(&payload[16..32]).ok()?;
    let expires_at = i64::from_be_bytes(payload[32..40].try_into().ok()?);
    (expires_at > now_ms).then_some((user_id, track_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret() -> SecretBox {
        SecretBox::from_key_bytes(&[7u8; 32]).unwrap()
    }

    #[test]
    fn round_trip_returns_the_authorised_pair() {
        let secret = secret();
        let user = Uuid::new_v4();
        let track = Uuid::new_v4();
        let ticket = mint(&secret, user, track, 1_000).unwrap();
        assert_eq!(verify(&secret, &ticket, 999), Some((user, track)));
    }

    #[test]
    fn expired_forged_and_malformed_tickets_are_all_rejected() {
        let secret = secret();
        let ticket = mint(&secret, Uuid::new_v4(), Uuid::new_v4(), 1_000).unwrap();
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
