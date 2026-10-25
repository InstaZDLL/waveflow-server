//! Lyrics looked up on LRCLIB — RFC-024 decisions 1 to 4.
//!
//! **Off unless an operator names `lrclib`** in `WAVEFLOW_LYRICS_PROVIDERS`;
//! disabled, every call returns at once having read and sent nothing.
//!
//! **Asked only on demand**, by a reader that found no lyrics of the track's
//! own (decision 2), and **waited for briefly** (decision 3): the first ask
//! schedules one request and waits at most [`ANSWER_WAIT`] for it. A request
//! that takes longer still lands in the cache for the next listen.
//!
//! **Three outcomes**, as in RFC-011 decision 5: a *hit* stands for the long
//! TTL, a *miss* — a 404, an instrumental, a match on another duration — for
//! the short one (an instrumental for the long one: it is an answer), and
//! *unreached* writes nothing, so the next ask tries again.
//!
//! **Paced, bounded and single-flight.** Requests keep the configured rate,
//! at most [`MAX_IN_FLIGHT`] wait or run, and concurrent asks for one lookup
//! key wait on one request. The writer gate is taken for the row write only,
//! after the request is over.
//!
//! **Nothing about a request is logged but a verdict.** The request URL holds
//! the track's title and artist in its query string; it is never written to a
//! log, and neither is a response body.

use std::{sync::Arc, time::Duration};

use dashmap::DashMap;
use serde::Deserialize;

use crate::{config::LyricsSettings, database::Database, enrichment::Pace};

/// LRCLIB's root. A constant, not a setting: there is no operator field, so
/// there is no URL to validate.
const LRCLIB_BASE_URL: &str = "https://lrclib.net";

/// What LRCLIB asks a client to send: a name, a version and a link.
const USER_AGENT: &str = concat!(
    "WaveFlowServer/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/InstaZDLL/waveflow-server)"
);

/// The most an answer may weigh before it is refused unread: the cap lyrics
/// uploads and sidecars already have.
pub const MAX_ANSWER_BYTES: usize = 1024 * 1024;

/// How long the reader that scheduled a lookup waits for it (decision 3).
pub const ANSWER_WAIT: Duration = Duration::from_secs(4);

/// How far LRCLIB's own duration may be from the track's, in seconds, before
/// its answer is taken as another recording's. LRCLIB matches with the same
/// tolerance.
const DURATION_TOLERANCE_SECS: f64 = 2.0;

/// How many lookups may be waiting or running at once. Beyond it nothing is
/// scheduled; the next ask tries again, as after an unreached lookup.
pub const MAX_IN_FLIGHT: usize = 64;

/// A provider an operator can name in `WAVEFLOW_LYRICS_PROVIDERS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LyricsProvider {
    Lrclib,
}

impl LyricsProvider {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "lrclib" => Some(Self::Lrclib),
            _ => None,
        }
    }

    /// The name in the setting, in the cache's `provider` column, in the
    /// native `source` of a sheet, and in logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lrclib => "lrclib",
        }
    }
}

/// What a lookup said, when it said something. Unreached has no variant: it
/// is the absence of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsOutcome {
    Hit,
    Miss,
    Instrumental,
}

impl LyricsOutcome {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hit" => Some(Self::Hit),
            "miss" => Some(Self::Miss),
            "instrumental" => Some(Self::Instrumental),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Instrumental => "instrumental",
        }
    }
}

/// One cached row.
#[derive(Debug, Clone)]
pub struct CachedLyrics {
    pub outcome: LyricsOutcome,
    pub synced: Option<String>,
    pub plain: Option<String>,
    pub fetched_at: i64,
    pub expires_at: i64,
}

/// What a track is looked up by (decision 2).
#[derive(Debug, Clone)]
pub struct TrackLookup {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub duration_ms: i64,
}

impl TrackLookup {
    /// The duration LRCLIB is sent, rounded to the second.
    fn duration_secs(&self) -> i64 {
        (self.duration_ms.max(0) + 500) / 1000
    }

