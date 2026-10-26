//! Bootstrapping, sessions, tokens and the authorization code flow.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Method;
use axum::http::Request;
use axum::http::StatusCode;
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::storage::database::AccountRole;
use waveflow_server::storage::database::LibraryRole;
use waveflow_server::storage::database::LibraryVisibility;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod oauth;
mod sessions;
mod setup;
mod subsonic_credential;
mod throttling;
mod tokens;

/// Pulls the authorization code out of the redirect the consent step returns.
fn code_from(redirect_to: &str) -> String {
    redirect_to
        .split("code=")
        .nth(1)
        .expect("the redirect carries a code")
        .split('&')
        .next()
        .expect("the code is delimited")
        .to_owned()
}
