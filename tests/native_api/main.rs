//! The `/api/v2` surface.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Method;
use axum::http::Request;
use axum::http::StatusCode;
use futures_util::future::BoxFuture;
use tower::ServiceExt;
use waveflow_server::authentication::now_ms;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;
use waveflow_server::services::ServiceError;
use waveflow_server::services::MAX_HISTORY_LIMIT;
use waveflow_server::services::{
    ScrobbleEnvelope, ScrobbleProvider, ScrobbleTarget, ScrobbleVerdict,
};

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod artists;
mod browse;
mod correction_patches;
mod corrections;
mod folders;
mod libraries;
mod lyrics;
mod m3u_import;
mod playlists;
mod radio;
mod scrobbling;
mod tokens;
mod user_data;
