//! Metadata enrichment — RFC-011: the cache, and the engine that fills it.
//!
//! **Demand-driven and never on the answering path** (decision 6). A reader asks
//! [`Enrichment::artist`] for what the cache holds; for every configured
//! provider with nothing fresh, that call schedules a fill in the background
//! and returns at once. The first visit to an artist shows nothing; the next
//! one shows what came back.
//!
//! **Three outcomes** (decision 5), written on the provider's own row: a *hit*
//! stands for the long TTL, a *miss* — the provider answered and knows nothing
//! — for the short one, and *unreached* writes nothing at all, so the next
//! visit asks again. A network blip must never become "this artist has no
//! biography" for a month.
//!
//! **Bounded per provider**: requests are spaced to the configured rate, and
//! concurrent first visits for one name collapse onto a single fill. The
//! writer gate is taken for the row write only, after the requests are over.
//!
//! **Nothing about a request is logged but the provider and a verdict.** A
//! Last.fm request carries the operator's key in its query string; neither the
//! URL nor a response body is ever written to a log (decision 9).

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use waveflow_core::metadata::{
    deezer::{is_placeholder_artist_picture, DeezerArtistHit, DeezerClient, DeezerError},
    lastfm::LastfmClient,
    name_match::{normalize_name, select_by_name},
    theaudiodb::TheAudioDbClient,
    LookupError,
};

use crate::{
    config::MetadataSettings,
    database::Database,
    portrait::{self, AddressPolicy, Refusal},
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

/// How many neighbours are asked for and kept. Subsonic's `getArtistInfo`
/// defaults to twenty; asking for more leaves room for the ones a caller cannot
/// see, which decision 7 drops at read time.
const SIMILAR_LIMIT: u32 = 50;

/// How many fills may be waiting or running at once, across providers.
///
/// Pacing reserves a slot for every request and refuses none, so without this
/// a page that opens two thousand artists would queue two thousand fills —
/// a quarter of an hour at the default rate — and an outage, which writes no
/// row, would let every further visit add more. Beyond the ceiling nothing is
/// scheduled; the visit after asks again, which is what an unreached lookup
/// does anyway.
pub const MAX_IN_FLIGHT: usize = 64;

/// A provider an operator can name in `WAVEFLOW_METADATA_PROVIDERS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    LastFm,
    /// Portraits only: Deezer has no biography, and its related artists are
    /// not asked for.
    Deezer,
    /// Banners only (RFC-014): TheAudioDB's fanart. Its biographies wait on a
    /// language this server does not know.
    TheAudioDb,
}

impl Provider {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "lastfm" => Some(Self::LastFm),
            "deezer" => Some(Self::Deezer),
            "theaudiodb" => Some(Self::TheAudioDb),
            _ => None,
        }
    }

    /// The name in the setting, in the cache's `provider` column, and in logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LastFm => "lastfm",
            Self::Deezer => "deezer",
            Self::TheAudioDb => "theaudiodb",
        }
    }
}

/// What a provider said, when it said something. Unreached has no variant: it
/// is the absence of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Hit,
    Miss,
}

impl Outcome {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hit" => Some(Self::Hit),
            "miss" => Some(Self::Miss),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
        }
    }
}

/// One provider's cached row for one name.
#[derive(Debug, Clone)]
pub struct CachedArtist {
    pub provider: Provider,
    pub outcome: Outcome,
    pub bio_summary: Option<String>,
    pub bio_full: Option<String>,
    /// The portrait this provider supplied, in the artwork store. Cleared by
    /// the database when the sweeper takes the artwork row.
    pub image_hash: Option<String>,
    /// The banner this provider supplied (RFC-014), cleared the same way.
    pub fanart_hash: Option<String>,
    pub fetched_at: i64,
    pub expires_at: i64,
}

/// A portrait already in the artwork store, waiting for its row.
#[derive(Debug)]
pub(crate) struct StoredPortrait {
    pub(crate) hash: String,
    pub(crate) format: &'static str,
    pub(crate) byte_size: i64,
}

/// A Deezer artist offered as a portrait (RFC-013 decision 4), its preview
/// already in the artwork store. The row naming the preview is the caller's to
/// write, with whatever names it.
#[derive(Debug)]
pub(crate) struct DeezerCandidate {
    pub(crate) deezer_id: i64,
    pub(crate) name: String,
    pub(crate) fans: Option<i64>,
    pub(crate) preview: StoredPortrait,
}

/// Why a search for candidates gave none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateSearchError {
    /// Deezer is not among the operator's providers, or images are off.
    NotConfigured,
    /// Deezer, or a picture host it named, could not be reached, or a preview
    /// could not be stored.
    Unreached,
}

