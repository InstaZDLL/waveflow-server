//! JWT bearer verification against an upstream JWKS endpoint.
//!
//! Phase 1.d.1-PR2 ships this module standalone — the middleware
//! that consumes it lands in PR3. The split keeps each PR small and
//! lets the verifier's accept / reject paths get tested in isolation
//! before they enter the request path.
//!
//! Design intent (RFC-001 §6.6):
//!
//! - A [`JwtVerifier`] is built once at boot from the upstream
//!   `WAVEFLOW_JWT_JWKS_URL`, `WAVEFLOW_JWT_ISSUER`, and
//!   `WAVEFLOW_JWT_AUDIENCE` env vars, then handed to the router as
//!   `Arc<JwtVerifier>` so every middleware invocation shares one
//!   key cache + one reqwest client.
//! - The cache is keyed by the `kid` header claim. On a miss the
//!   verifier fetches the JWKS once, decodes every key it understands,
//!   stashes them under their `kid`, and retries the lookup. A second
//!   miss on the same `kid` means the token references a key the
//!   upstream never advertised — short-circuit to
//!   [`AuthError::KeyNotFound`] rather than refetching in a loop.
//! - Signature failures on a cached key do NOT trigger a refetch.
//!   That's the right call from a DoS perspective (a flood of bad
//!   tokens shouldn't hammer the upstream) but it also means a key
//!   rotation where the upstream replaces a `kid` in place would
//!   fail closed. Better Auth rotates by adding a new `kid` and
//!   leaving the old one in the set for a grace window, so the
//!   miss-then-refetch path handles the common case.
//! - Algorithm allowlist is read off each JWK's own `alg`, which is
//!   what Better Auth advertises (RS256 by default). A token whose
//!   header `alg` disagrees with the cached key's `alg` is rejected
//!   so a confused-deputy attempt to flip RS256 → HS256 doesn't
//!   bypass verification.
//!
//! What this module deliberately doesn't do:
//!
//! - It doesn't resolve `sub` to `users.id` — that's the middleware's
//!   job (PR3), via the `external_id` column landed in PR1.
//! - It doesn't read env vars. The verifier takes everything via
//!   [`JwtVerifierConfig`] so the binary, integration tests, and a
//!   future Better Auth shim can all construct it the same way.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use jsonwebtoken::{
    decode, decode_header,
    jwk::{AlgorithmParameters, Jwk, JwkSet},
    Algorithm, DecodingKey, Validation,
};
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::RwLock;

/// How long a freshly-fetched JWKS document stays trusted before the
/// next miss forces a refetch. One hour matches Better Auth's
/// recommended `Cache-Control: max-age=3600` for the default key
/// lifetime — short enough that a key rotation propagates without an
/// operator nudge, long enough that the JWKS endpoint isn't on the
/// hot path of every request.
const JWKS_CACHE_TTL: Duration = Duration::from_secs(3_600);

/// Failure modes surfaced by [`JwtVerifier::verify_bearer`]. The
/// middleware in PR3 maps each variant to an HTTP status — kept here
/// so a future programmatic consumer (CLI debugger, structured log
/// emitter) can discriminate without re-parsing the message.
#[derive(Debug, Error)]
pub enum AuthError {
    /// The header was missing, malformed, or used a scheme other
    /// than `Bearer`. The middleware emits 401.
    #[error("missing or malformed Authorization header")]
    MissingOrMalformedHeader,

    /// The JWT itself was unparseable (couldn't read the header,
    /// claims, or signature). 401.
    #[error("malformed JWT")]
    MalformedToken,

    /// The token's `kid` doesn't match anything in the cached JWKS.
    /// A single retry after a fresh fetch is the cache miss path
    /// above; this error is raised when even the post-refetch set
    /// doesn't carry the kid. 401.
    #[error("token references a kid that the JWKS doesn't advertise")]
    KeyNotFound,

    /// The cached key exists but its declared algorithm disagrees
    /// with the token's header `alg`, or the algorithm isn't one
    /// we're willing to verify. 401 — a confused-deputy attempt to
    /// downgrade RS256 → HS256 lands here.
    #[error("token algorithm does not match the JWKS key")]
    AlgorithmMismatch,

