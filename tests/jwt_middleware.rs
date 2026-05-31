//! End-to-end tests for the JWT-bearer auth middleware
//! ([`waveflow_server::middleware::authenticate`]).
//!
//! Strategy: each test spins up a fresh mock JWKS server, a verifier
//! pointed at it, and a waveflow-server app wired with that verifier.
//! Tokens are minted with the matching private key, and the assertions
//! exercise the middleware's accept / reject branches by hitting the
//! tenant-scoped `/api/v1/profiles` endpoint.
//!
//! Why profiles: it's the smallest CRUD surface that requires
//! authentication. A 401 vs 200 there is unambiguous proof of the
//! middleware's decision.

mod jwks_harness;
mod support;

use jsonwebtoken::Header;
use jwks_harness::{good_claims, header_with_kid, JwksHarness, TEST_KID};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::{spawn_app_with_jwt, spawn_app_with_jwt_and_shim};

/// Bootstrap helper — mint a user with the supplied `external_id`,
/// return its id. JWT-only test mode can't hit `POST /api/v1/users`
/// (it's gated by the dev shim), so the shim-and-JWT mode is the
/// transition shape these tests need.
async fn mint_user_with_external_id(base: &str, external_id: &str) -> i64 {
    let body: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/users"))
        .json(&json!({ "external_id": external_id }))
        .send()
        .await
        .expect("user create failed")
        .error_for_status()
        .expect("non-2xx on user create")
        .json()
        .await
        .expect("user create body");
    body["id"].as_i64().expect("user id missing from response")
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn valid_bearer_authenticates_request(pool: PgPool) {
    let harness = JwksHarness::spawn().await;
    let base = spawn_app_with_jwt_and_shim(pool, harness.verifier_arc()).await;

    let external_id = "auth-user-jwt-happy";
    let user_id = mint_user_with_external_id(&base, external_id).await;

    let token = harness.mint(&good_claims(external_id), &header_with_kid(TEST_KID));

    // Hit the protected endpoint with Bearer — should authenticate
    // and let the request through to the empty-list happy path.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.is_empty());

    // And the round-trip — create a profile via Bearer, then GET via
    // Bearer — proves the UserId extension threads through to the
    // tenant-scoped query.
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .json(&json!({ "name": "via-JWT", "color_id": "emerald" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let profile_id = created["id"].as_i64().unwrap();

    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"].as_i64().unwrap(), profile_id);

    // And the same user_id underlies both auth paths — the X-User-Id
    // shim and the Bearer JWT both surface the same row.
    let list_via_shim: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list_via_shim.len(), 1);
    assert_eq!(list_via_shim[0]["id"].as_i64().unwrap(), profile_id);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn missing_bearer_with_jwt_only_returns_401(pool: PgPool) {
    let harness = JwksHarness::spawn().await;
    // JWT-only mode (no shim). We can't mint a user from the
    // endpoint, so this test only exercises the rejection path.
    let base = spawn_app_with_jwt(pool, harness.verifier_arc()).await;

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn bearer_with_unknown_sub_lazy_provisions_user(pool: PgPool) {
    let harness = JwksHarness::spawn().await;
    let base = spawn_app_with_jwt_and_shim(pool.clone(), harness.verifier_arc()).await;

    // No `mint_user_with_external_id` call — the sub in the token
    // has no matching row in `users` at request time. Phase 1.c.3a
    // says: a valid JWT IS the authoritative onboarding signal, so
    // the middleware inserts the row and lets the request through.
    let external_id = "fresh-better-auth-sub";
    let token = harness.mint(&good_claims(external_id), &header_with_kid(TEST_KID));

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    // First request gets 200 with an empty profile list — proves the
    // request was authenticated AND scoped to the new user (an
    // unscoped query would have returned someone else's profiles).
    assert_eq!(resp.status(), StatusCode::OK);
    let profiles: Value = resp.json().await.unwrap();
    assert_eq!(profiles, json!([]));

    // The row landed in `users` with the verified sub.
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE external_id = $1")
            .bind(external_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row_count, 1);

    // Second request reuses the same row — idempotent UPSERT, no
    // duplicate insert attempt.
    let resp2 = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let row_count_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE external_id = $1")
            .bind(external_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row_count_after, 1);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn bearer_with_bad_signature_returns_401(pool: PgPool) {
    // Two independent harnesses → two independent keypairs. Mint
    // the token with harness A, hand the verifier from harness B
    // to the server. The signature won't validate against the
    // server's JWKS — exactly the wrong-key scenario.
    let signing_harness = JwksHarness::spawn().await;
    let server_harness = JwksHarness::spawn().await;
    let base = spawn_app_with_jwt_and_shim(pool, server_harness.verifier_arc()).await;

    // Mint a user via the shim so the sub *could* resolve — proving
    // the 401 isn't just a "no user" miss but a signature failure.
    mint_user_with_external_id(&base, "auth-user-bad-sig").await;

    let token = signing_harness.mint(
        &good_claims("auth-user-bad-sig"),
        &header_with_kid(TEST_KID),
    );

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn bearer_with_no_kid_returns_401(pool: PgPool) {
    let harness = JwksHarness::spawn().await;
    let base = spawn_app_with_jwt_and_shim(pool, harness.verifier_arc()).await;
    mint_user_with_external_id(&base, "auth-user-no-kid").await;

    let header = Header::new(jsonwebtoken::Algorithm::RS256);
    // header.kid stays None — verifier rejects with MalformedToken
    // → 401.
    let token = harness.mint(&good_claims("auth-user-no-kid"), &header);

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn bearer_with_wrong_scheme_returns_401(pool: PgPool) {
    let harness = JwksHarness::spawn().await;
    let base = spawn_app_with_jwt_and_shim(pool, harness.verifier_arc()).await;
    let token = harness.mint(&good_claims("anyone"), &header_with_kid(TEST_KID));

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        // Wrong scheme; verifier's strip_bearer_prefix rejects.
        .header(reqwest::header::AUTHORIZATION, format!("Basic {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// With JWT configured AND the shim enabled, the JWT path takes
/// precedence when both headers are present. A request that carries
/// `Authorization: Bearer <invalid>` AND `X-User-Id: 1` MUST 401 —
/// the forgeable header can't override a failed JWT check, otherwise
/// an attacker could downgrade auth by sending a bogus Bearer
/// alongside a forged user id.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn invalid_bearer_does_not_downgrade_to_shim(pool: PgPool) {
    let harness = JwksHarness::spawn().await;
    let base = spawn_app_with_jwt_and_shim(pool, harness.verifier_arc()).await;
    let user_id = mint_user_with_external_id(&base, "real-user").await;

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .header(reqwest::header::AUTHORIZATION, "Bearer obviously.not.a.jwt")
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "an invalid Bearer must not silently fall back to the shim — \
         that would let an attacker downgrade auth"
    );
}

/// With no auth configured at all, every `/api/v1/*` route 503s —
/// same prod-gate behaviour as the legacy `reject_dev_auth_disabled`
/// branch from pre-PR3 mod.rs, now generalised to "neither path
/// configured".
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn no_auth_configured_returns_503(pool: PgPool) {
    let base = support::spawn_app_prod_gate(pool).await;

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        // Header is irrelevant — the gate is at the auth layer
        // before any parsing.
        .header("x-user-id", "42")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
