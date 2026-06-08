//! Artwork object storage (Phase 1.h.1 + 1.h.2).
//!
//! Thin wrapper over the Apache `object_store` crate so the rest of
//! the server talks to a single `ArtworkStorage` struct regardless of
//! the backend. 1.h.1 shipped `LocalFileSystem`; 1.h.2 adds the S3
//! family (AWS, MinIO, Cloudflare R2, Backblaze B2 — every provider
//! that speaks the S3 wire protocol) without a caller change — same
//! trait, same `put` / `get` / `exists` surface.
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
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path as StorePath;
// object_store 0.13 moved `put` / `get` / `head` off the bare
// `ObjectStore` trait onto the `ObjectStoreExt` extension trait —
// importing both restores the call shape we use against
// `Arc<dyn ObjectStore>` throughout this module + `api/artwork.rs`
// + `artwork_jobs.rs`.
use object_store::{Error as ObjectStoreError, ObjectStore, ObjectStoreExt, PutPayload};
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

/// Type-alias kept for source compatibility with the 1.h.1 wire-up
/// (`Config::artwork: Option<ArtworkConfig>`). The field still names
/// the backend choice; the underlying type is now the richer
/// `ArtworkBackend` enum so 1.h.2's S3 variant is reachable without
/// a churn pass across `Config`, `main.rs`, and the test harness.
pub type ArtworkConfig = ArtworkBackend;

/// Storage backend choice parsed from env at boot. The artwork
/// endpoints answer 503 when this is `None` — same opt-in shape as
/// streaming, so a deploy without artwork configured doesn't 5xx, it
/// just declines the feature.
///
/// `Debug` carries the structural fields but redacts the S3 secret
/// access key — a derived impl would dump it into any `tracing`
/// field that prints the config (same hygiene the parent `Config`
/// already applies to `database_url` and `stream_secret`).
#[derive(Clone)]
pub enum ArtworkBackend {
    /// `WAVEFLOW_ARTWORK_LOCAL_DIR` — root the LocalFileSystem
    /// backend writes into. Created at boot if missing so a fresh
    /// deploy doesn't need a manual mkdir. The path is canonicalised
    /// by `LocalFileSystem::new_with_prefix`; the per-request key is
    /// flat under that root.
    Local { dir: PathBuf },
    /// S3-compatible object storage. The same builder reaches AWS,
    /// MinIO, Cloudflare R2, and Backblaze B2 — non-AWS providers
    /// just need the `endpoint` override. Static credentials only in
    /// 1.h.2; IAM role / IMDS auth would be a follow-up if a deploy
    /// asks for it.
    S3 {
        bucket: String,
        /// Optional endpoint override. `None` reaches `s3.<region>.amazonaws.com`;
        /// set for MinIO (`http://minio:9000`), R2
        /// (`https://<account>.r2.cloudflarestorage.com`), B2
        /// (`https://s3.<region>.backblazeb2.com`).
        endpoint: Option<String>,
        /// AWS region. Required even for non-AWS providers because
        /// `AmazonS3Builder` validates a region is set. MinIO ignores
        /// the value; R2 / B2 echo it back. Default `us-east-1`.
        region: String,
        access_key_id: String,
        /// Redacted in `Debug`. The wrapping struct's redaction relies
        /// on this field staying private to the module.
        secret_access_key: String,
        /// Optional key prefix. Empty by default; set to share a
        /// bucket with other workloads without colliding on the
        /// `artwork/` namespace.
        prefix: Option<String>,
    },
}

impl std::fmt::Debug for ArtworkBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local { dir } => f.debug_struct("Local").field("dir", dir).finish(),
            Self::S3 {
                bucket,
                endpoint,
                region,
                access_key_id,
                secret_access_key: _,
                prefix,
            } => f
                .debug_struct("S3")
                .field("bucket", bucket)
                .field("endpoint", endpoint)
                .field("region", region)
                .field("access_key_id", access_key_id)
                .field("secret_access_key", &"<redacted>")
                .field("prefix", prefix)
                .finish(),
        }
    }
}

impl ArtworkBackend {
    /// Load from env. Returns `Ok(None)` when the feature is
    /// unconfigured — matches the streaming "absent secret = feature
    /// off" convention so a half-set environment doesn't silently
    /// fall back to anything surprising.
    ///
    /// Resolution rules:
    /// - `WAVEFLOW_ARTWORK_S3_BUCKET` set → S3 backend (other S3
    ///   env vars validated; access key + secret are required).
    /// - else `WAVEFLOW_ARTWORK_LOCAL_DIR` set → LocalFileSystem.
    /// - else `None` (feature off).
    /// - Both set at once → boot fails. The two backends are
    ///   mutually exclusive (one server, one cache), and silently
    ///   picking a winner would let an operator believe they're
    ///   storing to S3 when the local path won (or vice versa).
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let local_dir = env_nonempty("WAVEFLOW_ARTWORK_LOCAL_DIR").map(PathBuf::from);
        let bucket = env_nonempty("WAVEFLOW_ARTWORK_S3_BUCKET");