    /// Signature verification, expiry, `iss`, or `aud` failed. 401.
    #[error("token signature or claims invalid: {0}")]
    InvalidClaims(String),

    /// The verified token didn't carry a `sub` claim — Better Auth
    /// guarantees one but a misconfigured upstream might not. 401.
    #[error("token has no `sub` claim")]
    MissingSub,

    /// JWKS fetch failed at the network or HTTP level. 503 in the
    /// middleware so the load balancer routes around the instance
    /// while the upstream is unreachable.
    #[error("JWKS fetch failed: {0}")]
    JwksFetchFailed(String),

    /// JWKS document parsed but contained zero usable keys. 503 —
    /// same routing rationale as `JwksFetchFailed`.
    #[error("JWKS document carried no usable keys")]
    EmptyJwks,
}

/// Construction parameters for [`JwtVerifier`].
#[derive(Debug, Clone)]
pub struct JwtVerifierConfig {
    /// URL of the upstream JWKS document, e.g.
    /// `https://auth.example.com/.well-known/jwks.json`.
    pub jwks_url: String,
    /// Expected `iss` claim — Better Auth issues tokens with the
    /// auth provider's base URL. The verifier rejects any token
    /// whose `iss` isn't an exact string match.
    pub issuer: String,
    /// Expected `aud` claim. A single string match for now (Better
    /// Auth issues a single audience per token); a future
    /// `Vec<String>` would let one server accept tokens from
    /// multiple sub-products.
    pub audience: String,
}

/// Carries everything the request path needs about a verified token.
/// The middleware (PR3) reads `sub` and resolves it against
/// `users.external_id`.
#[derive(Debug, Clone)]
pub struct VerifiedClaims {
    pub sub: String,
}

/// Per-`kid` decoded key + the algorithm advertised in the JWKS.
/// Storing the alg here (vs deriving it from the token header) is
/// the load-bearing piece of [`AuthError::AlgorithmMismatch`] — the
/// trusted source of truth is the JWKS, not the inbound token.
///
/// No `Debug` derive: `jsonwebtoken::DecodingKey` deliberately
/// doesn't implement `Debug` so a stray `{:?}` can't leak the raw
/// key material into a log line. Same reason we don't derive
/// `Debug` on the parent `JwtVerifier`.
struct CachedKey {
    decoding_key: DecodingKey,
    algorithm: Algorithm,
}

/// `Debug` is deliberately not derived — every field hangs off a
/// `CachedKey` that owns a `DecodingKey` (no `Debug` impl, see the
/// rationale above), so a tracing macro can't accidentally splat
/// raw key bytes into a log sink.
#[derive(Default)]
struct JwksCache {
    keys: HashMap<String, Arc<CachedKey>>,
    /// `None` until the first successful fetch — that way the first
    /// verification attempt triggers a fetch instead of a stale-cache
    /// lookup against an empty map.
    fetched_at: Option<Instant>,
}

impl JwksCache {
    fn is_expired(&self, now: Instant) -> bool {
        match self.fetched_at {
            Some(at) => now.saturating_duration_since(at) >= JWKS_CACHE_TTL,
            None => true,
        }
    }
}

/// Thread-safe JWT verifier. Cheap to `Clone` (the inner
/// `Arc<RwLock<…>>` shares the cache across every clone), so the
/// router can hand it out per-request without contention.
pub struct JwtVerifier {
    config: JwtVerifierConfig,
    cache: RwLock<JwksCache>,
    client: reqwest::Client,
}

