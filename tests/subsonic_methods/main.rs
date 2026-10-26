//! The facade methods that are neither browsing nor a wire shape.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::services::ServiceError;
use waveflow_server::storage::database::AccountRole;
use waveflow_server::storage::database::LibraryRole;
use waveflow_server::storage::database::LibraryVisibility;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod bookmarks;
mod playlists;
mod radio;
mod remaining_methods;
mod transcoding;

// An account with a Subsonic API key drawn at run time, so no credential is
/// ever written into the source.
async fn subsonic_account(state: &waveflow_server::AppState, username: &str) -> (Uuid, String) {
    let hash = security::hash_password(&Uuid::new_v4().to_string()).unwrap();
    let account = state
        .db
        .create_account(username, &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let api_key = format!("wfsk_{}", Uuid::new_v4().simple());
    state
        .db
        .set_subsonic_credential(
            account,
            account,
            &state
                .secret_box
                .encrypt(Uuid::new_v4().to_string().as_bytes())
                .unwrap(),
            &security::token_hash(&api_key),
            now_ms(),
        )
        .await
        .unwrap();
    (account, api_key)
}