    /// The cache key: a digest of what is sent, folded the way the catalogue
    /// folds names, so two spellings of one request share one row. `None`
    /// when there is nothing to match on — no artist, no title, no duration.
    pub fn key(&self) -> Option<String> {
        let artist = waveflow_core::scanner::canonical_name(&self.artist);
        let title = waveflow_core::scanner::canonical_name(&self.title);
        if artist.is_empty() || title.is_empty() || self.duration_secs() <= 0 {
            return None;
        }
        let album = self
            .album
            .as_deref()
            .map(waveflow_core::scanner::canonical_name)
            .unwrap_or_default();
        let mut hasher = blake3::Hasher::new();
        for part in [artist.as_str(), title.as_str(), album.as_str()] {
            hasher.update(part.as_bytes());
            hasher.update(&[0]);
        }
        hasher.update(self.duration_secs().to_string().as_bytes());
        Some(hasher.finalize().to_hex().to_string())
    }
}

/// Lyrics a lookup found: the LRC text when LRCLIB has timings, the plain
/// text otherwise or as well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundLyrics {
    pub synced: Option<String>,
    pub plain: Option<String>,
}

impl CachedLyrics {
    fn found(&self) -> Option<FoundLyrics> {
        (self.outcome == LyricsOutcome::Hit).then(|| FoundLyrics {
            synced: self.synced.clone(),
            plain: self.plain.clone(),
        })
    }
}

/// The lookup, or its absence. Cheap to clone; every clone drives the same
/// engine.
#[derive(Clone)]
pub struct LyricsLookup {
    engine: Option<Arc<Engine>>,
}

struct Engine {
    db: Database,
    provider: LyricsProvider,
    http: reqwest::Client,
    base_url: String,
    ttl_ms: i64,
    miss_ttl_ms: i64,
    request_timeout: Duration,
    answer_wait: Duration,
    pace: Pace,
    /// The lookups in flight, by key, each with the signal its end raises.
    in_flight: DashMap<String, tokio::sync::watch::Receiver<bool>>,
    /// How many lookups are waiting or running: the count [`MAX_IN_FLIGHT`]
    /// bounds, and the one [`LyricsLookup::idle`] waits on.
    running: tokio::sync::watch::Sender<usize>,
}

impl LyricsLookup {
    /// Lookups switched off.
    pub fn disabled() -> Self {
        Self { engine: None }
    }

    /// The lookup `settings` describe, against LRCLIB's real address.
    pub fn new(db: Database, settings: &LyricsSettings) -> anyhow::Result<Self> {
        Self::with_base_url(db, settings, LRCLIB_BASE_URL.to_owned())
    }