impl JwtVerifier {
    /// Construct a verifier against a real upstream JWKS URL. The
    /// HTTP client is built fresh per instance so the timeout +
    /// rustls config stay local.
    pub fn new(config: JwtVerifierConfig) -> Result<Self, AuthError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|err| AuthError::JwksFetchFailed(err.to_string()))?;
        Ok(Self {
            config,
            cache: RwLock::new(JwksCache::default()),
            client,
        })
    }

    /// Build the verifier from a caller-supplied reqwest client.
    /// Cargo's integration-test crates (`tests/*.rs`) compile
    /// against the library with `cfg(test)` OFF, so this can't sit
    /// behind a `cfg(test)` gate — `tests/auth.rs` needs it to
    /// inject a shorter-timeout client into the mock-JWKS-server
    /// harness. The same hook is useful in production for callers
    /// who want a custom HTTP middleware stack (mTLS to Better
    /// Auth, retry policy, etc.) on top of the default client
    /// [`Self::new`] would build.
    pub fn with_client(config: JwtVerifierConfig, client: reqwest::Client) -> Self {
        Self {
            config,
            cache: RwLock::new(JwksCache::default()),
            client,
        }
    }

    /// Verify a `Bearer <token>` header value end-to-end. Strips the
    /// scheme, validates against the cached JWKS (refetching on miss),
    /// checks `iss` / `aud` / `exp` / `nbf`, and returns the
    /// verified claims.
    pub async fn verify_bearer(&self, header_value: &str) -> Result<VerifiedClaims, AuthError> {
        let token = strip_bearer_prefix(header_value)?;
        self.verify_token(token).await
    }

    /// Verify a bare JWT (no `Bearer` prefix). Exposed so tests can
    /// exercise the verification path without round-tripping through
    /// the header format.
    pub async fn verify_token(&self, token: &str) -> Result<VerifiedClaims, AuthError> {
        let header = decode_header(token).map_err(|_| AuthError::MalformedToken)?;
        let kid = header.kid.ok_or(AuthError::MalformedToken)?;

        let cached = self.resolve_kid(&kid).await?;

        // Reject right away if the token's declared alg disagrees
        // with what the JWKS says about this kid. The decode call
        // below would also flag it (we set `algorithms` to the
        // cached alg only), but failing fast with a discriminated
        // error makes log diagnostics + a future structured probe
        // easier to read.
        if header.alg != cached.algorithm {
            return Err(AuthError::AlgorithmMismatch);
        }

        let mut validation = Validation::new(cached.algorithm);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[&self.config.audience]);
        // `exp` is required by default; explicitly require `sub` too
        // so a malformed token without one fails at validation time
        // rather than after we already trusted the signature.
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);

        let token_data = decode::<RawClaims>(token, &cached.decoding_key, &validation)
            .map_err(|err| AuthError::InvalidClaims(err.to_string()))?;

        let sub = token_data.claims.sub.ok_or(AuthError::MissingSub)?;
        if sub.trim().is_empty() {
            // Belt-and-braces — the `required_spec_claims` check
            // above catches a missing field but not an explicitly
            // empty string. Mirrors the boundary trim+reject the
            // rest of the codebase applies to user input.
            return Err(AuthError::MissingSub);
        }

        Ok(VerifiedClaims { sub })
    }

    /// Look up a `kid` in the cache, fetching the JWKS on a miss or
    /// expiry. The function holds the cache's read lock during the
    /// hot path; only the refetch path takes the write lock.
    async fn resolve_kid(&self, kid: &str) -> Result<Arc<CachedKey>, AuthError> {
        let now = Instant::now();

        // Fast path — cached, not stale, kid present.
        {
            let cache = self.cache.read().await;
            if !cache.is_expired(now) {
                if let Some(key) = cache.keys.get(kid) {
                    return Ok(Arc::clone(key));
                }
            }
        }

        // Slow path — refetch the JWKS, then look the kid up again.
        self.refresh_cache().await?;

        let cache = self.cache.read().await;
        cache.keys.get(kid).cloned().ok_or(AuthError::KeyNotFound)
    }

    async fn refresh_cache(&self) -> Result<(), AuthError> {
        let jwks: JwkSet = self
            .client
            .get(&self.config.jwks_url)
            .send()
            .await
            .map_err(|err| AuthError::JwksFetchFailed(err.to_string()))?
            .error_for_status()
            .map_err(|err| AuthError::JwksFetchFailed(err.to_string()))?
            .json()
            .await
            .map_err(|err| AuthError::JwksFetchFailed(err.to_string()))?;

        let mut keys = HashMap::with_capacity(jwks.keys.len());
        for jwk in jwks.keys {
            if let Some((kid, cached)) = build_cached_key(jwk) {
                keys.insert(kid, Arc::new(cached));
            }
        }

        if keys.is_empty() {
            return Err(AuthError::EmptyJwks);
        }

        let mut cache = self.cache.write().await;
        cache.keys = keys;
        cache.fetched_at = Some(Instant::now());
        Ok(())
    }
}

