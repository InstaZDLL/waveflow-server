//! Smoke tests for the OpenAPI spec + Scalar API reference.
//!
//! Guards two regressions a future refactor could introduce silently:
//! - removing the `#[utoipa::path]` macro from a handler (the path
//!   disappears from `paths` and the docs go stale)
//! - breaking the Scalar UI mount (a 404 is invisible until someone
//!   actually visits `/reference` in a browser)

mod support;

use serde_json::Value;
use sqlx::PgPool;
use support::spawn_app;
use waveflow_server::{OPENAPI_JSON_PATH, SCALAR_PATH};

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn openapi_doc_lists_every_handler(pool: PgPool) {
    let base = spawn_app(pool).await;

    let spec: Value = reqwest::Client::new()
        .get(format!("{base}{OPENAPI_JSON_PATH}"))
        .send()
        .await
        .expect("request failed")
        .error_for_status()
        .expect("non-2xx response")
        .json()
        .await
        .expect("invalid JSON");

    assert_eq!(spec["info"]["title"], "waveflow-server");
    assert_eq!(spec["openapi"].as_str().unwrap().chars().next(), Some('3'));

    let paths = spec["paths"].as_object().expect("paths must be a map");

    // Every handler shipped in 1.b.* must be advertised. If a future
    // refactor drops the `#[utoipa::path]` macro from one of these,
    // this test catches it before the docs go stale.
    assert!(paths.contains_key("/health"), "missing /health in spec");
    assert!(paths.contains_key("/ready"), "missing /ready in spec");
    assert!(
        paths.contains_key("/api/v1/profiles/{profile_id}/libraries"),
        "missing /api/v1/profiles/{{profile_id}}/libraries in spec"
    );
    assert!(
        paths.contains_key("/api/v1/profiles/{profile_id}/libraries/{id}"),
        "missing /api/v1/profiles/{{profile_id}}/libraries/{{id}} in spec"
    );
    assert!(
        paths.contains_key("/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"),
        "missing tracks collection path in spec"
    );
    assert!(
        paths.contains_key("/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}"),
        "missing tracks item path in spec"
    );
    assert!(
        paths.contains_key("/api/v1/profiles/{profile_id}/playlists"),
        "missing playlists collection path in spec"
    );
    assert!(
        paths.contains_key("/api/v1/profiles/{profile_id}/playlists/{id}"),
        "missing playlists item path in spec"
    );
    assert!(
        paths.contains_key("/api/v1/profiles/{profile_id}/playlists/{id}/tracks"),
        "missing playlist tracks path in spec"
    );
    assert!(
        paths.contains_key(
            "/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{track_id}/stream-url"
        ),
        "missing stream-url mint path in spec"
    );
    assert!(
        paths.contains_key("/api/v1/stream/{token}"),
        "missing stream path in spec"
    );

    // Phase 1.f sync surface — drop the `#[utoipa::path]` macro from
    // any of these and the desktop / web stop seeing it in the docs.
    assert!(
        paths.contains_key("/api/v1/sync/ops"),
        "missing /api/v1/sync/ops in spec"
    );
    assert!(
        paths.contains_key("/api/v1/sync/ack"),
        "missing /api/v1/sync/ack in spec"
    );

    // `POST /api/v1/users` retired in Phase 1.d.2 — the JWT path's
    // lazy auto-provisioning is the only way to land a `users` row,
    // so the spec must NOT advertise the legacy bootstrap endpoint.
    assert!(
        !paths.contains_key("/api/v1/users"),
        "spec still advertises /api/v1/users — endpoint retired in 1.d.2"
    );

    // The /ready operation must declare both the 200 and the 503
    // shape — the readiness contract is a 503-as-data API and the
    // generated docs need to communicate it.
    let ready_responses = &paths["/ready"]["get"]["responses"];
    assert!(
        ready_responses["200"].is_object(),
        "ready endpoint missing 200 response"
    );
    assert!(
        ready_responses["503"].is_object(),
        "ready endpoint missing 503 response"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn scalar_ui_mounted(pool: PgPool) {
    let base = spawn_app(pool).await;

    let resp = reqwest::Client::new()
        .get(format!("{base}{SCALAR_PATH}"))
        .send()
        .await
        .expect("request failed")
        .error_for_status()
        .expect("non-2xx response");

    let content_type = resp
        .headers()
        .get("content-type")
        .expect("missing content-type")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        content_type.starts_with("text/html"),
        "Scalar UI must serve HTML, got {content_type}"
    );

    let body = resp.text().await.expect("body");
    // The Scalar bundle is referenced by URL in the template; a
    // presence check on the marker string is enough — we don't want
    // to assert on the exact vendor URL since utoipa-scalar may bump
    // it across minor releases.
    assert!(
        body.to_lowercase().contains("scalar"),
        "Scalar template marker missing in /reference body"
    );
}
