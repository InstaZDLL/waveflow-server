//! Integration tests for the JWT verifier ([`waveflow_server::auth`]).
//!
//! Strategy: stand up a tiny axum server that serves a single JWKS
//! document built from an RSA-2048 keypair generated at test time.
//! The matching private key signs every token the test mints, the
//! verifier is pointed at the mock server's URL, and we exercise
//! each accept / reject branch independently.
//!
//! Why RSA-2048: it's what Better Auth ships with by default
//! (RS256). Per-test keygen is ~50 ms which is fine; cache + reuse
//! would buy us nothing because each `#[tokio::test]` runs in its
//! own process state.

use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::SystemTime,
};

use axum::{routing::get, Json, Router};
use jsonwebtoken::{
    encode,
    jwk::{
        AlgorithmParameters, CommonParameters, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse,
        RSAKeyParameters, RSAKeyType,
    },
    Algorithm, EncodingKey, Header,
};
use rsa::{pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts, RsaPrivateKey, RsaPublicKey};
use serde::Serialize;
use serde_json::json;
use waveflow_server::auth::{AuthError, JwtVerifier, JwtVerifierConfig};

const TEST_ISSUER: &str = "https://auth.test.example.com";
const TEST_AUDIENCE: &str = "waveflow-server-test";
const TEST_KID: &str = "test-key-1";

/// Test fixture — the bits a test needs to mint signed tokens AND
/// point the verifier at the matching JWKS endpoint.
struct AuthHarness {
    jwks_url: String,
    encoding_key: EncodingKey,
    /// Bumped by the mock JWKS handler on each inbound request. The
    /// `cache_hit_skips_second_fetch` test asserts this stays at `1`
    /// after two verifies for the same `kid` — proves the cache hit
    /// path skips the HTTP round-trip, not just that it succeeds.
    jwks_request_count: Arc<AtomicUsize>,
}

impl AuthHarness {
    /// Bootstrap with a JWK Set whose `alg` field is populated
    /// (`RS256`). The standard happy-path harness.
    async fn spawn() -> Self {
        Self::spawn_with(true).await
    }

    /// Bootstrap with a JWK Set whose `alg` field is OMITTED (RFC
    /// 7517 §4.4 marks `alg` as optional). Used to verify the
    /// RSA-no-alg fallback in `build_cached_key`.
    async fn spawn_without_alg() -> Self {
        Self::spawn_with(false).await
    }

    /// Bootstrap: generate a keypair, publish the public half on a
    /// background axum server, return the URL + the private key.
    /// Drop semantics: the server keeps running for the test's life
    /// (no explicit shutdown — the OS reaps the socket when the
    /// process exits, which is fine for in-process integration
    /// tests).
    async fn spawn_with(advertise_alg: bool) -> Self {
        // 2048-bit is the Better Auth default. Smaller (1024) is
        // faster to generate but would teach the test suite a habit
        // we don't want to carry into prod.
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen failed");
        let public = RsaPublicKey::from(&private);

        let encoding_key = {
            let pem = private
                .to_pkcs1_pem(Default::default())
                .expect("pem encode");
            EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key from pem")
        };

        let jwks = build_jwks(&public, advertise_alg);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let jwks_request_count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&jwks_request_count);

        let app = Router::new().route(
            "/.well-known/jwks.json",
            get(move || {
                let jwks = jwks.clone();
                let counter = Arc::clone(&counter);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Json(jwks)
                }
            }),
        );

        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        Self {
            jwks_url: format!("http://{addr}/.well-known/jwks.json"),
            encoding_key,
            jwks_request_count,
        }
    }

    /// How many times the mock JWKS endpoint has been hit since the
    /// harness was spawned. The cache test uses this to prove that
    /// the second verify on a cached kid doesn't refetch.
    fn jwks_request_count(&self) -> usize {
        self.jwks_request_count.load(Ordering::SeqCst)
    }

    /// Mint a signed token from explicit claims. Returns the
    /// compact-serialised JWT.
    fn mint(&self, claims: &impl Serialize, header: &Header) -> String {
        encode(header, claims, &self.encoding_key).expect("token encode failed")
    }

    /// Verifier wired against the harness's mock JWKS, with the
    /// shared `iss` / `aud` config.
    fn verifier(&self) -> JwtVerifier {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("http client");
        JwtVerifier::with_client(
            JwtVerifierConfig {
                jwks_url: self.jwks_url.clone(),
                issuer: TEST_ISSUER.to_string(),
                audience: TEST_AUDIENCE.to_string(),
            },
            client,
        )
    }
}

fn build_jwks(public: &RsaPublicKey, advertise_alg: bool) -> JwkSet {
    let n_b64 = base64_url(&public.n().to_bytes_be());
    let e_b64 = base64_url(&public.e().to_bytes_be());

    JwkSet {
        keys: vec![Jwk {
            common: CommonParameters {
                public_key_use: Some(PublicKeyUse::Signature),
                key_algorithm: advertise_alg.then_some(KeyAlgorithm::RS256),
                key_id: Some(TEST_KID.to_string()),
                ..Default::default()
            },
            algorithm: AlgorithmParameters::RSA(RSAKeyParameters {
                key_type: RSAKeyType::RSA,
                n: n_b64,
                e: e_b64,
            }),
        }],
    }
}

fn base64_url(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD.encode(bytes)
}

fn header_with_kid(kid: &str) -> Header {
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some(kid.to_string());
    h
}

