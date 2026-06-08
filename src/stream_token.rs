//! Signed-URL tokens for the streaming endpoint.
//!
//! The mint endpoint (`POST /api/v1/profiles/{p}/libraries/{l}/tracks/{t}/stream-url`,
//! JWT-authed) verifies tenant ownership, then issues a short-lived
//! token a browser can drop into `<audio src>` without attaching any
//! header. The stream endpoint (`GET /api/v1/stream/{token}`)
//! validates the token cryptographically and serves the file — no DB
//! lookup on the hot path.
//!
//! Token shape: `<base64url(payload_json)>.<base64url(hmac_sha256(secret, payload_json))>`
//! where `payload_json` is `{ "p": "<file_path>", "exp": <unix_seconds> }`.
//!
//! Constant-time HMAC comparison via `Hmac::verify_slice` guards
//! against signature-timing oracles. Expiry is checked separately —
//! a token with a tampered `exp` would fail the signature check
//! first, so a leak of one URL can't be extended just by re-encoding
//! the expiry.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
// hmac 0.13 split `new_from_slice` out behind the `KeyInit` trait,
// so the bare `Hmac::new_from_slice` call below now needs the trait
// in scope alongside `Mac`.
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Maximum validity window we'll ever mint. Caps a leaked URL at
/// ~one minute of usefulness regardless of what the caller asks for;
/// also bounds the clock-skew risk on the verifying side.
pub const MAX_LIFETIME_SECS: u64 = 60;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamClaim {
    /// File path relative to `WAVEFLOW_MUSIC_ROOT`. The stream
    /// handler canonicalises this and verifies it still lives under
    /// the music root; this struct only carries the string the mint
    /// endpoint signed.
    pub p: String,
    /// Expiry as a unix epoch second.
    pub exp: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum StreamTokenError {
    #[error("token missing the signature separator")]
    Shape,
    #[error("base64 decode failed")]
    Base64,
    #[error("payload is not valid JSON")]
    Payload,
    #[error("signature mismatch")]
    Signature,
    #[error("token has expired")]
    Expired,
    #[error("HMAC key initialisation failed")]
    Hmac,
}

/// Sign a claim with the supplied secret and return the URL-safe
/// token string. The secret is whatever bytes `WAVEFLOW_STREAM_SECRET`
/// resolves to at boot — `Hmac::new_from_slice` accepts any length
/// and pads/truncates internally, so we don't bake a key-size
/// requirement into the API.
pub fn mint(secret: &[u8], claim: &StreamClaim) -> Result<String, StreamTokenError> {
    let payload = serde_json::to_vec(claim).map_err(|_| StreamTokenError::Payload)?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| StreamTokenError::Hmac)?;
    mac.update(&payload);
    let sig = mac.finalize().into_bytes();

    let mut out = String::with_capacity(payload.len() + sig.len());
    URL_SAFE_NO_PAD.encode_string(&payload, &mut out);
    out.push('.');
    URL_SAFE_NO_PAD.encode_string(sig, &mut out);
    Ok(out)
}

/// Verify a token, returning the claim if (and only if) the
/// signature matches AND the expiry hasn't passed. `now` is supplied
/// by the caller so tests can pin a deterministic clock.
pub fn verify(secret: &[u8], token: &str, now: u64) -> Result<StreamClaim, StreamTokenError> {
    let (payload_b64, sig_b64) = token.split_once('.').ok_or(StreamTokenError::Shape)?;

    let payload = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| StreamTokenError::Base64)?;
    let sig = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| StreamTokenError::Base64)?;

    // Constant-time MAC verify before parsing the payload — keeps
    // the JSON parser away from arbitrary unauthenticated bytes.
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| StreamTokenError::Hmac)?;
    mac.update(&payload);
    mac.verify_slice(&sig)
        .map_err(|_| StreamTokenError::Signature)?;

    let claim: StreamClaim =
        serde_json::from_slice(&payload).map_err(|_| StreamTokenError::Payload)?;
    if claim.exp <= now {
        return Err(StreamTokenError::Expired);
    }
    Ok(claim)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"unit-test-secret-please-rotate";

    fn claim(p: &str, exp: u64) -> StreamClaim {
        StreamClaim {
            p: p.to_string(),
            exp,
        }
    }

    #[test]
    fn round_trips_a_valid_token() {
        let token = mint(SECRET, &claim("Music/song.flac", 1_000)).unwrap();
        let parsed = verify(SECRET, &token, 999).unwrap();
        assert_eq!(parsed.p, "Music/song.flac");
        assert_eq!(parsed.exp, 1_000);
    }

    #[test]
    fn rejects_a_tampered_payload() {
        let token = mint(SECRET, &claim("Music/song.flac", 1_000)).unwrap();
        let (_, sig) = token.split_once('.').unwrap();
        // Swap the path to a different file but keep the signature
        // from the original — the MAC must reject.
        let mut payload = String::new();
        URL_SAFE_NO_PAD.encode_string(
            serde_json::to_vec(&claim("Music/other.flac", 1_000)).unwrap(),
            &mut payload,
        );
        let tampered = format!("{payload}.{sig}");
        assert!(matches!(
            verify(SECRET, &tampered, 999),
            Err(StreamTokenError::Signature)
        ));
    }

    #[test]
    fn rejects_a_tampered_expiry() {
        let token = mint(SECRET, &claim("Music/song.flac", 1_000)).unwrap();
        let (_, sig) = token.split_once('.').unwrap();
        // Extend the expiry by re-encoding the payload — same as
        // tampering the path, the MAC catches it.
        let mut payload = String::new();
        URL_SAFE_NO_PAD.encode_string(
            serde_json::to_vec(&claim("Music/song.flac", 9_999_999_999)).unwrap(),
            &mut payload,
        );
        let tampered = format!("{payload}.{sig}");
        assert!(matches!(
            verify(SECRET, &tampered, 999),
            Err(StreamTokenError::Signature)
        ));
    }

    #[test]
    fn rejects_an_expired_token() {
        let token = mint(SECRET, &claim("Music/song.flac", 1_000)).unwrap();
        assert!(matches!(
            verify(SECRET, &token, 1_000),
            Err(StreamTokenError::Expired)
        ));
        assert!(matches!(
            verify(SECRET, &token, 1_001),
            Err(StreamTokenError::Expired)
        ));
    }

    #[test]
    fn rejects_a_token_signed_with_a_different_secret() {
        let token = mint(SECRET, &claim("Music/song.flac", 1_000)).unwrap();
        assert!(matches!(
            verify(b"different-secret", &token, 999),
            Err(StreamTokenError::Signature)
        ));
    }

    #[test]
    fn rejects_shape_errors() {
        assert!(matches!(
            verify(SECRET, "no-dot", 999),
            Err(StreamTokenError::Shape)
        ));
        assert!(matches!(
            verify(SECRET, "not_base64.also_not", 999),
            Err(StreamTokenError::Base64)
        ));
    }
}
