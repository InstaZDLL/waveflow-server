//! The two change feeds: the user journal and the library event stream —
//! claims, cursors and tenancy.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::services::ServiceError;
use waveflow_server::services::MAX_QUEUE_TRACKS;
use waveflow_server::services::MAX_SHARE_TRACKS;
use waveflow_server::storage::catalog::LibraryRecord;
use waveflow_server::storage::database::AccountRole;
use waveflow_server::storage::database::LibraryRole;
use waveflow_server::storage::database::LibraryVisibility;
use waveflow_server::storage::sync::MutationContext;
use waveflow_server::storage::sync::SyncError;
use waveflow_server::storage::sync::MAX_SYNC_LIMIT;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod claims;
mod journal;
mod library_feed;
