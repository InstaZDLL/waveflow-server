//! Integration tests for the JWT verifier ([`waveflow_server::auth`]).
//!
//! The signing harness + JWKS-server fixture live in the shared
//! [`jwks_harness`] module — [`JwksHarness::spawn`] returns the
//! RSA-2048 / RS256 default, [`JwksHarness::spawn_es256`] returns
//! a P-256 / ES256 mirror, [`JwksHarness::spawn_without_alg`]
//! returns the alg-less JWKS for the RFC 7517 §4.4 fallback test.
//!
//! Why test both RSA and ES256: the verifier's [`build_cached_key`]
//! has separate branches for `AlgorithmParameters::RSA` and
//! `AlgorithmParameters::EllipticCurve`. The RS256 sweep covers
//! Better Auth's default deployment; the ES256 sweep guards the
//! EC branch so a future change can't silently break it (every
//! major auth provider with a JWKS supports ES256, and Better Auth
//! itself offers ES256 as an opt-in).

mod jwks_harness;

use std::time::SystemTime;

use jsonwebtoken::{Algorithm, Header};
use jwks_harness::{good_claims, JwksHarness, TEST_AUDIENCE, TEST_ISSUER, TEST_KID};
use serde_json::json;
use waveflow_server::auth::AuthError;

/// Default `sub` claim across the suite — a stable value lets each
/// test assert on the round-tripped sub without a per-test fixture.
const TEST_SUB: &str = "auth-provider-user-42";

#[tokio::test]
async fn verifies_a_valid_token() {
    let harness = JwksHarness::spawn().await;
    let token = harness.mint(&good_claims(TEST_SUB), &harness.header_with_kid(TEST_KID));

    let verified = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect("token should verify");
    assert_eq!(verified.sub, TEST_SUB);
}

#[tokio::test]
async fn verifies_bearer_prefix() {
    let harness = JwksHarness::spawn().await;
    let token = harness.mint(&good_claims(TEST_SUB), &harness.header_with_kid(TEST_KID));

    let verified = harness
        .verifier()
        .verify_bearer(&format!("Bearer {token}"))
        .await
        .expect("bearer header should verify");
    assert_eq!(verified.sub, TEST_SUB);
}

#[tokio::test]
async fn rejects_expired_token() {
    let harness = JwksHarness::spawn().await;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // jsonwebtoken's default exp leeway is 60s for clock skew, so
    // `exp = now - 60` would actually still verify. Push it well
    // past the leeway window so the rejection is unambiguous.
    let claims = json!({
        "sub": TEST_SUB,
        "iss": TEST_ISSUER,
        "aud": TEST_AUDIENCE,
        "iat": now - 3600,
        "exp": now - 1800,
    });
    let token = harness.mint(&claims, &harness.header_with_kid(TEST_KID));

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("expired token should reject");
    assert!(
        matches!(err, AuthError::InvalidClaims(_)),
        "expected InvalidClaims, got {err:?}"
    );
}

#[tokio::test]
async fn rejects_wrong_issuer() {
    let harness = JwksHarness::spawn().await;
    let mut claims = good_claims(TEST_SUB);
    claims["iss"] = json!("https://evil.example.com");
    let token = harness.mint(&claims, &harness.header_with_kid(TEST_KID));

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("wrong issuer should reject");
    assert!(matches!(err, AuthError::InvalidClaims(_)));
}

#[tokio::test]
async fn rejects_wrong_audience() {
    let harness = JwksHarness::spawn().await;
    let mut claims = good_claims(TEST_SUB);
    claims["aud"] = json!("some-other-product");
    let token = harness.mint(&claims, &harness.header_with_kid(TEST_KID));

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("wrong audience should reject");
    assert!(matches!(err, AuthError::InvalidClaims(_)));
}