        match (local_dir, bucket) {
            (Some(_), Some(_)) => anyhow::bail!(
                "WAVEFLOW_ARTWORK_LOCAL_DIR and WAVEFLOW_ARTWORK_S3_BUCKET are mutually \
                 exclusive — choose one backend or unset both to disable artwork"
            ),
            (Some(dir), None) => Ok(Some(Self::Local { dir })),
            (None, Some(bucket)) => {
                let access_key_id =
                    env_nonempty("WAVEFLOW_ARTWORK_S3_ACCESS_KEY_ID").ok_or_else(|| {
                        anyhow::anyhow!(
                            "WAVEFLOW_ARTWORK_S3_ACCESS_KEY_ID is required when \
                             WAVEFLOW_ARTWORK_S3_BUCKET is set"
                        )
                    })?;
                let secret_access_key = env_nonempty("WAVEFLOW_ARTWORK_S3_SECRET_ACCESS_KEY")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "WAVEFLOW_ARTWORK_S3_SECRET_ACCESS_KEY is required when \
                             WAVEFLOW_ARTWORK_S3_BUCKET is set"
                        )
                    })?;
                let region = env_nonempty("WAVEFLOW_ARTWORK_S3_REGION")
                    .unwrap_or_else(|| "us-east-1".to_string());
                let endpoint = env_nonempty("WAVEFLOW_ARTWORK_S3_ENDPOINT");
                let prefix = env_nonempty("WAVEFLOW_ARTWORK_S3_PREFIX");

                Ok(Some(Self::S3 {
                    bucket,
                    endpoint,
                    region,
                    access_key_id,
                    secret_access_key,
                    prefix,
                }))
            }
            (None, None) => Ok(None),
        }
    }
}

/// Helper — read an env var, treat empty strings as unset. Matches
/// the same convention the streaming knobs use (`std::env::var`
/// returns `Ok("")` for exported-but-empty, which would otherwise
/// slip a zero-byte credential past the structural checks).
fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

impl ArtworkStorage {
    /// Dispatch to the right backend constructor. The single
    /// entrypoint `main.rs` calls at boot — anything more specific
    /// (`local`, `s3`) stays available for tests that want to wire
    /// a single backend directly without going through env.
    pub fn from_backend(backend: &ArtworkBackend) -> anyhow::Result<Self> {
        match backend {
            ArtworkBackend::Local { dir } => Self::local(dir),
            ArtworkBackend::S3 {
                bucket,
                endpoint,
                region,
                access_key_id,
                secret_access_key,
                prefix,
            } => Self::s3(
                bucket,
                endpoint.as_deref(),
                region,
                access_key_id,
                secret_access_key,
                prefix.as_deref(),
            ),
        }
    }

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

    /// Build the S3 backend. Reaches AWS by default; the `endpoint`
    /// override aims the same builder at MinIO / R2 / B2. The
    /// `prefix` is folded into a `PrefixStore` wrapper so every
    /// `put` / `get` lives under it without the caller threading the
    /// prefix through `key_for`.
    ///
    /// Validation is deferred to `AmazonS3Builder::build` — the
    /// crate already enforces a bucket + region pair. The boot path
    /// in `main.rs` only sees a failure if a credential is rejected
    /// at first use, but that's surfaced through `StorageError`
    /// once the first request lands.
    pub fn s3(
        bucket: &str,
        endpoint: Option<&str>,
        region: &str,
        access_key_id: &str,
        secret_access_key: &str,
        prefix: Option<&str>,
    ) -> anyhow::Result<Self> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(bucket)
            .with_region(region)
            .with_access_key_id(access_key_id)
            .with_secret_access_key(secret_access_key);
        if let Some(ep) = endpoint {
            // Custom endpoint (MinIO / R2 / B2) needs the virtual-
            // hosted-vs-path style flipped: most non-AWS providers
            // require path-style addressing. AWS itself supports both
            // and ignores the flag, so this is safe to set
            // unconditionally for the endpoint-override branch.
            builder = builder.with_endpoint(ep).with_allow_http(true);
        }
        let store = builder
            .build()
            .map_err(|err| anyhow::anyhow!("failed to build S3 backend: {err}"))?;

        // `object_store::prefix::PrefixStore` transparently scopes
        // every key under the supplied prefix. We use it instead of
        // mutating `key_for` so the shared key shape (`artwork/<hash>`)
        // stays a single source of truth.
        let prefixed: Arc<dyn ObjectStore> = match prefix.filter(|p| !p.is_empty()) {
            Some(prefix) => Arc::new(object_store::prefix::PrefixStore::new(
                store,
                StorePath::from(prefix),
            )),
            None => Arc::new(store),
        };
        Ok(Self { store: prefixed })
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

