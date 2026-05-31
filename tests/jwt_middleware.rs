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
//!
//! Phase 1.d.2 collapsed the auth surface to JWT-only; the `*_with_shim`
//! variants of these tests retired alongside the shim itself.

mod support;

use jsonwebtoken::Header;
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::{
    good_claims, header_with_kid, spawn_app_with_jwt, spawn_authenticated, JwksHarness, TEST_KID,
};

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn valid_bearer_authenticates_and_provisions_user(pool: PgPool) {
    let auth = spawn_authenticated(pool, "auth-user-jwt-happy").await;

    // List → empty (the user owns no profiles yet).
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{}/api/v1/profiles", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.is_empty());

    // Create + re-list — proves the UserId extension threads through
    // to the tenant-scoped storage call.
    let created: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/profiles", auth.base))
        .bearer_auth(&auth.token)
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
        .get(format!("{}/api/v1/profiles", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"].as_i64().unwrap(), profile_id);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn missing_bearer_returns_401(pool: PgPool) {
    let harness = JwksHarness::spawn().await;
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
    let base = spawn_app_with_jwt(pool.clone(), harness.verifier_arc()).await;

    // Fire two requests concurrently with the same fresh token so
    // both racing tasks hit the SELECT-miss → UPSERT path together.
    // Idempotence is the property we're proving: both must succeed
    // and exactly one row must land — proves the UPSERT fallback
    // catches the race we'd otherwise hit between the SELECT and
    // the INSERT.
    let external_id = "fresh-better-auth-sub";
    let token = harness.mint(&good_claims(external_id), &header_with_kid(TEST_KID));

    let client = reqwest::Client::new();
    let req_a = client
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send();
    let req_b = client
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send();
    let (resp_a, resp_b) = tokio::join!(req_a, req_b);
    let resp_a = resp_a.unwrap();
    let resp_b = resp_b.unwrap();

    // Both requests authenticated AND tenant-scoped to the new user.
    assert_eq!(resp_a.status(), StatusCode::OK);
    assert_eq!(resp_b.status(), StatusCode::OK);
    let profiles_a: Value = resp_a.json().await.unwrap();
    let profiles_b: Value = resp_b.json().await.unwrap();
    assert_eq!(profiles_a, json!([]));
    assert_eq!(profiles_b, json!([]));

    // Exactly one row landed despite two parallel UPSERTs.
    let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE external_id = $1")
        .bind(external_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row_count, 1);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn bearer_with_bad_signature_returns_401(pool: PgPool) {
    // Two independent harnesses → two independent keypairs. Mint
    // the token with harness A, hand the verifier from harness B
    // to the server. The signature won't validate against the
    // server's JWKS — exactly the wrong-key scenario.
    let signing_harness = JwksHarness::spawn().await;
    let server_harness = JwksHarness::spawn().await;
    let base = spawn_app_with_jwt(pool, server_harness.verifier_arc()).await;

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
    let base = spawn_app_with_jwt(pool, harness.verifier_arc()).await;

    // header.kid stays None — verifier rejects with MalformedToken
    // → 401.
    let header = Header::new(jsonwebtoken::Algorithm::RS256);
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
    let base = spawn_app_with_jwt(pool, harness.verifier_arc()).await;
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