#[tokio::test]
async fn rejects_unknown_kid() {
    let harness = JwksHarness::spawn().await;
    let token = harness.mint(
        &good_claims(TEST_SUB),
        &harness.header_with_kid("unknown-kid"),
    );

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("unknown kid should reject");
    assert!(matches!(err, AuthError::KeyNotFound), "got {err:?}");
}

#[tokio::test]
async fn rejects_missing_sub() {
    let harness = JwksHarness::spawn().await;
    let mut claims = good_claims(TEST_SUB);
    claims.as_object_mut().unwrap().remove("sub");
    let token = harness.mint(&claims, &harness.header_with_kid(TEST_KID));

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("missing sub should reject");
    // `sub` is intentionally OUT of `set_required_spec_claims` so
    // jsonwebtoken's structural-claim check can't flatten it into
    // `InvalidClaims` — the discriminated `MissingSub` branch is
    // the only legitimate outcome here.
    assert!(matches!(err, AuthError::MissingSub), "got {err:?}");
}

#[tokio::test]
async fn rejects_blank_sub() {
    let harness = JwksHarness::spawn().await;
    let mut claims = good_claims(TEST_SUB);
    claims["sub"] = json!("   ");
    let token = harness.mint(&claims, &harness.header_with_kid(TEST_KID));

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("blank sub should reject");
    assert!(matches!(err, AuthError::MissingSub), "got {err:?}");
}

#[tokio::test]
async fn rejects_garbage_token() {
    let harness = JwksHarness::spawn().await;
    let err = harness
        .verifier()
        .verify_token("not.a.jwt")
        .await
        .expect_err("garbage should reject");
    assert!(matches!(err, AuthError::MalformedToken), "got {err:?}");
}

#[tokio::test]
async fn rejects_token_without_kid_header() {
    let harness = JwksHarness::spawn().await;
    // A header that explicitly drops the kid.
    let mut header = Header::new(Algorithm::RS256);
    header.kid = None;
    let token = harness.mint(&good_claims(TEST_SUB), &header);

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("token without kid should reject");
    assert!(matches!(err, AuthError::MalformedToken), "got {err:?}");
}

#[tokio::test]
async fn rejects_when_jwks_unreachable() {
    // Bind a port + drop the listener so the URL is reachable in
    // DNS terms but every connect attempt is refused.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    let verifier = waveflow_server::auth::JwtVerifier::with_client(
        waveflow_server::auth::JwtVerifierConfig {
            jwks_url: format!("http://{addr}/.well-known/jwks.json"),
            issuer: TEST_ISSUER.to_string(),
            audience: TEST_AUDIENCE.to_string(),
        },
        client,
    );

    // Mint a syntactically valid token from a throwaway harness —
    // the fetch fails before the verifier reaches the signature
    // check, so it doesn't matter which key signed it.
    let throwaway = JwksHarness::spawn().await;
    let token = throwaway.mint(&good_claims(TEST_SUB), &throwaway.header_with_kid(TEST_KID));

    let err = verifier
        .verify_token(&token)
        .await
        .expect_err("unreachable JWKS should reject");
    assert!(matches!(err, AuthError::JwksFetchFailed(_)), "got {err:?}");
}

/// Reading the same kid twice exercises the cache hit path. The
/// `jwks_request_count` AtomicUsize on the harness proves the second
/// verify didn't refetch (just `1` request after 2 verifies).
#[tokio::test]
async fn cache_hit_skips_second_fetch() {
    let harness = JwksHarness::spawn().await;
    let verifier = harness.verifier();

    let token = harness.mint(&good_claims(TEST_SUB), &harness.header_with_kid(TEST_KID));
    verifier.verify_token(&token).await.expect("first verify");
    verifier
        .verify_token(&token)
        .await
        .expect("second verify (cache hit) should succeed");

    assert_eq!(
        harness.jwks_request_count(),
        1,
        "second verify must hit the cache, not the upstream"
    );
}

