//! The public face of the engine: building it from the settings, reading the
//! cache and scheduling fills, and the Deezer portrait search.

use std::{path::PathBuf, sync::Arc, time::Duration};

use dashmap::DashMap;
use waveflow_core::metadata::{
    deezer::{is_placeholder_artist_picture, DeezerArtistHit, DeezerClient},
    lastfm::LastfmClient,
    theaudiodb::TheAudioDbClient,
};

use crate::{
    config::MetadataSettings,
    remote::portrait::{self, AddressPolicy},
    storage::database::Database,
};

use super::{
    engine::{Client, Engine, Source},
    CachedArtist, CandidateSearchError, DeezerCandidate, Pace, Provider, StoredPortrait,
};

/// Last.fm's API root. A constant, not a setting (decision 9): there is no
/// operator field, so there is no URL to validate.
const LASTFM_BASE_URL: &str = "https://ws.audioscrobbler.com/2.0/";

/// Deezer's API root, a constant for the same reason. It asks for no key.
const DEEZER_BASE_URL: &str = "https://api.deezer.com";

/// The hosts Deezer serves its pictures from (decision 8). A picture URL
/// naming any other host is dropped without being contacted.
const DEEZER_IMAGE_HOSTS: [&str; 2] = ["e-cdns-images.dzcdn.net", "cdn-images.dzcdn.net"];

/// TheAudioDB's API root, a constant like the others (RFC-014 decision 1). The
/// operator's key travels in the path, so this URL is never logged either.
const THEAUDIODB_BASE_URL: &str = "https://www.theaudiodb.com/api/v1/json";

/// The hosts TheAudioDB serves its fanart from (RFC-014 decision 2). A fanart
/// URL naming any other host is dropped without being contacted.
const THEAUDIODB_IMAGE_HOSTS: [&str; 2] = ["r2.theaudiodb.com", "www.theaudiodb.com"];

/// TheAudioDB's own ceiling, whatever the shared rate says: thirty requests a
/// minute on its free tier (RFC-014 decision 5). The pace is the slower of this
/// and the operator's.
const THEAUDIODB_INTERVAL: Duration = Duration::from_secs(2);

/// Where each provider is reached. [`Endpoints::default`] is production; the
/// test suite points these at listeners of its own, which is the only reason
/// the type exists.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub lastfm: String,
    pub deezer: String,
    /// The hosts a Deezer picture may be fetched from.
    pub deezer_image_hosts: Vec<String>,
    pub theaudiodb: String,
    /// The hosts a TheAudioDB fanart may be fetched from.
    pub theaudiodb_image_hosts: Vec<String>,
    /// The addresses a picture may be fetched from. Only the test suite,
    /// whose image host is on loopback, sets anything but the default.
    pub image_addresses: AddressPolicy,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            lastfm: LASTFM_BASE_URL.to_owned(),
            deezer: DEEZER_BASE_URL.to_owned(),
            deezer_image_hosts: DEEZER_IMAGE_HOSTS.map(str::to_owned).to_vec(),
            theaudiodb: THEAUDIODB_BASE_URL.to_owned(),
            theaudiodb_image_hosts: THEAUDIODB_IMAGE_HOSTS.map(str::to_owned).to_vec(),
            image_addresses: AddressPolicy::default(),
        }
    }
}

/// The engine, or its absence.
///
/// Cheap to clone; every clone drives the same engine. Disabled when no
/// provider is configured, and then every call returns at once having read and
/// sent nothing.
#[derive(Clone)]
pub struct Enrichment {
    engine: Option<Arc<Engine>>,
}

impl Enrichment {
    /// Enrichment switched off.
    pub fn disabled() -> Self {
        Self { engine: None }
    }

    /// The engine `settings` describe, against the providers' real addresses.
    pub fn new(
        db: Database,
        settings: &MetadataSettings,
        artwork_dir: PathBuf,
    ) -> anyhow::Result<Self> {
        Self::with_endpoints(db, settings, artwork_dir, Endpoints::default())
    }

