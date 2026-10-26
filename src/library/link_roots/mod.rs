//! The folders outside every library that a followed symbolic link may point
//! into (RFC-021, what decision 3 left open).
//!
//! A library that follows links reads a link only when its target stays inside
//! the library root: a link is a way for anyone who can write into the library
//! folder to make the server read whatever it can read. A link farm whose
//! files live elsewhere — a beets `link: yes` import into a separate folder —
//! was therefore not followed at all. `WAVEFLOW_LINK_ROOTS` names the folders
//! an operator vouches for, and a followed link may reach into those as well.
//!
//! **Resolved when used, not when read.** The configured paths are
//! canonicalised at every scan and every stream, so a mount that appears after
//! the boot is honoured, one that vanished reaches nothing, and a configured
//! path that is itself a link is judged by where it points now.
//!
//! **Never into another library.** A target inside another library's root is
//! refused even when an allowed folder holds it: following it would show one
//! library's files to the members of another.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// The configured roots, as the operator wrote them.
#[derive(Debug, Clone, Default)]
pub struct LinkRoots(Arc<Vec<PathBuf>>);

impl LinkRoots {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self(Arc::new(roots))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every configured root that resolves now, canonical. One that does not —
    /// a mount not there, a folder removed — is left out, and reaches nothing.
    pub fn resolved(&self) -> Vec<PathBuf> {
        self.0
            .iter()
            .filter_map(|root| match std::fs::canonicalize(root) {
                Ok(root) if root.is_dir() => Some(root),
                Ok(_) | Err(_) => {
                    tracing::debug!(
                        root = %root.display(),
                        "a link root does not resolve to a folder; links into it are not followed"
                    );
                    None
                }
            })
            .collect()
    }
}

/// What a stored path of one library may lead through when it is read: whether
/// that library follows links at all, and every other library's root, which a
/// link may never resolve into.
///
/// Carried with the track by the read that resolves it (the stream, the
/// quality analysis, the spectrogram), so the rule is checked against the
/// library as it is when the file is opened, not as it was when it was
/// scanned: a link retargeted since, or a links setting turned off since,
/// opens nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkPolicy {
    pub follow_links: bool,
    /// The other libraries' roots as stored, canonicalised when used.
    pub other_libraries: Vec<PathBuf>,
}

impl LinkPolicy {
    /// The columns a read selects as `follow_links` and `other_roots`, the
    /// latter a JSON array of root paths.
    pub fn from_columns(follow_links: i64, other_roots: Option<&str>) -> Self {
        Self {
            follow_links: follow_links != 0,
            other_libraries: other_roots
                .and_then(|roots| serde_json::from_str::<Vec<PathBuf>>(roots).ok())
                .unwrap_or_default(),
        }
    }

    /// The other libraries' roots that resolve now, canonical. One that does
    /// not resolve holds nothing a link could reach.
    pub fn other_libraries_resolved(&self) -> Vec<PathBuf> {
        self.other_libraries
            .iter()
            .filter_map(|root| std::fs::canonicalize(root).ok())
            .collect()
    }
}

/// Whether a canonical `target` lies inside one of the canonical `roots` and
/// inside none of the canonical `libraries` — every other library's root.
pub fn admits(roots: &[PathBuf], libraries: &[PathBuf], target: &Path) -> bool {
    roots.iter().any(|root| target.starts_with(root))
        && !libraries.iter().any(|library| target.starts_with(library))
}

#[cfg(test)]
mod tests;
