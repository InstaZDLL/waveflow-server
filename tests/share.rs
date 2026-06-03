//! End-to-end tests for `/api/v1/share/*` and the per-playlist
//! mint / revoke endpoints. Phase 1.g.1 of the WaveFlow roadmap.
//!
//! Coverage matrix mirrors `tests/playlists.rs` for the tenant-
//! isolation battery (foreign profile 404, proxy attack, 401 gate)
//! plus share-specific properties:
//!
//! - Mint is idempotent — a second call returns the same token.
//! - Revoke + re-mint produces a NEW token (the partial UNIQUE
//!   index allows reuse of the previous value).
//! - Public GET returns 404 on unknown / revoked tokens, with no
//!   way for an attacker to distinguish the two.
//! - The public payload omits `profile_id` and other tenant-
//!   identifying fields.

mod support;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::{spawn_authenticated, spawn_two_authenticated};

async fn mint_profile(base: &str, token: &str, name: &str) -> i64 {
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .bearer_auth(token)
        .json(&json!({ "name": name, "color_id": "emerald" }))
        .send()
        .await
        .expect("profile create failed")
        .error_for_status()
        .expect("non-2xx on profile create")
        .json()
        .await
        .expect("profile create body");
    created["id"].as_i64().expect("profile id missing")
}

async fn mint_playlist(base: &str, token: &str, profile_id: i64, name: &str) -> i64 {
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .bearer_auth(token)
        .json(&json!({ "name": name }))
        .send()
        .await
        .expect("playlist create failed")
        .error_for_status()
        .expect("non-2xx on playlist create")
        .json()
        .await
        .expect("playlist create body");
    created["id"].as_i64().expect("playlist id missing")
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn mint_returns_token_and_public_get_resolves(pool: PgPool) {
    let h = spawn_authenticated(pool, "user-share").await;
    let pid = mint_profile(&h.base, &h.token, "p").await;
    let plid = mint_playlist(&h.base, &h.token, pid, "Soirée").await;

    let resp: Value = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/playlists/{}/share",
            h.base, pid, plid
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = resp["token"].as_str().expect("mint token").to_string();
    assert_eq!(token.len(), 32, "token must be 32 chars (Phase 1.g spec)");

    // Public GET — no JWT, just the token.
    let public: Value = reqwest::Client::new()
        .get(format!("{}/api/v1/share/playlists/{}", h.base, token))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(public["id"].as_i64(), Some(plid));
    assert_eq!(public["name"].as_str(), Some("Soirée"));
    assert!(public["tracks"].is_array());
    // Tenant-identifying fields must NOT be on the public payload.
    assert!(
        public.get("profile_id").is_none(),
        "public payload must not leak profile_id, got {public}"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn mint_is_idempotent(pool: PgPool) {
    let h = spawn_authenticated(pool, "user-share").await;
    let pid = mint_profile(&h.base, &h.token, "p").await;
    let plid = mint_playlist(&h.base, &h.token, pid, "p").await;

    let first: Value = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/playlists/{}/share",
            h.base, pid, plid
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let second: Value = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/playlists/{}/share",
            h.base, pid, plid
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        first["token"].as_str(),
        second["token"].as_str(),
        "two mints in a row must return the same token"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn revoke_closes_the_public_url(pool: PgPool) {
    let h = spawn_authenticated(pool, "user-share").await;
    let pid = mint_profile(&h.base, &h.token, "p").await;
    let plid = mint_playlist(&h.base, &h.token, pid, "p").await;

    let token = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/playlists/{}/share",
            h.base, pid, plid
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    let revoke = reqwest::Client::new()
        .delete(format!(
            "{}/api/v1/profiles/{}/playlists/{}/share",
            h.base, pid, plid
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

    let public = reqwest::Client::new()
        .get(format!("{}/api/v1/share/playlists/{}", h.base, token))
        .send()
        .await
        .unwrap();
    assert_eq!(
        public.status(),
        StatusCode::NOT_FOUND,
        "revoked token must surface as 404 with no body, same shape as never-minted"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn revoke_then_remint_returns_a_fresh_token(pool: PgPool) {
    let h = spawn_authenticated(pool, "user-share").await;
    let pid = mint_profile(&h.base, &h.token, "p").await;
    let plid = mint_playlist(&h.base, &h.token, pid, "p").await;

    let initial = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/playlists/{}/share",
            h.base, pid, plid
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    reqwest::Client::new()
        .delete(format!(
            "{}/api/v1/profiles/{}/playlists/{}/share",
            h.base, pid, plid
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    let reminted = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/playlists/{}/share",
            h.base, pid, plid
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(
        initial, reminted,
        "revoke + re-mint MUST produce a fresh token — pinning a stale URL would defeat revoke"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn mint_against_foreign_playlist_is_404(pool: PgPool) {
    let two = spawn_two_authenticated(pool, "alice", "bob").await;
    let pid_a = mint_profile(&two.base, &two.a.token, "alice").await;
    let plid_a = mint_playlist(&two.base, &two.a.token, pid_a, "alice's playlist").await;

    let status = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/playlists/{}/share",
            two.base, pid_a, plid_a
        ))
        .bearer_auth(&two.b.token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "tenant proxy attack must 404, never leak existence"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn public_get_unknown_token_is_404(pool: PgPool) {
    let h = spawn_authenticated(pool, "user-share").await;
    let resp = reqwest::Client::new()
        .get(format!(
            "{}/api/v1/share/playlists/{}",
            h.base,
            "x".repeat(32)
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn mint_requires_auth(pool: PgPool) {
    let h = spawn_authenticated(pool, "user-share").await;
    let pid = mint_profile(&h.base, &h.token, "p").await;
    let plid = mint_playlist(&h.base, &h.token, pid, "p").await;
    let status = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/playlists/{}/share",
            h.base, pid, plid
        ))
        // No bearer.
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