/// One neighbour in a provider's cached list.
#[derive(Debug, Clone)]
pub struct CachedSimilar {
    pub name: String,
    pub name_canonical: String,
    pub score: f64,
}

/// What one fill brought back from one provider, before it is written.
#[derive(Debug, Default)]
pub(crate) struct ArtistAnswer {
    pub(crate) bio_summary: Option<String>,
    pub(crate) bio_full: Option<String>,
    pub(crate) similar: Vec<CachedSimilar>,
    pub(crate) image: Option<StoredPortrait>,
    /// The banner, from TheAudioDB's fanart (RFC-014).
    pub(crate) fanart: Option<StoredPortrait>,
}

impl ArtistAnswer {
    /// A hit carries something; an answer with nothing in it is a miss.
    pub(crate) fn outcome(&self) -> Outcome {
        if self.bio_summary.is_some()
            || self.bio_full.is_some()
            || !self.similar.is_empty()
            || self.image.is_some()
            || self.fanart.is_some()
        {
            Outcome::Hit
        } else {
            Outcome::Miss
        }
    }
}

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

struct Engine {
    db: Database,
    /// In the operator's order, which is the order fields are merged in.
    sources: Vec<Source>,
    ttl_ms: i64,
    miss_ttl_ms: i64,
    request_timeout: Duration,
    /// Where fetched portraits are stored: the artwork store.
    artwork_dir: PathBuf,
    image_addresses: AddressPolicy,
    /// The fills in flight, by provider and canonical name. A visit that finds
    /// its key here schedules nothing: the fill already running answers it.
    in_flight: DashMap<(Provider, String), ()>,
    /// How many fills are waiting or running: the count [`MAX_IN_FLIGHT`]
    /// bounds, and the one [`Enrichment::idle`] waits on.
    running: tokio::sync::watch::Sender<usize>,
}

struct Source {
    provider: Provider,
    client: Client,
    pace: Pace,
}

enum Client {
    LastFm {
        client: LastfmClient,
        key: String,
    },
    Deezer {
        client: DeezerClient,
        image_hosts: Vec<String>,
    },
    TheAudioDb {
        client: TheAudioDbClient,
        image_hosts: Vec<String>,
    },
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
            let http = crate::outbound::client(settings.request_timeout)
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
        let now = crate::authentication::now_ms();
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

impl Engine {
    /// Starts a fill for one provider and one name, unless one is running or
    /// [`MAX_IN_FLIGHT`] are.
    fn schedule(self: Arc<Self>, index: usize, name: &str, canonical: &str) {
        let key = (self.sources[index].provider, canonical.to_owned());
        // A place is reserved before the key is claimed, in one step under the
        // counter's lock: a length read then an insert would let two visits on
        // two threads both see one place left and both take it.
        let reserved = self.running.send_if_modified(|count| {
            let free = *count < MAX_IN_FLIGHT;
            if free {
                *count += 1;
            }
            free
        });
        if !reserved {
            return;
        }
        if self.in_flight.insert(key.clone(), ()).is_some() {
            self.running.send_modify(|count| *count -= 1);
            return;
        }
        // Released however the fill ends — a panic, or a task dropped before
        // its first poll included, which is why it is built here rather than
        // inside the task. A key left behind would stop that name from ever
        // being asked again.
        let flight = Flight { engine: self, key };
        let name = name.to_owned();
        tokio::spawn(async move {
            flight.engine.fill(index, &name, &flight.key.1).await;
            drop(flight);
        });
    }

    async fn fill(&self, index: usize, name: &str, canonical: &str) {
        let source = &self.sources[index];
        let answer = match &source.client {
            Client::LastFm { client, key } => self.ask_lastfm(source, client, key, name).await,
            Client::Deezer {
                client,
                image_hosts,
            } => self.ask_deezer(source, client, image_hosts, name).await,
            Client::TheAudioDb {
                client,
                image_hosts,
            } => self.ask_theaudiodb(source, client, image_hosts, name).await,
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err(verdict) => {
                tracing::warn!(
                    provider = source.provider.as_str(),
                    verdict = %verdict,
                    "artist lookup unreached; nothing cached"
                );
                return;
            }
        };
        let fetched_at = crate::authentication::now_ms();
        let outcome = answer.outcome();
        let ttl = match outcome {
            Outcome::Hit => self.ttl_ms,
            Outcome::Miss => self.miss_ttl_ms,
        };
        if let Err(error) = self
            .db
            .write_metadata_artist(
                source.provider,
                canonical,
                &answer,
                fetched_at,
                fetched_at.saturating_add(ttl),
            )
            .await
        {
            tracing::warn!(
                provider = source.provider.as_str(),
                error = %error,
                "artist lookup answered but could not be cached"
            );
            return;
        }
        tracing::debug!(
            provider = source.provider.as_str(),
            outcome = outcome.as_str(),
            "artist lookup cached"
        );
    }

