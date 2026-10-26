//! The catalogue model: albums, artists, credits, genres and tenancy.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
use waveflow_server::authentication::now_ms;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;
use waveflow_server::services::ServiceError;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod change_feed;
mod credits;
mod listings;
mod tags;
