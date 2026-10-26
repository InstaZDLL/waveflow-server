//! Streaming, transcoding and the tickets a browser plays from.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use tower::ServiceExt;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::storage::catalog::LibraryRecord;
use waveflow_server::storage::database::AccountRole;
use waveflow_server::storage::database::LibraryVisibility;
use waveflow_server::Config;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod shares;
mod stream_tickets;
mod streaming;
mod transcoding;
