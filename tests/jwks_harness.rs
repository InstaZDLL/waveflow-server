//! Shared JWKS test harness — spawns a mock JWKS endpoint backed by
//! a keypair generated at construction time (RSA-2048 / RS256 by
//! default; ES256 / P-256 via [`JwksHarness::spawn_es256`]) and
//! exposes both a `JwtVerifier` pointed at it and a `mint(claims)`
//! helper that signs tokens with the matching private key.
//!
//! Cargo's integration tests each compile as their own crate, so
//! this file is included via `mod jwks_harness;` from the consumers
//! (`tests/auth.rs`, `tests/jwt_middleware.rs`) — Cargo also
//! compiles it as a standalone "empty" test binary, hence the
//! `#![allow(dead_code)]` blanket.

#![allow(dead_code)]

use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

use axum::{routing::get, Json, Router};
use jsonwebtoken::{
    encode,
    jwk::{
        AlgorithmParameters, CommonParameters, EllipticCurve, EllipticCurveKeyParameters,
        EllipticCurveKeyType, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse, RSAKeyParameters,
        RSAKeyType,
    },
    Algorithm, EncodingKey, Header,
};
use p256::{
    elliptic_curve::sec1::ToEncodedPoint, pkcs8::EncodePrivateKey, SecretKey as P256Secret,
};
use rsa::{pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts, RsaPrivateKey, RsaPublicKey};
use serde::Serialize;
use serde_json::json;
use waveflow_server::auth::{JwtVerifier, JwtVerifierConfig};

pub const TEST_ISSUER: &str = "https://auth.test.example.com";
pub const TEST_AUDIENCE: &str = "waveflow-server-test";
pub const TEST_KID: &str = "test-key-1";

/// JWS algorithm the harness uses to sign tokens AND advertise in
/// the JWK. Picks the right `Algorithm` for both the test's
/// `Header::new(...)` and the verifier's algorithm cross-check.
#[derive(Debug, Clone, Copy)]
pub enum HarnessAlg {
    Rs256,
    Es256,
}

impl HarnessAlg {
    pub fn algorithm(self) -> Algorithm {
        match self {
            HarnessAlg::Rs256 => Algorithm::RS256,
            HarnessAlg::Es256 => Algorithm::ES256,
        }
    }

    fn key_algorithm(self) -> KeyAlgorithm {
        match self {
            HarnessAlg::Rs256 => KeyAlgorithm::RS256,
            HarnessAlg::Es256 => KeyAlgorithm::ES256,
        }
    }
}

pub struct JwksHarness {
    pub jwks_url: String,
    pub encoding_key: EncodingKey,
    pub alg: HarnessAlg,
    pub jwks_request_count: Arc<AtomicUsize>,
}

impl JwksHarness {
    /// RSA-2048 / RS256 — Better Auth's default. Most tests use this.
    pub async fn spawn() -> Self {
        Self::spawn_rs256(true).await
    }

    /// RSA-2048 with the JWK's `alg` field OMITTED. Covers RFC 7517
    /// §4.4 — `build_cached_key` defaults RSA → RS256 when alg is
    /// absent, EC needs explicit alg.
    pub async fn spawn_without_alg() -> Self {
        Self::spawn_rs256(false).await
    }

