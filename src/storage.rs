//! Artwork object storage (Phase 1.h.1).
//!
//! Thin wrapper over the Apache `object_store` crate so the rest of
//! the server talks to a single `ArtworkStorage` struct regardless of
//! the backend. `LocalFileSystem` lands in 1.h.1; the S3 backend
//! (object_store's `aws` feature) follows in 1.h.2 without a caller
//! change — same trait, same `put` / `get` / `exists` surface.
//!
//! Layout in the store is flat: every artwork lives at the key
//! `artwork/<blake3_hex>`. The hash is the row identity in
//! `metadata_artwork`; no extension on the key, since the MIME type
//! travels alongside in the Postgres row (so we serve the original
//! Content-Type even though the byte stream is opaque to the store).
//!
//! Size limits, hash validation, and MIME validation live at the
//! HTTP boundary (`api/artwork.rs`); the storage layer trusts its
//! inputs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use object_store::local::LocalFileSystem;
use object_store::path::Path as StorePath;
use object_store::{Error as ObjectStoreError, ObjectStore, PutPayload};
use thiserror::Error;

/// Per-process artwork storage handle. Cheap to clone — the inner
/// `ObjectStore` sits behind an `Arc`. Constructed once at boot
/// alongside the rest of `AppState` and threaded through the
/// `/api/v1/artwork` routes.
#[derive(Clone)]
pub struct ArtworkStorage {
    store: Arc<dyn ObjectStore>,
}

/// Top-level errors a storage call can surface. Distinct from the
/// HTTP layer's status codes so the handler can pick the right
/// response per variant.
#[derive(Debug, Error)]
pub enum StorageError {
    /// The requested key didn't exist in the backend. Maps to HTTP
    /// 404 in the GET handler.
    #[error("artwork not found in object store")]
    NotFound,
    /// The backend returned a non-recoverable error (I/O, perms,
    /// upstream S3 failure once 1.h.2 lands). Maps to HTTP 500.
    #[error("object store backend failed: {0}")]
    Backend(#[from] ObjectStoreError),
}

/// Configuration parsed from env at boot. The artwork endpoints
/// answer 503 when this is `None` — same pattern as streaming, so a
/// deploy without artwork wired up doesn't 5xx, it just declines the
/// feature.
#[derive(Debug, Clone)]
pub struct ArtworkConfig {
    /// `WAVEFLOW_ARTWORK_LOCAL_DIR` — root the LocalFileSystem
    /// backend writes into. Created at boot if missing so a fresh
    /// deploy doesn't need a manual mkdir. The path is canonicalised
    /// once at boot; the per-request path is just a key inside the
    /// backend.
    pub local_dir: PathBuf,
}

impl ArtworkConfig {
    /// Load from env. Returns `Ok(None)` when the feature is
    /// unconfigured — matches the streaming "absent secret = feature
    /// off" convention so a half-set environment doesn't silently
    /// fall back to anything surprising.
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(dir) = std::env::var("WAVEFLOW_ARTWORK_LOCAL_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
        else {
            return Ok(None);
        };
        Ok(Some(Self { local_dir: dir }))
    }
}

impl ArtworkStorage {
    /// Build the LocalFileSystem backend. Creates the root directory
    /// if missing — useful for fresh containers / dev. The
    /// `LocalFileSystem::new_with_prefix` constructor canonicalises
    /// the path internally, so callers don't have to.
    pub fn local(root: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(root).map_err(|err| {
            anyhow::anyhow!("failed to create artwork root {}: {err}", root.display())
        })?;
        let store = LocalFileSystem::new_with_prefix(root).map_err(|err| {
            anyhow::anyhow!(
                "failed to open LocalFileSystem at {}: {err}",
                root.display()
            )
        })?;
        Ok(Self {
            store: Arc::new(store),
        })
    }

    /// Idempotent write. Re-uploading the same hash silently
    /// overwrites the existing object — that's safe because the
    /// BLAKE3 hash is computed from the bytes themselves, so a hash
    /// collision implies byte-equal payloads. The caller (HTTP
    /// handler) is responsible for skipping the upload when the
    /// metadata row already exists; this method exists for the
    /// first-write path and as a recovery hook if the row was
    /// written but the object lost (rare; the row is written after
    /// `put` succeeds in `api::artwork`).
    pub async fn put(&self, hash: &str, bytes: Bytes) -> Result<(), StorageError> {
        let path = key_for(hash);
        self.store.put(&path, PutPayload::from_bytes(bytes)).await?;
        Ok(())
    }