/// RFC 7517 §4.4 marks the JWK `alg` parameter as OPTIONAL. The
/// fallback in `build_cached_key` should accept an alg-less RSA key
/// and default to RS256 — without it, an upstream that follows the
/// spec but doesn't bother advertising alg would surface as
/// EmptyJwks → 503.
#[tokio::test]
async fn rsa_jwk_without_alg_defaults_to_rs256() {
    let harness = JwksHarness::spawn_without_alg().await;
    let token = harness.mint(&good_claims(TEST_SUB), &harness.header_with_kid(TEST_KID));

    let verified = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect("RSA alg-less JWK must default to RS256");
    assert_eq!(verified.sub, TEST_SUB);
}

// ─── ES256 / elliptic-curve branch coverage ────────────────────────
//
// Mirrors the RSA happy / reject paths against a P-256 keypair so
// `build_cached_key`'s `AlgorithmParameters::EllipticCurve` branch
// is exercised. We don't re-test every reject variant — the verifier
// is algorithm-agnostic once the cached key is built, so RS256's
// expired / wrong-iss / wrong-aud sweep applies transparently. The
// happy path + alg cross-check + unknown-kid coverage is enough to
// catch a regression in the EC branch.

#[tokio::test]
async fn es256_verifies_a_valid_token() {
    let harness = JwksHarness::spawn_es256().await;
    let token = harness.mint(&good_claims(TEST_SUB), &harness.header_with_kid(TEST_KID));

    let verified = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect("ES256 token should verify");
    assert_eq!(verified.sub, TEST_SUB);
}

#[tokio::test]
async fn es256_rejects_token_signed_with_wrong_key() {
    // Two ES256 harnesses → two independent P-256 keypairs. Sign
    // with A, verify against B → InvalidClaims (signature mismatch).
    let signing_harness = JwksHarness::spawn_es256().await;
    let server_harness = JwksHarness::spawn_es256().await;

    let token = signing_harness.mint(
        &good_claims(TEST_SUB),
        &signing_harness.header_with_kid(TEST_KID),
    );

    let err = server_harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("wrong-key ES256 must reject");
    assert!(matches!(err, AuthError::InvalidClaims(_)), "got {err:?}");
}

#[tokio::test]
async fn es256_rejects_unknown_kid() {
    let harness = JwksHarness::spawn_es256().await;
    let token = harness.mint(
        &good_claims(TEST_SUB),
        &harness.header_with_kid("unknown-kid"),
    );

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("ES256 unknown kid should reject");
    assert!(matches!(err, AuthError::KeyNotFound), "got {err:?}");
}

/// An RS256-signed token presented against an ES256 JWKS must be
/// rejected. The two JWK Sets have no overlapping kids OR algorithms,
/// so the verifier's resolve_kid path returns `KeyNotFound` — same
/// outcome as a confused-deputy attempt to bypass the EC branch with
/// an RSA token. (`AlgorithmMismatch` would require minting a token
/// whose header *lies* about its alg, which jsonwebtoken's `encode`
/// refuses to do — that defense gets exercised in a future PR that
/// builds tokens by hand.)
#[tokio::test]
async fn es256_jwk_rejects_rs256_token() {
    let rsa_harness = JwksHarness::spawn().await;
    let ec_harness = JwksHarness::spawn_es256().await;

    let token = rsa_harness.mint(
        &good_claims(TEST_SUB),
        &rsa_harness.header_with_kid(TEST_KID),
    );

    let err = ec_harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("RS256 token against ES256 JWKS must reject");
    // The kid + alg cross-rejections both produce KeyNotFound — the
    // ES256 JWK Set carries the same TEST_KID but for an EC key, and
    // the alg-mismatch path the verifier *would* normally take on a
    // cached-key mismatch can't fire because the kid match wins on
    // jsonwebtoken's strict per-alg key lookup.
    assert!(
        matches!(err, AuthError::KeyNotFound | AuthError::AlgorithmMismatch),
        "got {err:?}"
    );
}
