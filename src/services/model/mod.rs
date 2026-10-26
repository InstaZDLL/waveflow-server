//! The records the domain services answer with, one file per domain.
//!
//! Every one of them is re-exported from `services`, which is the path the
//! API, the Subsonic façade and the tests name it by.

pub(super) mod accounts;
pub(super) mod browse;
pub(super) mod catalog;
pub(super) mod enrichment;
pub(super) mod library;
pub(super) mod metadata;
pub(super) mod personal;
pub(super) mod playlists;
pub(super) mod uploads;

#[cfg(test)]
mod tests;