    /// Fetch the object bytes. The HTTP handler buffers the full
    /// payload before responding (artwork caps at 4 MiB, no
    /// streaming benefit); we expose the convenient `Bytes` shape
    /// rather than a `GetResult` stream.
    pub async fn get(&self, hash: &str) -> Result<Bytes, StorageError> {
        let path = key_for(hash);
        match self.store.get(&path).await {
            Ok(result) => {
                let bytes = result.bytes().await?;
                Ok(bytes)
            }
            Err(ObjectStoreError::NotFound { .. }) => Err(StorageError::NotFound),
            Err(other) => Err(StorageError::Backend(other)),
        }
    }

    /// HEAD-style existence check. Used by the upload handler to
    /// short-circuit re-uploads of bytes already in the cache
    /// without paying a full `GET` round-trip.
    pub async fn exists(&self, hash: &str) -> Result<bool, StorageError> {
        let path = key_for(hash);
        match self.store.head(&path).await {
            Ok(_) => Ok(true),
            Err(ObjectStoreError::NotFound { .. }) => Ok(false),
            Err(other) => Err(StorageError::Backend(other)),
        }
    }
}

/// Build the `object_store` key for a hash. Keys are flat
/// `artwork/<hash>` — no sharding, no extension. `object_store`'s
/// `Path` is a URL-safe wrapper; constructing from a known-safe
/// hex hash never fails, but we surface the parse error rather than
/// `unwrap` so a corrupt input can't panic the request thread.
fn key_for(hash: &str) -> StorePath {
    // `Path::from` is infallible-on-display when the input is
    // restricted to `[0-9a-f]` — the boundary already enforces
    // that, so this is a safe construction in practice.
    StorePath::from(format!("artwork/{hash}"))
}

/// Returns `true` when the input matches the BLAKE3 hex shape we
/// accept on the wire. 64 chars, lowercase `[0-9a-f]`. Used by the
/// API layer to reject obviously malformed hashes before any
/// database hit, mirroring the `share` module's `isWellShapedToken`
/// rationale (block any URL-meaningful character at the boundary).
pub fn is_well_shaped_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_valid_blake3_hex() {
        let valid = "0".repeat(64);
        assert!(is_well_shaped_hash(&valid));
        // A real BLAKE3 hash mixing the alphabet.
        assert!(is_well_shaped_hash(
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        ));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!is_well_shaped_hash(""));
        assert!(!is_well_shaped_hash(&"0".repeat(63)));
        assert!(!is_well_shaped_hash(&"0".repeat(65)));
    }

    #[test]
    fn rejects_uppercase() {
        let upper = "AF1349B9F5F9A1A6A0404DEA36DCC9499BCB25C9ADC112B7CC9A93CAE41F3262";
        assert!(!is_well_shaped_hash(upper));
    }

    #[test]
    fn rejects_non_hex_chars() {
        // 'g' is outside the hex alphabet.
        let bad = "g".repeat(64);
        assert!(!is_well_shaped_hash(&bad));
        // Slash would be a path-traversal vector if it slipped past
        // the validator and into `key_for`.
        let mut with_slash = "0".repeat(63);
        with_slash.push('/');
        assert!(!is_well_shaped_hash(&with_slash));
    }

    #[tokio::test]
    async fn local_backend_round_trips_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = ArtworkStorage::local(dir.path()).expect("local storage");
        let hash = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
        let payload = Bytes::from_static(b"\xff\xd8\xff\xe0fake jpeg payload");

        assert!(!storage.exists(hash).await.expect("exists head"));
        storage.put(hash, payload.clone()).await.expect("put");
        assert!(storage.exists(hash).await.expect("exists head after put"));
        let fetched = storage.get(hash).await.expect("get");
        assert_eq!(fetched, payload);
    }

    #[tokio::test]
    async fn local_backend_get_missing_returns_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = ArtworkStorage::local(dir.path()).expect("local storage");
        let hash = "0".repeat(64);
        let err = storage.get(&hash).await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound));
    }
}
