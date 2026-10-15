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
    deezer::{DeezerClient, DeezerError},
    lastfm::LastfmClient,
    name_match::{normalize_name, select_by_name},
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
}

impl Provider {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "lastfm" => Some(Self::LastFm),
            "deezer" => Some(Self::Deezer),
            _ => None,
        }
    }

    /// The name in the setting, in the cache's `provider` column, and in logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LastFm => "lastfm",
            Self::Deezer => "deezer",
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
}

impl ArtistAnswer {
    /// A hit carries something; an answer with nothing in it is a miss.
    pub(crate) fn outcome(&self) -> Outcome {
        if self.bio_summary.is_some()
            || self.bio_full.is_some()
            || !self.similar.is_empty()
            || self.image.is_some()
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
            };
            sources.push(Source {
                provider,
                client,
                pace: Pace::new(settings.rate_per_sec),
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
        // since. Linked again from the cache, without a second download.
        if let Some(hash) = ordered.iter().find_map(|row| row.image_hash.as_deref()) {
            engine.db.link_artist_portrait(&canonical, hash).await?;
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

fn duration_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