    /// Single test for every branch of [`ArtworkBackend::from_env`].
    /// Bundled into one function so the env-var mutation stays
    /// serialised — splitting into multiple `#[test]` would let
    /// cargo's parallel runner observe partial state from a peer
    /// test (the process-global `std::env` table is shared).
    ///
    /// Variables we touch are all under the `WAVEFLOW_ARTWORK_*`
    /// prefix that no other test in the workspace reads or writes;
    /// the final block restores `None` for each so nothing leaks to
    /// later tests if this one ever runs before another that probes
    /// the same surface (none today).
    #[test]
    fn from_env_resolves_backend_choice() {
        // Helper closures keep the unsafe blocks short; in Rust
        // 2024 `std::env::set_var` / `remove_var` are `unsafe`
        // (mutating the process env race-conditions with peer
        // threads reading it), which is precisely why this test
        // is one serial function rather than five parallel ones.
        let setv = |k: &str, v: &str| unsafe { std::env::set_var(k, v) };
        let unsetv = |k: &str| unsafe { std::env::remove_var(k) };

        // Pre-clean — paranoia in case a prior crashed test left
        // a var dangling.
        for var in [
            "WAVEFLOW_ARTWORK_LOCAL_DIR",
            "WAVEFLOW_ARTWORK_S3_BUCKET",
            "WAVEFLOW_ARTWORK_S3_ACCESS_KEY_ID",
            "WAVEFLOW_ARTWORK_S3_SECRET_ACCESS_KEY",
            "WAVEFLOW_ARTWORK_S3_REGION",
            "WAVEFLOW_ARTWORK_S3_ENDPOINT",
            "WAVEFLOW_ARTWORK_S3_PREFIX",
        ] {
            unsetv(var);
        }

        // Both unset → feature off (None).
        assert!(ArtworkBackend::from_env().expect("clean env").is_none());

        // Local only → Local variant.
        setv("WAVEFLOW_ARTWORK_LOCAL_DIR", "/tmp/wf-artwork-test");
        let backend = ArtworkBackend::from_env()
            .expect("local set")
            .expect("local backend resolved");
        assert!(
            matches!(&backend, ArtworkBackend::Local { dir } if dir == Path::new("/tmp/wf-artwork-test")),
            "expected Local variant, got {backend:?}",
        );
        unsetv("WAVEFLOW_ARTWORK_LOCAL_DIR");

        // S3 bucket set without credentials → boot bails with the
        // specific missing-var named in the error message.
        setv("WAVEFLOW_ARTWORK_S3_BUCKET", "wf-artwork");
        let err = ArtworkBackend::from_env().expect_err("creds missing");
        assert!(
            err.to_string()
                .contains("WAVEFLOW_ARTWORK_S3_ACCESS_KEY_ID"),
            "error must name the missing access-key var: {err}",
        );

        // Full S3 set → S3 variant with the documented default
        // region applied.
        setv("WAVEFLOW_ARTWORK_S3_ACCESS_KEY_ID", "AKIA-test");
        setv("WAVEFLOW_ARTWORK_S3_SECRET_ACCESS_KEY", "secret-test");
        let backend = ArtworkBackend::from_env()
            .expect("S3 set")
            .expect("S3 backend resolved");
        match &backend {
            ArtworkBackend::S3 {
                bucket,
                region,
                endpoint,
                prefix,
                access_key_id,
                ..
            } => {
                assert_eq!(bucket, "wf-artwork");
                assert_eq!(region, "us-east-1", "default region should apply");
                assert!(endpoint.is_none());
                assert!(prefix.is_none());
                assert_eq!(access_key_id, "AKIA-test");
            }
            other => panic!("expected S3 variant, got {other:?}"),
        }

        // Both backends set at once → bail with the mutually-
        // exclusive message; protects an operator from quietly
        // pointing at the wrong backend after a botched migration.
        setv("WAVEFLOW_ARTWORK_LOCAL_DIR", "/tmp/wf-artwork-test");
        let err = ArtworkBackend::from_env().expect_err("both set");
        assert!(
            err.to_string().contains("mutually exclusive"),
            "error must flag the mutually-exclusive violation: {err}",
        );

        // Cleanup so later tests see a pristine env table.
        for var in [
            "WAVEFLOW_ARTWORK_LOCAL_DIR",
            "WAVEFLOW_ARTWORK_S3_BUCKET",
            "WAVEFLOW_ARTWORK_S3_ACCESS_KEY_ID",
            "WAVEFLOW_ARTWORK_S3_SECRET_ACCESS_KEY",
        ] {
            unsetv(var);
        }
    }

    /// Smoke test on the S3 builder construction path. Pure-static
    /// validation — no network round-trip — so it runs in CI
    /// without an S3 endpoint.
    #[test]
    fn s3_backend_builds_with_endpoint_override() {
        let storage = ArtworkStorage::s3(
            "wf-artwork",
            Some("http://localhost:9000"),
            "us-east-1",
            "AKIA-test",
            "secret-test",
            Some("dev/"),
        );
        assert!(
            storage.is_ok(),
            "S3 builder should succeed with full input: {:?}",
            storage.err(),
        );
    }
}
