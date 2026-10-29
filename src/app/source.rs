//! Where the source of the running server is (AGPL-3.0 §13).
//!
//! The licence asks a network service to offer its corresponding source to
//! everyone who uses it remotely. This is that offer, stated once: `/health`
//! reports it, the web client links to it from the sign-in page and from
//! Settings, and the OpenAPI document carries it beside the licence.
//!
//! **The tag of this version, not `main`.** A server one release behind would
//! otherwise offer source it is not running. The tag `v<version>` is the one
//! the release workflow builds from, and its archive carries `Cargo.lock` and
//! the `waveflow-core` revision `Cargo.toml` pins — the complete offer that
//! `docs/plans/standalone-download-plan.md` §3 describes.
//!
//! **Read from `Cargo.toml`, not written here.** The repository is the
//! manifest's `repository` field, so whoever distributes a modified build — and
//! owes its users *that* source — moves every link by changing one line there.

/// The source tree of the running version: the repository at its tag.
pub const SOURCE_URL: &str = concat!(
    env!("CARGO_PKG_REPOSITORY"),
    "/tree/v",
    env!("CARGO_PKG_VERSION")
);

/// The licence text, at the same tag as the source it covers.
pub const LICENSE_URL: &str = concat!(
    env!("CARGO_PKG_REPOSITORY"),
    "/blob/v",
    env!("CARGO_PKG_VERSION"),
    "/LICENSE"
);
