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

    // Right length, right alphabet, but the final character sets padding
    // bits a digest never sets. It decodes to nothing a verifier can match.
    let canonical = challenge_for(&"x".repeat(64));
    const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let last = canonical.chars().last().unwrap();
    let shifted = ALPHABET
        .chars()
        .nth(ALPHABET.chars().position(|c| c == last).unwrap() + 1)
        .unwrap();
    let non_canonical = format!("{}{shifted}", &canonical[..42]);
    assert_eq!(non_canonical.len(), 43);
    assert_eq!(
        validate_challenge("S256", &non_canonical),
        Err(GrantError::UnsupportedChallenge),
        "a non-canonical encoding is not a challenge any verifier can match"
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
