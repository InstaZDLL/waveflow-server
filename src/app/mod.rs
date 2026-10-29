//! The assembled server: the state every surface shares, the boot that
//! builds it, the router that serves it and the OpenAPI document it
//! publishes.
//!
//! Private on purpose. What it holds was public at the crate root before it
//! had a module of its own, and is still reached there through the
//! re-exports in `lib.rs`.

mod boot;
pub mod laptop;
mod openapi;
mod router;
mod state;

pub use boot::{initialize, shutdown_signal};
pub use openapi::{ApiDoc, OPENAPI_JSON_PATH, SCALAR_PATH};
pub use router::app;
pub use state::AppState;

#[cfg(test)]
mod tests;
