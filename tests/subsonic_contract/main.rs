//! The frozen v2.0-beta contract, and what a foreign catalogue may not see.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::storage::catalog::LibraryRecord;
use waveflow_server::storage::database::AccountRole;
use waveflow_server::storage::database::LibraryVisibility;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod compatibility;
mod isolation;
mod play_queue;
