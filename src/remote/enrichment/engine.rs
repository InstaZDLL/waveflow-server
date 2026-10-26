//! The engine behind the handle: one fill per provider and name, paced and
//! bounded, and the verdict an unreached lookup is logged with.

use std::{path::PathBuf, sync::Arc, time::Duration};

use dashmap::DashMap;
use waveflow_core::metadata::{
    deezer::{DeezerClient, DeezerError},
    lastfm::LastfmClient,
    name_match::{normalize_name, select_by_name},
    theaudiodb::TheAudioDbClient,
    LookupError,
};

use crate::{
    remote::portrait::{self, AddressPolicy, Refusal},
    storage::database::Database,
};

use super::{ArtistAnswer, CachedSimilar, Outcome, Pace, Provider, StoredPortrait, MAX_IN_FLIGHT};

/// How many neighbours are asked for and kept. Subsonic's `getArtistInfo`
/// defaults to twenty; asking for more leaves room for the ones a caller cannot
/// see, which decision 7 drops at read time.
const SIMILAR_LIMIT: u32 = 50;

pub(super) struct Engine {
    pub(super) db: Database,
    /// In the operator's order, which is the order fields are merged in.
    pub(super) sources: Vec<Source>,
    pub(super) ttl_ms: i64,
    pub(super) miss_ttl_ms: i64,
    pub(super) request_timeout: Duration,
    /// Where fetched portraits are stored: the artwork store.
    pub(super) artwork_dir: PathBuf,
    pub(super) image_addresses: AddressPolicy,
    /// The fills in flight, by provider and canonical name. A visit that finds
    /// its key here schedules nothing: the fill already running answers it.
    pub(super) in_flight: DashMap<(Provider, String), ()>,
    /// How many fills are waiting or running: the count [`MAX_IN_FLIGHT`]
    /// bounds, and the one [`Enrichment::idle`](super::Enrichment::idle) waits on.
    pub(super) running: tokio::sync::watch::Sender<usize>,
}

pub(super) struct Source {
    pub(super) provider: Provider,
    pub(super) client: Client,
    pub(super) pace: Pace,
}

pub(super) enum Client {
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

impl Engine {
    /// Starts a fill for one provider and one name, unless one is running or
    /// [`MAX_IN_FLIGHT`] are.
    pub(super) fn schedule(self: Arc<Self>, index: usize, name: &str, canonical: &str) {
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
        let fetched_at = crate::auth::authentication::now_ms();
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
    pub(super) fn deezer(&self) -> Option<(&Source, &DeezerClient, &[String])> {
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
    pub(super) async fn bounded<T, E>(
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
pub(super) enum Verdict {
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
