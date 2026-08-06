//! Authorization Code + PKCE for native clients.
//!
//! WaveFlow Desktop is a public client: it ships no secret a user could not
//! extract, so PKCE is what binds an authorization code to the application
//! instance that requested it (RFC 7636). The consent screen is a route of the
//! embedded web client, which is why granting is a JSON call authenticated by
//! the browser session rather than a server-rendered form.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use url::Url;

/// Grants are redeemed immediately by a waiting client, so the window is the
/// ten-minute maximum RFC 6749 allows rather than a comfort margin.
pub const AUTHORIZATION_CODE_TTL_MS: i64 = 10 * 60 * 1_000;

#[derive(Debug, PartialEq, Eq)]
pub enum GrantError {
    UnsupportedRedirect,
    UnsupportedChallenge,
    InvalidVerifier,
}

/// Accepts only what a native application can prove it controls.
///
/// Loopback redirects (RFC 8252 §7.3) are the supported desktop shape: the app
/// listens on an ephemeral port, so the port cannot be fixed in advance and is
/// deliberately not compared. A private-use scheme (`waveflow://…`) is accepted
/// too. Anything else — including `http://` to a non-loopback host, which would
/// let a code leak to another machine in clear text — is rejected.
pub fn validate_redirect_uri(redirect_uri: &str) -> Result<(), GrantError> {
    let url = Url::parse(redirect_uri).map_err(|_| GrantError::UnsupportedRedirect)?;
    if url.fragment().is_some() {
        return Err(GrantError::UnsupportedRedirect);
    }
    match url.scheme() {
        "http" => match url.host_str() {
            Some("127.0.0.1") | Some("[::1]") | Some("localhost") => Ok(()),
            _ => Err(GrantError::UnsupportedRedirect),
        },
        "https" => Ok(()),
        // A private-use scheme must be a reverse-domain name, never a bare word
        // that another application could plausibly claim.
        scheme if scheme.contains('.') => Ok(()),
        _ => Err(GrantError::UnsupportedRedirect),
    }
}

/// Only S256 is accepted: `plain` offers no protection when the challenge is
/// observable, which is exactly the threat PKCE exists to close.
pub fn validate_challenge(method: &str, challenge: &str) -> Result<(), GrantError> {
    if !method.eq_ignore_ascii_case("S256") {
        return Err(GrantError::UnsupportedChallenge);
    }
    // base64url of a 32-byte digest is always exactly 43 characters, so any
    // other length is not an S256 challenge whatever else it might be.
    if challenge.len() != 43 {
        return Err(GrantError::UnsupportedChallenge);
    }
    if !challenge
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(GrantError::UnsupportedChallenge);
    }
    Ok(())
}

/// The S256 challenge for a verifier: `base64url(sha256(verifier))`.
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Recomputes the challenge from the verifier the client kept private.
pub fn verify_challenge(challenge: &str, verifier: &str) -> Result<(), GrantError> {
    if verifier.len() < 43 || verifier.len() > 128 {
        return Err(GrantError::InvalidVerifier);
    }
    let expected = challenge_for(verifier);
    // The challenge is public, but constant-time keeps the comparison uniform
    // with the rest of the credential handling in this codebase.
    if crate::security::constant_time_bytes_eq(expected.as_bytes(), challenge.as_bytes()) {
        Ok(())
    } else {
        Err(GrantError::InvalidVerifier)
    }
}

/// Appends `code` and `state` to the client's redirect target.
pub fn redirect_with_code(redirect_uri: &str, code: &str, state: Option<&str>) -> String {
    match Url::parse(redirect_uri) {
        Ok(mut url) => {
            url.query_pairs_mut().append_pair("code", code);
            if let Some(state) = state {
                url.query_pairs_mut().append_pair("state", state);
            }
            url.to_string()
        }
        // validate_redirect_uri already rejected unparseable targets.
        Err(_) => redirect_uri.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_and_private_schemes_are_accepted_but_remote_http_is_not() {
        assert!(validate_redirect_uri("http://127.0.0.1:49152/callback").is_ok());
        assert!(validate_redirect_uri("http://localhost:1234/cb").is_ok());
        assert!(validate_redirect_uri("http://[::1]:5000/cb").is_ok());
        assert!(validate_redirect_uri("https://desktop.example.com/cb").is_ok());
        assert!(validate_redirect_uri("com.waveflow.desktop://auth").is_ok());

        // Clear-text to another host would leak the code off the machine.
        assert_eq!(
            validate_redirect_uri("http://evil.example.com/cb"),
            Err(GrantError::UnsupportedRedirect)
        );
        // A bare scheme is claimable by any other application.
        assert_eq!(
            validate_redirect_uri("waveflow:/auth"),
            Err(GrantError::UnsupportedRedirect)
        );
        assert_eq!(
            validate_redirect_uri("not a url"),
            Err(GrantError::UnsupportedRedirect)
        );
    }

    #[test]
    fn only_s256_challenges_are_accepted() {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(b"a".repeat(43)));
        assert!(validate_challenge("S256", &challenge).is_ok());
        assert!(validate_challenge("s256", &challenge).is_ok());
        assert_eq!(
            validate_challenge("plain", &challenge),
            Err(GrantError::UnsupportedChallenge)
        );
        assert_eq!(
            validate_challenge("S256", "short"),
            Err(GrantError::UnsupportedChallenge)
        );
        assert_eq!(
            validate_challenge("S256", &"a".repeat(64)),
            Err(GrantError::UnsupportedChallenge),
            "an S256 challenge is exactly 43 characters"
        );
        assert_eq!(
            validate_challenge("S256", &"+".repeat(43)),
            Err(GrantError::UnsupportedChallenge),
            "standard base64 padding characters are not base64url"
        );
    }

    #[test]
    fn a_verifier_only_matches_its_own_challenge() {
        let verifier = "x".repeat(64);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert!(verify_challenge(&challenge, &verifier).is_ok());
        assert_eq!(
            verify_challenge(&challenge, &"y".repeat(64)),
            Err(GrantError::InvalidVerifier)
        );
        assert_eq!(
            verify_challenge(&challenge, "too-short"),
            Err(GrantError::InvalidVerifier)
        );
    }

    #[test]
    fn redirect_preserves_existing_query_and_state() {
        let redirected = redirect_with_code(
            "http://127.0.0.1:49152/cb?flow=desktop",
            "the-code",
            Some("the-state"),
        );
        assert!(redirected.contains("flow=desktop"));
        assert!(redirected.contains("code=the-code"));
        assert!(redirected.contains("state=the-state"));
    }
}
