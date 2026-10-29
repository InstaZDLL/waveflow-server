//! The assembled server: the state every surface shares, the boot that
//! builds it, the router that serves it, the OpenAPI document it
//! publishes and where its own source is.
//!
//! Private on purpose. What it holds was public at the crate root before it
//! had a module of its own, and is still reached there through the
//! re-exports in `lib.rs`.

mod boot;
mod openapi;
mod router;
mod source;
mod state;

pub use boot::{initialize, shutdown_signal};
pub use openapi::{ApiDoc, OPENAPI_JSON_PATH, SCALAR_PATH};
pub use router::app;
pub(crate) use source::{LICENSE_URL, SOURCE_URL};
pub use state::AppState;

#[cfg(test)]
mod tests;