    /// `artist.getInfo` and `artist.getSimilar`. Both must be reached for the
    /// answer to count: half an answer written as a whole one would cache the
    /// missing half as "nothing" for the TTL.
    async fn ask_lastfm(
        &self,
        source: &Source,
        client: &LastfmClient,
        key: &str,
        name: &str,
    ) -> Result<ArtistAnswer, Verdict> {
        source.pace.wait().await;
        let info = self.bounded(client.artist_get_info(name, key)).await?;
        source.pace.wait().await;
        let similar = self
            .bounded(client.artist_get_similar(name, key, SIMILAR_LIMIT))
            .await?;
        let own = waveflow_core::scanner::canonical_name(name);
        let similar = similar
            .into_iter()
            .filter_map(|neighbour| {
                let name_canonical = waveflow_core::scanner::canonical_name(&neighbour.name);
                (!name_canonical.is_empty() && name_canonical != own).then(|| CachedSimilar {
                    name: neighbour.name,
                    name_canonical,
                    score: f64::from(neighbour.match_score),
                })
            })
            .collect();
        let (bio_summary, bio_full) = info
            .map(|info| (info.bio_summary, info.bio_full))
            .unwrap_or_default();
        Ok(ArtistAnswer {
            bio_summary,
            bio_full,
            similar,
            image: None,
            fanart: None,
        })
    }

    /// The artist search, then the picture it names — decision 8.
    ///
    /// No artist by that name, or one without a picture, is a miss. So is a
    /// picture the fetcher refuses for what it is — off the allowlist, at a
    /// refused address, not an image, too large — since asking again would
    /// get the same answer. A picture host that could not be reached is
    /// unreached, like the search, and writes nothing.
    async fn ask_deezer(
        &self,
        source: &Source,
        client: &DeezerClient,
        image_hosts: &[String],
        name: &str,
    ) -> Result<ArtistAnswer, Verdict> {
        source.pace.wait().await;
        let hits = self.bounded(client.search_artist(name)).await?;
        let picture = select_by_name(hits, &normalize_name(name), |hit| Some(hit.name.as_str()))
            .and_then(|hit| hit.best_picture());
        let Some(picture) = picture else {
            return Ok(ArtistAnswer::default());
        };
        let fetched = portrait::fetch(
            &picture,
            image_hosts,
            self.image_addresses,
            self.request_timeout,
        )
        .await;
        let portrait = match fetched {
            Ok(portrait) => portrait,
            Err(refusal) if refusal.is_answer() => {
                tracing::warn!(
                    provider = source.provider.as_str(),
                    verdict = %refusal,
                    "artist portrait refused; cached as a miss"
                );
                return Ok(ArtistAnswer::default());
            }
            Err(refusal) => return Err(Verdict::Image(refusal)),
        };
        portrait::store(&self.artwork_dir, &portrait)
            .await
            .map_err(|_| Verdict::Store)?;
        Ok(ArtistAnswer {
            image: Some(StoredPortrait {
                hash: portrait.hash,
                format: portrait.format,
                byte_size: portrait.bytes.len() as i64,
            }),
            ..ArtistAnswer::default()
        })
    }

    /// The Deezer source, its client and the hosts its pictures come from.
    fn deezer(&self) -> Option<(&Source, &DeezerClient, &[String])> {
        self.sources.iter().find_map(|source| match &source.client {
            Client::Deezer {
                client,
                image_hosts,
            } => Some((source, client, image_hosts.as_slice())),
            Client::LastFm { .. } | Client::TheAudioDb { .. } => None,
        })
    }

    /// The artist search, then the first fanart that is really one (RFC-014
    /// decision 2).
    ///
    /// The fanarts are tried in TheAudioDB's order: one the fetcher refuses
    /// for what it is gives way to the next, and the first stored ends the
    /// search. None accepted, or no artist by that name, is a miss. A picture
    /// host that could not be reached is unreached, like the search, and
    /// writes nothing: it says nothing about the artist.
    async fn ask_theaudiodb(
        &self,
        source: &Source,
        client: &TheAudioDbClient,
        image_hosts: &[String],
        name: &str,
    ) -> Result<ArtistAnswer, Verdict> {
        source.pace.wait().await;
        // English is asked only because the call takes a language: the
        // biography it selects is not kept.
        let Some(artist) = self.bounded(client.artist_info(name, "en")).await? else {
            return Ok(ArtistAnswer::default());
        };
        for fanart in &artist.fanart_urls {
            source.pace.wait().await;
            let fetched = portrait::fetch(
                fanart,
                image_hosts,
                self.image_addresses,
                self.request_timeout,
            )
            .await;
            let picture = match fetched {
                Ok(picture) => picture,
                Err(refusal) if refusal.is_answer() => {
                    tracing::debug!(
                        provider = source.provider.as_str(),
                        verdict = %refusal,
                        "artist fanart refused; trying the next"
                    );
                    continue;
                }
                Err(refusal) => return Err(Verdict::Image(refusal)),
            };
            portrait::store(&self.artwork_dir, &picture)
                .await
                .map_err(|_| Verdict::Store)?;
            return Ok(ArtistAnswer {
                fanart: Some(StoredPortrait {
                    hash: picture.hash,
                    format: picture.format,
                    byte_size: picture.bytes.len() as i64,
                }),
                ..ArtistAnswer::default()
            });
        }
        Ok(ArtistAnswer::default())
    }

