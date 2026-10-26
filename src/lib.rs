//! WaveFlow Server v2 library surface.

pub mod api;
mod app;
pub mod auth;
pub mod cli;
pub mod config;
pub mod http;
pub mod library;
pub mod media;
pub mod quality;
pub mod remote;
pub mod services;
pub mod storage;
pub mod subsonic;

pub use app::{app, initialize, shutdown_signal, ApiDoc, AppState, OPENAPI_JSON_PATH, SCALAR_PATH};
pub use config::Config;
