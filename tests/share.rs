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
use uuid::Uuid;

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

/// Push a playlist `insert` sync_op carrying both canonical ids and
/// wait for the apply pipeline to materialise the row in the same
/// transaction. Mirrors what the desktop drain task will do once
/// Phase 1.g.0-desktop ships.
///
/// Each call uses a fresh UUID for `device_id` so repeated
/// invocations within the same test never hit the
/// `(user_id, device_id, lamport_ts)` UNIQUE — the constraint is
/// scoped per device, so two "different devices" can both use
/// lamport_ts = 1 without colliding. Simpler than threading a
/// counter through callers, and accurate to real desktop life
/// where every test scenario plays the role of a fresh device.
async fn materialise_playlist_via_sync(
    base: &str,
    token: &str,
    profile_canonical: &str,
    playlist_canonical: &str,
    name: &str,
) {
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/sync/ops"))
        .bearer_auth(token)
        .json(&json!({
            "device_id": Uuid::new_v4().to_string(),
            "ops": [{
                "operation_id": Uuid::new_v4(),
                "lamport_ts": 1,
                "entity": "playlist",
                "entity_id": playlist_canonical,
                "op": "insert",
                "payload": { "name": name },
                "profile_canonical_id": profile_canonical,
            }],
        }))
        .send()
        .await
        .expect("sync push failed");
    assert!(
        resp.status().is_success(),
        "sync push for materialisation must succeed: {}",
        resp.status()
    );
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

// ---------------------------------------------------------------
// by-canonical surface (Phase 1.g.1b)
// ---------------------------------------------------------------

const PROF_CANON: &str = "prof-1111aaaa-1111-4111-8111-111111111111";
const PROF_CANON_B: &str = "prof-2222bbbb-2222-4222-8222-222222222222";

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn by_canonical_mint_resolves_via_public_get(pool: PgPool) {
    let h = spawn_authenticated(pool, "user-share-canon").await;
    let pl_canon = "pl-1g1b-aaaa";

    materialise_playlist_via_sync(&h.base, &h.token, PROF_CANON, pl_canon, "Soirée canon").await;

    let resp: Value = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/share/playlists/by-canonical/{PROF_CANON}/{pl_canon}",
            h.base
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
    assert_eq!(token.len(), 32);

    // Same public GET surface — by-canonical mint is just a different
    // way to land the share_token row, the read side is unchanged.
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
    assert_eq!(public["name"].as_str(), Some("Soirée canon"));
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn by_canonical_mint_is_idempotent(pool: PgPool) {
    let h = spawn_authenticated(pool, "user-share-canon-idem").await;
    let pl_canon = "pl-1g1b-bbbb";

    materialise_playlist_via_sync(&h.base, &h.token, PROF_CANON, pl_canon, "Idem").await;

    let mint_once = || async {
        reqwest::Client::new()
            .post(format!(
                "{}/api/v1/share/playlists/by-canonical/{PROF_CANON}/{pl_canon}",
                h.base
            ))
            .bearer_auth(&h.token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    };

    let t1 = mint_once().await["token"].as_str().unwrap().to_string();
    let t2 = mint_once().await["token"].as_str().unwrap().to_string();
    assert_eq!(t1, t2, "mint must be idempotent");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn by_canonical_revoke_closes_the_link(pool: PgPool) {
    let h = spawn_authenticated(pool, "user-share-canon-revoke").await;
    let pl_canon = "pl-1g1b-cccc";

    materialise_playlist_via_sync(&h.base, &h.token, PROF_CANON, pl_canon, "Revoke me").await;

    // Mint then revoke.
    let mint: Value = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/share/playlists/by-canonical/{PROF_CANON}/{pl_canon}",
            h.base
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
    let token = mint["token"].as_str().unwrap().to_string();

    let revoke = reqwest::Client::new()
        .delete(format!(
            "{}/api/v1/share/playlists/by-canonical/{PROF_CANON}/{pl_canon}",
            h.base
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

    // Public GET now 404s on the revoked token.
    let resp = reqwest::Client::new()
        .get(format!("{}/api/v1/share/playlists/{}", h.base, token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn by_canonical_revoke_then_remint_returns_fresh_token(pool: PgPool) {
    // Mirrors `revoke_then_remint_returns_a_fresh_token` for the
    // by-canonical surface: pins the COALESCE-on-NULL path. After a
    // revoke sets share_token to NULL, the next mint must generate a
    // brand-new candidate rather than re-using the prior value.
    let h = spawn_authenticated(pool, "user-share-canon-remint").await;
    let pl_canon = "pl-1g1b-remint";

    materialise_playlist_via_sync(&h.base, &h.token, PROF_CANON, pl_canon, "Remint").await;

    let mint_url = format!(
        "{}/api/v1/share/playlists/by-canonical/{PROF_CANON}/{pl_canon}",
        h.base
    );

    let first: Value = reqwest::Client::new()
        .post(&mint_url)
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let t1 = first["token"].as_str().unwrap().to_string();

    let revoke = reqwest::Client::new()
        .delete(&mint_url)
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

    let second: Value = reqwest::Client::new()
        .post(&mint_url)
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let t2 = second["token"].as_str().unwrap().to_string();

    assert_eq!(t2.len(), 32);
    assert_ne!(
        t1, t2,
        "revoke + re-mint must produce a new token (NULL → fresh COALESCE candidate)"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn by_canonical_foreign_profile_is_404(pool: PgPool) {
    let two = spawn_two_authenticated(pool, "alice-canon", "bob-canon").await;
    let pl_canon = "pl-1g1b-foreign";

    // Alice materialises the playlist under her profile.
    materialise_playlist_via_sync(&two.base, &two.a.token, PROF_CANON, pl_canon, "Alice's").await;

    // Bob tries to mint with Alice's canonical ids — must 404.
    let resp = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/share/playlists/by-canonical/{PROF_CANON}/{pl_canon}",
            two.base
        ))
        .bearer_auth(&two.b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Bob also can't mint pointing at an unknown profile canonical for
    // his own playlists — same shape.
    let resp = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/share/playlists/by-canonical/{PROF_CANON_B}/{pl_canon}",
            two.base
        ))
        .bearer_auth(&two.b.token)
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
