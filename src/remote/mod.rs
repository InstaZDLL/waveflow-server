//! Everything that talks to another host: the outbound HTTP client, metadata
//! enrichment and the portraits it fetches, LRCLIB lyrics, podcast feeds and
//! the scrobbling adapters.

pub mod enrichment;
pub mod lrclib;
pub mod outbound;
pub mod podcast;
pub mod portrait;
pub mod scrobblers;