/// Default claims for the happy-path test. `exp` is 5 min in the
/// future, `nbf` is `iat`. All times are unix epoch seconds (JWT
/// spec — distinct from the rest of the codebase's millisecond
/// timestamps).
fn good_claims() -> serde_json::Value {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    json!({
        "sub": "auth-provider-user-42",
        "iss": TEST_ISSUER,
        "aud": TEST_AUDIENCE,
        "iat": now,
        "nbf": now,
        "exp": now + 300,
    })
}

#[tokio::test]
async fn verifies_a_valid_token() {
    let harness = AuthHarness::spawn().await;
    let token = harness.mint(&good_claims(), &header_with_kid(TEST_KID));

    let verified = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect("token should verify");
    assert_eq!(verified.sub, "auth-provider-user-42");
}

#[tokio::test]
async fn verifies_bearer_prefix() {
    let harness = AuthHarness::spawn().await;
    let token = harness.mint(&good_claims(), &header_with_kid(TEST_KID));

    let verified = harness
        .verifier()
        .verify_bearer(&format!("Bearer {token}"))
        .await
        .expect("bearer header should verify");
    assert_eq!(verified.sub, "auth-provider-user-42");
}

#[tokio::test]
async fn rejects_expired_token() {
    let harness = AuthHarness::spawn().await;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // jsonwebtoken's default exp leeway is 60s for clock skew, so
    // `exp = now - 60` would actually still verify. Push it well
    // past the leeway window so the rejection is unambiguous.
    let claims = json!({
        "sub": "auth-provider-user-42",
        "iss": TEST_ISSUER,
        "aud": TEST_AUDIENCE,
        "iat": now - 3600,
        "exp": now - 1800,
    });
    let token = harness.mint(&claims, &header_with_kid(TEST_KID));

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
    let harness = AuthHarness::spawn().await;
    let mut claims = good_claims();
    claims["iss"] = json!("https://evil.example.com");
    let token = harness.mint(&claims, &header_with_kid(TEST_KID));

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("wrong issuer should reject");
    assert!(matches!(err, AuthError::InvalidClaims(_)));
}

#[tokio::test]
async fn rejects_wrong_audience() {
    let harness = AuthHarness::spawn().await;
    let mut claims = good_claims();
    claims["aud"] = json!("some-other-product");
    let token = harness.mint(&claims, &header_with_kid(TEST_KID));

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("wrong audience should reject");
    assert!(matches!(err, AuthError::InvalidClaims(_)));
}

#[tokio::test]
async fn rejects_unknown_kid() {
    let harness = AuthHarness::spawn().await;
    let token = harness.mint(&good_claims(), &header_with_kid("unknown-kid"));

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("unknown kid should reject");
    assert!(matches!(err, AuthError::KeyNotFound), "got {err:?}");
}

#[tokio::test]
async fn rejects_missing_sub() {
    let harness = AuthHarness::spawn().await;
    let mut claims = good_claims();
    claims.as_object_mut().unwrap().remove("sub");
    let token = harness.mint(&claims, &header_with_kid(TEST_KID));

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
    let harness = AuthHarness::spawn().await;
    let mut claims = good_claims();
    claims["sub"] = json!("   ");
    let token = harness.mint(&claims, &header_with_kid(TEST_KID));

    let err = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect_err("blank sub should reject");
    assert!(matches!(err, AuthError::MissingSub), "got {err:?}");
}

#[tokio::test]
async fn rejects_garbage_token() {
    let harness = AuthHarness::spawn().await;
    let err = harness
        .verifier()
        .verify_token("not.a.jwt")
        .await
        .expect_err("garbage should reject");
    assert!(matches!(err, AuthError::MalformedToken), "got {err:?}");
}

#[tokio::test]
async fn rejects_token_without_kid_header() {
    let harness = AuthHarness::spawn().await;
    // A header that explicitly drops the kid.
    let mut header = Header::new(Algorithm::RS256);
    header.kid = None;
    let token = harness.mint(&good_claims(), &header);

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
    let verifier = JwtVerifier::with_client(
        JwtVerifierConfig {
            jwks_url: format!("http://{addr}/.well-known/jwks.json"),
            issuer: TEST_ISSUER.to_string(),
            audience: TEST_AUDIENCE.to_string(),
        },
        client,
    );

    // Mint a syntactically valid token (no need for it to verify —
    // the fetch fails before the verifier reaches the signature
    // check).
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(TEST_KID.to_string());
    let dummy_private = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
    let pem = dummy_private.to_pkcs1_pem(Default::default()).unwrap();
    let encoding = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    let token = encode(&header, &good_claims(), &encoding).unwrap();

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
    let harness = AuthHarness::spawn().await;
    let verifier = harness.verifier();

    let token = harness.mint(&good_claims(), &header_with_kid(TEST_KID));
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
/// EmptyJwks → 503. Verifying an RS256 token against such a key
/// proves the fallback wires the right `Algorithm` for the
/// signature check.
#[tokio::test]
async fn rsa_jwk_without_alg_defaults_to_rs256() {
    let harness = AuthHarness::spawn_without_alg().await;
    let token = harness.mint(&good_claims(), &header_with_kid(TEST_KID));

    let verified = harness
        .verifier()
        .verify_token(&token)
        .await
        .expect("RSA alg-less JWK must default to RS256");
    assert_eq!(verified.sub, "auth-provider-user-42");
}
