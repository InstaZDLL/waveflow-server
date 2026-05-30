//! Shared JWKS test harness — spawns a mock JWKS endpoint backed by
//! an RSA-2048 keypair generated at construction time, and exposes
//! both a `JwtVerifier` pointed at it and a `mint(claims)` helper
//! that signs tokens with the matching private key.
//!
//! Cargo's integration tests each compile as their own crate, so
//! this file is included via `#[path]` from the consumers
//! (`tests/auth.rs`, `tests/jwt_middleware.rs`) rather than mounted
//! as a sub-module. The `#[allow(dead_code)]` on the items here
//! covers symbols a given consumer doesn't end up calling.

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
        AlgorithmParameters, CommonParameters, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse,
        RSAKeyParameters, RSAKeyType,
    },
    Algorithm, EncodingKey, Header,
};
use rsa::{pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts, RsaPrivateKey, RsaPublicKey};
use serde::Serialize;
use serde_json::json;
use waveflow_server::auth::{JwtVerifier, JwtVerifierConfig};

pub const TEST_ISSUER: &str = "https://auth.test.example.com";
pub const TEST_AUDIENCE: &str = "waveflow-server-test";
pub const TEST_KID: &str = "test-key-1";

pub struct JwksHarness {
    pub jwks_url: String,
    pub encoding_key: EncodingKey,
    pub jwks_request_count: Arc<AtomicUsize>,
}

impl JwksHarness {
    pub async fn spawn() -> Self {
        Self::spawn_with(true).await
    }

    pub async fn spawn_without_alg() -> Self {
        Self::spawn_with(false).await
    }

    pub async fn spawn_with(advertise_alg: bool) -> Self {
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

    pub fn jwks_request_count(&self) -> usize {
        self.jwks_request_count.load(Ordering::SeqCst)
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
        encode(header, claims, &self.encoding_key).expect("token encode failed")
    }
}

pub fn build_jwks(public: &RsaPublicKey, advertise_alg: bool) -> JwkSet {
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