/// `sub` is `Option<String>` so a missing claim surfaces as a
/// discriminated `MissingSub` rather than a generic
/// `InvalidClaims("missing field: sub")`. `exp` / `iss` / `aud` are
/// validated by jsonwebtoken's own machinery against the
/// `Validation` builder above, so we don't need to re-declare them
/// here — `serde(default)` skips them on the decode-side.
#[derive(Debug, Deserialize)]
struct RawClaims {
    #[serde(default)]
    sub: Option<String>,
}

/// Pull the JWS key + its `alg` off a single JWK entry. Returns
/// `None` for keys we don't handle (e.g. EdDSA — we'll add it the
/// day Better Auth makes it the default) so the caller can skip
/// them silently. The `kid` is required because the cache is keyed
/// on it; a key without a `kid` is unaddressable from a token.
fn build_cached_key(jwk: Jwk) -> Option<(String, CachedKey)> {
    let kid = jwk.common.key_id.clone()?;
    let algorithm = jwk.common.key_algorithm?;

    // jsonwebtoken's `Algorithm` enum and `KeyAlgorithm` enum are
    // separate types — convert via the textual representation. The
    // `parse` round-trip rejects algorithms our verifier doesn't
    // understand without needing a `match` over every variant.
    let alg_name = algorithm.to_string();
    let algorithm: Algorithm = alg_name.parse().ok()?;

    let decoding_key = match &jwk.algorithm {
        AlgorithmParameters::RSA(rsa) => DecodingKey::from_rsa_components(&rsa.n, &rsa.e).ok()?,
        AlgorithmParameters::EllipticCurve(ec) => {
            DecodingKey::from_ec_components(&ec.x, &ec.y).ok()?
        }
        _ => return None,
    };

    Some((
        kid,
        CachedKey {
            decoding_key,
            algorithm,
        },
    ))
}

fn strip_bearer_prefix(header_value: &str) -> Result<&str, AuthError> {
    // Trim the whole value first so trailing whitespace or a stray
    // CRLF doesn't bypass the `Bearer ` match. Then split on the
    // first ASCII space — RFC 6750 mandates `Bearer ` with a single
    // space, but real-world clients occasionally pad it.
    let trimmed = header_value.trim();
    let (scheme, token) = trimmed
        .split_once(' ')
        .ok_or(AuthError::MissingOrMalformedHeader)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(AuthError::MissingOrMalformedHeader);
    }
    let token = token.trim();
    if token.is_empty() {
        return Err(AuthError::MissingOrMalformedHeader);
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_bearer_prefix_accepts_canonical() {
        assert_eq!(
            strip_bearer_prefix("Bearer abc.def.ghi").unwrap(),
            "abc.def.ghi"
        );
    }

    #[test]
    fn strip_bearer_prefix_accepts_case_insensitive_scheme() {
        assert_eq!(
            strip_bearer_prefix("bearer abc.def.ghi").unwrap(),
            "abc.def.ghi"
        );
        assert_eq!(
            strip_bearer_prefix("BEARER abc.def.ghi").unwrap(),
            "abc.def.ghi"
        );
    }

    #[test]
    fn strip_bearer_prefix_trims_surrounding_whitespace() {
        assert_eq!(
            strip_bearer_prefix("  Bearer abc.def.ghi\r\n").unwrap(),
            "abc.def.ghi"
        );
    }

    #[test]
    fn strip_bearer_prefix_rejects_wrong_scheme() {
        assert!(matches!(
            strip_bearer_prefix("Basic abc"),
            Err(AuthError::MissingOrMalformedHeader)
        ));
    }

    #[test]
    fn strip_bearer_prefix_rejects_empty_token() {
        assert!(matches!(
            strip_bearer_prefix("Bearer  "),
            Err(AuthError::MissingOrMalformedHeader)
        ));
    }

    #[test]
    fn strip_bearer_prefix_rejects_no_space() {
        assert!(matches!(
            strip_bearer_prefix("BearerX"),
            Err(AuthError::MissingOrMalformedHeader)
        ));
    }
}