    /// P-256 / ES256. Exercises the elliptic-curve branch of
    /// `build_cached_key`. Per-test keygen is ~1 ms (vs ~50 ms for
    /// the RSA path), so an ES256 mirror across the suite is cheap.
    pub async fn spawn_es256() -> Self {
        let secret = P256Secret::random(&mut rand::thread_rng());
        let public = secret.public_key();

        let encoding_key = {
            // jsonwebtoken's `from_ec_pem` accepts PKCS#8 EC keys; the
            // p256 crate's PKCS#8 serializer is the path of least
            // friction (vs SEC1, which requires a feature flag).
            let pem = secret
                .to_pkcs8_pem(Default::default())
                .expect("pkcs8 pem encode");
            EncodingKey::from_ec_pem(pem.as_bytes()).expect("encoding key from EC pem")
        };

        // Encode the public key as the SEC1 uncompressed form (0x04 ||
        // X || Y), then split X / Y into base64url for the JWK.
        let encoded_point = public.to_encoded_point(false);
        let x = encoded_point.x().expect("EC public key missing x").to_vec();
        let y = encoded_point.y().expect("EC public key missing y").to_vec();

        let jwks = JwkSet {
            keys: vec![Jwk {
                common: CommonParameters {
                    public_key_use: Some(PublicKeyUse::Signature),
                    key_algorithm: Some(KeyAlgorithm::ES256),
                    key_id: Some(TEST_KID.to_string()),
                    ..Default::default()
                },
                algorithm: AlgorithmParameters::EllipticCurve(EllipticCurveKeyParameters {
                    key_type: EllipticCurveKeyType::EC,
                    curve: EllipticCurve::P256,
                    x: base64_url(&x),
                    y: base64_url(&y),
                }),
            }],
        };

        Self::serve(jwks, encoding_key, HarnessAlg::Es256).await
    }

    pub async fn spawn_rs256(advertise_alg: bool) -> Self {
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen failed");
        let public = RsaPublicKey::from(&private);

        let encoding_key = {
            let pem = private
                .to_pkcs1_pem(Default::default())
                .expect("pem encode");
            EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key from pem")
        };

        let jwks = build_rsa_jwks(&public, advertise_alg);
        Self::serve(jwks, encoding_key, HarnessAlg::Rs256).await
    }

    /// Shared listener + axum sub-app setup, factored out so the
    /// per-alg constructors only have to compose the JWK Set.
    async fn serve(jwks: JwkSet, encoding_key: EncodingKey, alg: HarnessAlg) -> Self {
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
            alg,
            jwks_request_count,
        }
    }

    pub fn jwks_request_count(&self) -> usize {
        self.jwks_request_count.load(Ordering::SeqCst)
    }

    /// Header with this harness's algorithm + the supplied `kid`. The
    /// default for most tests is `header_with_kid(TEST_KID)`.
    pub fn header_with_kid(&self, kid: &str) -> Header {
        let mut h = Header::new(self.alg.algorithm());
        h.kid = Some(kid.to_string());
        h
    }

    /// Build a `JwtVerifier` pointing at this harness's mock JWKS.
    /// Owned (not `Arc`-wrapped) so consumers can choose between
    /// the direct verify path and the `Arc`-wrapped state injection.
    pub fn verifier(&self) -> JwtVerifier {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
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

    /// Same as [`Self::verifier`] but wrapped for `AppState` injection.
    pub fn verifier_arc(&self) -> Arc<JwtVerifier> {
        Arc::new(self.verifier())
    }

    pub fn mint(&self, claims: &impl Serialize, header: &Header) -> String {
        // ES256 in jsonwebtoken 10 derives signatures deterministically
        // from the supplied secret key — the same `(claims, key)` pair
        // produces the same JWT across calls. RSA's PKCS#1 v1.5 sig is
        // already deterministic. So no extra rng plumbing is needed here.
        encode(header, claims, &self.encoding_key).expect("token encode failed")
    }
}

pub fn build_rsa_jwks(public: &RsaPublicKey, advertise_alg: bool) -> JwkSet {
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

/// Standalone helper — defaults to RS256. The per-harness
/// `[`JwksHarness::header_with_kid`]` is preferred when the test
/// might switch algorithms, but RSA-only tests can keep using this.
pub fn header_with_kid(kid: &str) -> Header {
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some(kid.to_string());
    h
}

/// Default claims for the happy-path test. `exp` is 5 min in the
/// future, `nbf` is `iat`. All times are unix epoch seconds (JWT
/// spec — distinct from the rest of the codebase's millisecond
/// timestamps).
pub fn good_claims(sub: &str) -> serde_json::Value {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    json!({
        "sub": sub,
        "iss": TEST_ISSUER,
        "aud": TEST_AUDIENCE,
        "iat": now,
        "nbf": now,
        "exp": now + 300,
    })
}