    /// [`Self::new`], with LRCLIB reached at `base_url`: the test suite's own
    /// listener, which is the only reason this exists.
    pub fn with_base_url(
        db: Database,
        settings: &LyricsSettings,
        base_url: String,
    ) -> anyhow::Result<Self> {
        let Some(&provider) = settings.providers.first() else {
            return Ok(Self::disabled());
        };
        let http = crate::outbound::builder(settings.request_timeout)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|_| anyhow::anyhow!("the outbound client could not be built"))?;
        let ttl_ms = duration_ms(settings.ttl);
        let miss_ttl_ms = duration_ms(settings.miss_ttl);
        Ok(Self {
            engine: Some(Arc::new(Engine {
                db,
                provider,
                http,
                base_url: base_url.trim_end_matches('/').to_owned(),
                ttl_ms,
                miss_ttl_ms,
                request_timeout: settings.request_timeout,
                answer_wait: ANSWER_WAIT,
                pace: Pace::new(settings.rate_per_sec),
                in_flight: DashMap::new(),
                running: tokio::sync::watch::channel(0).0,
            })),
        })
    }

    /// The same lookup, waiting `wait` on the answering path instead of
    /// [`ANSWER_WAIT`]. For the test suite, which proves the bound without
    /// spending four seconds on it; it has no effect once the lookup has been
    /// cloned.
    #[doc(hidden)]
    pub fn with_answer_wait(mut self, wait: Duration) -> Self {
        if let Some(engine) = self.engine.as_mut().and_then(Arc::get_mut) {
            engine.answer_wait = wait;
        }
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.engine.is_some()
    }

    /// The provider sheets from this lookup are credited to.
    pub fn provider(&self) -> Option<LyricsProvider> {
        self.engine.as_ref().map(|engine| engine.provider)
    }

    /// The lyrics LRCLIB has for `track`, from the cache or from one request.
    ///
    /// A fresh row answers at once. A stale one answers at once too, and has
    /// its refresh scheduled (decision 4). No row schedules the lookup — or
    /// joins the one already in flight — and waits for it for at most
    /// [`ANSWER_WAIT`]; whatever the cache holds then is the answer.
    pub async fn lyrics(&self, track: &TrackLookup) -> Result<Option<FoundLyrics>, sqlx::Error> {
        let Some(engine) = &self.engine else {
            return Ok(None);
        };
        let Some(key) = track.key() else {
            return Ok(None);
        };
        let now = crate::authentication::now_ms();
        if let Some(row) = engine
            .db
            .lyrics_lookup_row(engine.provider.as_str(), &key)
            .await?
        {
            if row.expires_at <= now {
                let _ = Arc::clone(engine).schedule(track, &key);
            }
            return Ok(row.found());
        }
        if let Some(mut done) = Arc::clone(engine).schedule(track, &key) {
            // Bounded, and a lookup that outlives it goes on: it is cached for
            // the next listen. A sender dropped without signalling — a fill
            // that panicked — ends the wait the same way.
            let _ = tokio::time::timeout(engine.answer_wait, done.wait_for(|done| *done)).await;
        }
        Ok(engine
            .db
            .lyrics_lookup_row(engine.provider.as_str(), &key)
            .await?
            .and_then(|row| row.found()))
    }

    /// Waits until no lookup is running. For the test suite.
    #[doc(hidden)]
    pub async fn idle(&self) {
        let Some(engine) = &self.engine else {
            return;
        };
        let mut running = engine.running.subscribe();
        let _ = running.wait_for(|count| *count == 0).await;
    }
}

impl Engine {
    /// Starts the lookup for `key` unless one is running — whose end signal
    /// is then returned — or [`MAX_IN_FLIGHT`] are, which returns `None`.
    fn schedule(
        self: Arc<Self>,
        track: &TrackLookup,
        key: &str,
    ) -> Option<tokio::sync::watch::Receiver<bool>> {
        if let Some(running) = self.in_flight.get(key) {
            return Some(running.clone());
        }
        // A place is reserved before the key is claimed, in one step under
        // the counter's lock, as enrichment does.
        let reserved = self.running.send_if_modified(|count| {
            let free = *count < MAX_IN_FLIGHT;
            if free {
                *count += 1;
            }
            free
        });
        if !reserved {
            return None;
        }
        let (done, signal) = tokio::sync::watch::channel(false);
        match self.in_flight.entry(key.to_owned()) {
            dashmap::Entry::Occupied(running) => {
                let running = running.get().clone();
                self.running.send_modify(|count| *count -= 1);
                return Some(running);
            }
            dashmap::Entry::Vacant(slot) => {
                slot.insert(signal.clone());
            }
        }
        // Released however the lookup ends — a panic, or a task dropped
        // before its first poll included, which is why it is built here.
        let flight = Flight {
            engine: self,
            key: key.to_owned(),
            done,
        };
        let track = track.clone();
        tokio::spawn(async move {
            flight.engine.fill(&track, &flight.key).await;
            drop(flight);
        });
        Some(signal)
    }