    /// One request, under the request timeout; anything but an answer is a
    /// verdict.
    async fn bounded<T, E>(
        &self,
        request: impl std::future::Future<Output = Result<T, E>>,
    ) -> Result<T, Verdict>
    where
        Verdict: From<E>,
    {
        match tokio::time::timeout(self.request_timeout, request).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(error)) => Err(Verdict::from(error)),
            Err(_) => Err(Verdict::Timeout),
        }
    }
}

/// Releases a fill's key and count when the fill ends.
struct Flight {
    engine: Arc<Engine>,
    key: (Provider, String),
}

impl Drop for Flight {
    fn drop(&mut self) {
        self.engine.in_flight.remove(&self.key);
        self.engine.running.send_modify(|count| *count -= 1);
    }
}

/// Why a lookup was unreached, in words that carry nothing from the request or
/// the response — no URL, no body, no key.
#[derive(Debug)]
enum Verdict {
    Timeout,
    Transport,
    Refused {
        code: i64,
    },
    Status {
        status: u16,
    },
    TooLarge,
    Undecodable,
    RateLimited,
    /// The picture host, not the provider, was not reached.
    Image(Refusal),
    /// The picture came back and could not be written to the artwork store.
    Store,
}

impl From<LookupError> for Verdict {
    fn from(error: LookupError) -> Self {
        match error {
            LookupError::Http(error) if error.is_timeout() => Self::Timeout,
            LookupError::Http(_) => Self::Transport,
            LookupError::Refused { code } => Self::Refused { code },
            LookupError::Status { status } => Self::Status { status },
            LookupError::TooLarge { .. } => Self::TooLarge,
            LookupError::Decode(_) => Self::Undecodable,
        }
    }
}

impl From<DeezerError> for Verdict {
    fn from(error: DeezerError) -> Self {
        match error {
            DeezerError::Http(error) if error.is_timeout() => Self::Timeout,
            DeezerError::Http(error) => {
                error
                    .status()
                    .map_or(Self::Transport, |status| Self::Status {
                        status: status.as_u16(),
                    })
            }
            DeezerError::Api(error) => Self::Refused {
                code: error.code.unwrap_or_default(),
            },
            DeezerError::RateLimited => Self::RateLimited,
            DeezerError::TooLarge { .. } => Self::TooLarge,
            DeezerError::Decode(_) => Self::Undecodable,
        }
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => formatter.write_str("timed out"),
            Self::Transport => formatter.write_str("transport failure"),
            Self::Refused { code } => write!(formatter, "refused with code {code}"),
            Self::Status { status } => write!(formatter, "answered HTTP {status}"),
            Self::TooLarge => formatter.write_str("answer over the size ceiling"),
            Self::Undecodable => formatter.write_str("answer not understood"),
            Self::RateLimited => formatter.write_str("rate-limited"),
            Self::Image(refusal) => refusal.fmt(formatter),
            Self::Store => formatter.write_str("portrait could not be stored"),
        }
    }
}

/// Spaces one provider's requests to its rate: a token bucket that holds one
/// token, so there is never a burst to spend.
///
/// The reservation is taken under the lock and the wait happens outside it, so
/// callers queue in order without one sleeping caller holding up the rest.
struct Pace {
    interval: Duration,
    next: std::sync::Mutex<Instant>,
}

impl Pace {
    fn new(rate_per_sec: u32) -> Self {
        Self {
            interval: Duration::from_secs(1) / rate_per_sec.max(1),
            next: std::sync::Mutex::new(Instant::now()),
        }
    }

    /// The operator's rate, but never faster than one request per `floor`:
    /// a provider's own ceiling, which the shared setting cannot raise.
    fn with_floor(rate_per_sec: u32, floor: Duration) -> Self {
        let pace = Self::new(rate_per_sec);
        Self {
            interval: pace.interval.max(floor),
            ..pace
        }
    }

    async fn wait(&self) {
        let slot = {
            let mut next = self
                .next
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let slot = (*next).max(Instant::now());
            *next = slot + self.interval;
            slot
        };
        tokio::time::sleep_until(slot.into()).await;
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
