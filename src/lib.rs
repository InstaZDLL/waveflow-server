//! WaveFlow Server v2 library surface.

// The library exists so the integration suite drives the router `main`
// serves, and its doc comments link private items freely: a constant, a
// projection macro, the method a public one defers to. Those links resolve
// in an editor and under `cargo doc --document-private-items`; what CI's
// `cargo doc` gate catches is a link that resolves nowhere.
#![allow(rustdoc::private_intra_doc_links)]

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

pub use app::{
    app, initialize, laptop, shutdown_signal, ApiDoc, AppState, OPENAPI_JSON_PATH, SCALAR_PATH,
};
pub use config::Config;
