//! Probes, the embedded client, the backup bundle, and how a request is named.
//!
//! Split out of `v2_foundations.rs`; the command-line tests left for `cli/`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::storage::database::AccountRole;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod lifecycle;
mod probes;
mod web;