    async fn fill(&self, track: &TrackLookup, key: &str) {
        self.pace.wait().await;
        let answer = match tokio::time::timeout(self.request_timeout, self.ask(track)).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(verdict)) => {
                tracing::warn!(
                    provider = self.provider.as_str(),
                    verdict = %verdict,
                    "lyrics lookup unreached; nothing cached"
                );
                return;
            }
            Err(_) => {
                tracing::warn!(
                    provider = self.provider.as_str(),
                    verdict = %Verdict::Timeout,
                    "lyrics lookup unreached; nothing cached"
                );
                return;
            }
        };
        let fetched_at = crate::authentication::now_ms();
        let ttl = match answer.outcome {
            LyricsOutcome::Hit | LyricsOutcome::Instrumental => self.ttl_ms,
            LyricsOutcome::Miss => self.miss_ttl_ms,
        };
        if let Err(error) = self
            .db
            .write_lyrics_lookup(
                self.provider.as_str(),
                key,
                &CachedLyrics {
                    outcome: answer.outcome,
                    synced: answer.synced,
                    plain: answer.plain,
                    fetched_at,
                    expires_at: fetched_at.saturating_add(ttl),
                },
            )
            .await
        {
            tracing::warn!(
                provider = self.provider.as_str(),
                error = %error,
                "lyrics lookup answered but could not be cached"
            );
            return;
        }
        tracing::debug!(
            provider = self.provider.as_str(),
            outcome = answer.outcome.as_str(),
            "lyrics lookup cached"
        );
    }

    /// `GET /api/get`, read and judged.
    async fn ask(&self, track: &TrackLookup) -> Result<Answer, Verdict> {
        let mut query = vec![
            ("artist_name", track.artist.clone()),
            ("track_name", track.title.clone()),
            ("duration", track.duration_secs().to_string()),
        ];
        if let Some(album) = track
            .album
            .as_deref()
            .filter(|album| !album.trim().is_empty())
        {
            query.push(("album_name", album.to_owned()));
        }
        let response = self
            .http
            .get(format!("{}/api/get", self.base_url))
            .query(&query)
            .send()
            .await
            .map_err(Verdict::from_transport)?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(Answer::miss());
        }
        if status != reqwest::StatusCode::OK {
            return Err(Verdict::Status {
                status: status.as_u16(),
            });
        }
        let body = read_bounded(response, MAX_ANSWER_BYTES).await?;
        let record: Record = serde_json::from_slice(&body).map_err(|_| Verdict::Undecodable)?;
        Ok(judge(record, track.duration_ms))
    }
}

/// What a request brought back, before it is written.
struct Answer {
    outcome: LyricsOutcome,
    synced: Option<String>,
    plain: Option<String>,
}

impl Answer {
    fn miss() -> Self {
        Self {
            outcome: LyricsOutcome::Miss,
            synced: None,
            plain: None,
        }
    }
}