    /// [`Self::new`], with the providers reached at `endpoints`.
    ///
    /// Only the configured providers are built. A key placed with no provider
    /// naming it builds nothing — the provider list is the switch, and a key
    /// is not consent (decision 3). With `WAVEFLOW_METADATA_IMAGES=off`,
    /// Deezer is not built either: portraits are all it is asked for.
    pub fn with_endpoints(
        db: Database,
        settings: &MetadataSettings,
        artwork_dir: PathBuf,
        endpoints: Endpoints,
    ) -> anyhow::Result<Self> {
        let mut sources = Vec::new();
        for &provider in &settings.providers {
            let http = crate::remote::outbound::client(settings.request_timeout)
                .map_err(|_| anyhow::anyhow!("the outbound client could not be built"))?;
            let client = match provider {
                Provider::LastFm => {
                    let key = settings.lastfm_key.clone().ok_or_else(|| {
                        anyhow::anyhow!("metadata provider lastfm is configured without a key")
                    })?;
                    Client::LastFm {
                        client: LastfmClient::with_http(http, endpoints.lastfm.clone())?,
                        key,
                    }
                }
                Provider::Deezer if !settings.images => continue,
                Provider::Deezer => Client::Deezer {
                    client: DeezerClient::with_http(http, endpoints.deezer.clone()),
                    image_hosts: endpoints.deezer_image_hosts.clone(),
                },
                // Banners are all it is asked for (RFC-014 decision 1).
                Provider::TheAudioDb if !settings.images => continue,
                Provider::TheAudioDb => {
                    let key = settings.theaudiodb_key.clone().ok_or_else(|| {
                        anyhow::anyhow!("metadata provider theaudiodb is configured without a key")
                    })?;
                    Client::TheAudioDb {
                        client: TheAudioDbClient::with_http(
                            http,
                            endpoints.theaudiodb.clone(),
                            key,
                        )
                        .map_err(|_| anyhow::anyhow!("the TheAudioDB address is not https"))?,
                        image_hosts: endpoints.theaudiodb_image_hosts.clone(),
                    }
                }
            };
            let pace = match provider {
                Provider::TheAudioDb => {
                    Pace::with_floor(settings.rate_per_sec, THEAUDIODB_INTERVAL)
                }
                Provider::LastFm | Provider::Deezer => Pace::new(settings.rate_per_sec),
            };
            sources.push(Source {
                provider,
                client,
                pace,
            });
        }
        if sources.is_empty() {
            return Ok(Self::disabled());
        }
        Ok(Self {
            engine: Some(Arc::new(Engine {
                db,
                sources,
                ttl_ms: duration_ms(settings.ttl),
                miss_ttl_ms: duration_ms(settings.miss_ttl),
                request_timeout: settings.request_timeout,
                artwork_dir,
                image_addresses: endpoints.image_addresses,
                in_flight: DashMap::new(),
                running: tokio::sync::watch::channel(0).0,
            })),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.engine.is_some()
    }

    /// How many providers are configured — the number of rows a name has
    /// once every one of them has answered.
    pub fn provider_count(&self) -> usize {
        self.engine
            .as_ref()
            .map_or(0, |engine| engine.sources.len())
    }

    /// Whether a portrait may be searched for on Deezer: the operator named
    /// it and left images on. Asked at every call, never remembered by a
    /// search (RFC-013 decision 4).
    pub(crate) fn searches_deezer(&self) -> bool {
        self.engine
            .as_ref()
            .is_some_and(|engine| engine.deezer().is_some())
    }

    /// Deezer's artists for `query`, in its order, at most `limit` of them,
    /// each with its picture fetched and stored as a preview (RFC-013
    /// decision 4).
    ///
    /// Every request — the search and each picture — keeps Deezer's pace,
    /// the one enrichment keeps, so a picker cannot make the server ask
    /// faster than the operator allowed. A hit with no real picture is not
    /// offered, nor one whose picture the fetcher refuses for what it is:
    /// there would be nothing to choose. A picture host that cannot be
    /// reached fails the whole search, as the search itself failing does.
    pub(crate) async fn deezer_candidates(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<DeezerCandidate>, CandidateSearchError> {
        let engine = self
            .engine
            .as_ref()
            .ok_or(CandidateSearchError::NotConfigured)?;
        let (source, client, image_hosts) =
            engine.deezer().ok_or(CandidateSearchError::NotConfigured)?;
        source.pace.wait().await;
        let hits = engine
            .bounded(client.search_artist(query))
            .await
            .map_err(|verdict| {
                tracing::warn!(
                    provider = source.provider.as_str(),
                    verdict = %verdict,
                    "portrait candidates unreached"
                );
                CandidateSearchError::Unreached
            })?;
        let mut candidates: Vec<DeezerCandidate> = Vec::new();
        for hit in hits {
            if candidates.len() >= limit {
                break;
            }
            if candidates.iter().any(|offered| offered.deezer_id == hit.id) {
                continue;
            }
            let Some(picture) = preview_picture(&hit) else {
                continue;
            };
            source.pace.wait().await;
            let fetched = portrait::fetch(
                &picture,
                image_hosts,
                engine.image_addresses,
                engine.request_timeout,
            )
            .await;
            let preview = match fetched {
                Ok(preview) => preview,
                Err(refusal) if refusal.is_answer() => continue,
                Err(refusal) => {
                    tracing::warn!(
                        provider = source.provider.as_str(),
                        verdict = %refusal,
                        "portrait candidate preview unreached"
                    );
                    return Err(CandidateSearchError::Unreached);
                }
            };
            portrait::store(&engine.artwork_dir, &preview)
                .await
                .map_err(|_| CandidateSearchError::Unreached)?;
            candidates.push(DeezerCandidate {
                deezer_id: hit.id,
                name: hit.name,
                fans: hit.nb_fan,
                preview: StoredPortrait {
                    hash: preview.hash,
                    format: preview.format,
                    byte_size: preview.bytes.len() as i64,
                },
            });
        }
        Ok(candidates)
    }

    /// What the cache holds about the artist called `name`, one row per
    /// configured provider that has one, in the operator's order — and a fill
    /// scheduled for every configured provider whose row is missing or stale.
    ///
    /// Stale rows are returned, not withheld (decision 5): a slightly old
    /// biography beats none, and serving it keeps a provider outage invisible.
    /// Nothing here waits on a provider.
    pub async fn artist(&self, name: &str) -> Result<Vec<CachedArtist>, sqlx::Error> {
        let Some(engine) = &self.engine else {
            return Ok(Vec::new());
        };
        let canonical = waveflow_core::scanner::canonical_name(name);
        if canonical.is_empty() {
            return Ok(Vec::new());
        }
        let rows = engine.db.metadata_artist_rows(&canonical).await?;
        let now = crate::auth::authentication::now_ms();
        let mut ordered = Vec::new();
        for (index, source) in engine.sources.iter().enumerate() {
            let row = rows.iter().find(|row| row.provider == source.provider);
            if row.is_none_or(|row| row.expires_at <= now) {
                Arc::clone(engine).schedule(index, name, &canonical);
            }
            ordered.extend(row.cloned());
        }
        // A portrait already fetched, and an artist row that does not show
        // it: an artist re-identified by a PID spec change, or one scanned
        // since. Linked again from the cache, without a second download. A
        // repair, not the answer: if it fails, the page is still served and
        // the next visit tries again.
        if let Some(hash) = ordered.iter().find_map(|row| row.image_hash.as_deref()) {
            if let Err(error) = engine.db.link_artist_portrait(&canonical, hash).await {
                tracing::warn!(%error, "cached artist portrait could not be linked");
            }
        }
        // And the banner, by the same reasoning (RFC-014 decision 3).
        if let Some(hash) = ordered.iter().find_map(|row| row.fanart_hash.as_deref()) {
            if let Err(error) = engine.db.link_artist_fanart(&canonical, hash).await {
                tracing::warn!(%error, "cached artist banner could not be linked");
            }
        }
        Ok(ordered)
    }

    /// Waits until no fill is running.
    ///
    /// For the test suite, which has to know a fill is over before it can say
    /// what the fill wrote — or that it wrote nothing.
    #[doc(hidden)]
    pub async fn idle(&self) {
        let Some(engine) = &self.engine else {
            return;
        };
        let mut running = engine.running.subscribe();
        // The sender lives as long as the engine, which `self` holds.
        let _ = running.wait_for(|count| *count == 0).await;
    }
}

/// The picture a candidate is previewed at, and kept at if chosen: Deezer's
/// big size, 500 pixels, since the preview is what becomes the portrait. The
/// largest real one when that size is missing; none for Deezer's placeholder,
/// which every size shares.
fn preview_picture(hit: &DeezerArtistHit) -> Option<String> {
    hit.picture_big
        .clone()
        .filter(|url| !is_placeholder_artist_picture(url))
        .or_else(|| hit.best_picture())
}

fn duration_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
