//! Reading the music on disk: the scanner and the watcher that wakes it, the
//! tag mapper, the identifiers derived from tags, credit splitting, the roots a
//! followed link may reach, playlist files and lyrics sheets.

pub mod credit_split;
pub mod link_roots;
pub mod lyrics;
pub mod pid;
pub mod playlist_file;
pub mod scanner;
pub mod tags;
pub mod watcher;