/// The part of LRCLIB's record this server reads.
#[derive(Debug, Deserialize)]
struct Record {
    #[serde(default)]
    instrumental: Option<bool>,
    #[serde(default, rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(default, rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

/// What a record means for a track of `duration_ms` (decision 2).
///
/// The duration is checked first, whatever the record says: an instrumental
/// of another recording is no more this track's answer than its words are,
/// and taken as one it would stand for the long TTL.
fn judge(record: Record, duration_ms: i64) -> Answer {
    if let Some(duration) = record.duration {
        let ours = duration_ms as f64 / 1000.0;
        if !duration.is_finite() || (duration - ours).abs() > DURATION_TOLERANCE_SECS {
            return Answer::miss();
        }
    }
    if record.instrumental == Some(true) {
        return Answer {
            outcome: LyricsOutcome::Instrumental,
            synced: None,
            plain: None,
        };
    }
    let text = |value: Option<String>| value.filter(|text| !text.trim().is_empty());
    let synced = text(record.synced_lyrics);
    let plain = text(record.plain_lyrics);
    if synced.is_none() && plain.is_none() {
        return Answer::miss();
    }
    Answer {
        outcome: LyricsOutcome::Hit,
        synced,
        plain,
    }
}

/// A body read whole, unless it outgrows `limit`. `Content-Length` is checked
/// first, and the running count covers an answer that announces nothing.
async fn read_bounded(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, Verdict> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(Verdict::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(Verdict::from_transport)? {
        if body.len() + chunk.len() > limit {
            return Err(Verdict::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Releases a lookup's key and count when it ends, and tells its waiters.
struct Flight {
    engine: Arc<Engine>,
    key: String,
    done: tokio::sync::watch::Sender<bool>,
}

impl Drop for Flight {
    fn drop(&mut self) {
        self.engine.in_flight.remove(&self.key);
        let _ = self.done.send(true);
        self.engine.running.send_modify(|count| *count -= 1);
    }
}

/// Why a lookup was unreached, in words that carry nothing from the request
/// or the response — no URL, no title, no body.
#[derive(Debug)]
enum Verdict {
    Timeout,
    Transport,
    Status { status: u16 },
    TooLarge,
    Undecodable,
}

impl Verdict {
    /// The error is dropped, not displayed: `reqwest` prints the request URL,
    /// which holds the track's title and artist.
    fn from_transport(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else {
            Self::Transport
        }
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => formatter.write_str("timed out"),
            Self::Transport => formatter.write_str("transport failure"),
            Self::Status { status } => write!(formatter, "answered HTTP {status}"),
            Self::TooLarge => formatter.write_str("answer over the size ceiling"),
            Self::Undecodable => formatter.write_str("answer not understood"),
        }
    }
}

fn duration_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(json: &str) -> Record {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_record_is_judged_by_what_it_carries() {
        let hit = judge(
            record(r#"{"duration":234,"plainLyrics":"a","syncedLyrics":"[00:01.00]a"}"#),
            234_400,
        );
        assert_eq!(hit.outcome, LyricsOutcome::Hit);
        assert_eq!(hit.synced.as_deref(), Some("[00:01.00]a"));
        let instrumental = judge(record(r#"{"instrumental":true,"plainLyrics":"x"}"#), 1_000);
        assert_eq!(instrumental.outcome, LyricsOutcome::Instrumental);
        assert!(instrumental.plain.is_none());
        let blank = judge(record(r#"{"plainLyrics":"  ","syncedLyrics":null}"#), 1_000);
        assert_eq!(blank.outcome, LyricsOutcome::Miss);
    }

    #[test]
    fn another_duration_is_another_recording() {
        // Two seconds either way is LRCLIB's own tolerance; three is not.
        let close = judge(record(r#"{"duration":236,"plainLyrics":"a"}"#), 234_000);
        assert_eq!(close.outcome, LyricsOutcome::Hit);
        let far = judge(record(r#"{"duration":237.5,"plainLyrics":"a"}"#), 234_000);
        assert_eq!(far.outcome, LyricsOutcome::Miss);
        // An instrumental is held to the same tolerance: another recording's
        // silence is not this track's answer, and would stand for 30 days.
        let other = judge(record(r#"{"duration":300,"instrumental":true}"#), 234_000);
        assert_eq!(other.outcome, LyricsOutcome::Miss);
        let ours = judge(record(r#"{"duration":235,"instrumental":true}"#), 234_000);
        assert_eq!(ours.outcome, LyricsOutcome::Instrumental);
    }

    #[test]
    fn the_key_folds_spelling_and_refuses_what_cannot_match() {
        let track = |artist: &str, title: &str, duration_ms| TrackLookup {
            artist: artist.into(),
            title: title.into(),
            album: Some("Post".into()),
            duration_ms,
        };
        assert_eq!(
            track("Björk", "Army of Me", 234_000).key(),
            track("BJÖRK", "army of me", 234_200).key()
        );
        assert_ne!(
            track("Björk", "Army of Me", 234_000).key(),
            track("Björk", "Army of Me", 236_000).key()
        );
        assert!(track("", "Army of Me", 234_000).key().is_none());
        assert!(track("Björk", "Army of Me", 0).key().is_none());
    }

    #[test]
    fn the_user_agent_names_the_project() {
        assert!(USER_AGENT.starts_with("WaveFlowServer/"));
        assert!(USER_AGENT.ends_with("(https://github.com/InstaZDLL/waveflow-server)"));
    }
}
