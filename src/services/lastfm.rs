//! The quarter of an hour in which somebody authorises this server at Last.fm.
//!
//! RFC-010 decision 11. The whole of decision 9's surface is one gesture —
//! present a secret already in hand to
//! `PUT /api/v2/scrobble-links/{provider}/{destination}` — and Last.fm hands
//! out no such secret. It takes a round trip through the person's browser: a
//! request token, an authorisation at their site, then the exchange of that
//! token for a session key, which lasts until it is revoked.
//!
//! **The server carries the journey.** The other road — "get a session key with
//! some third-party tool and paste it" — was tempting because it costs nothing,
//! which is also what disqualifies it: it lifts one destination out of the
//! model the other two live in, and makes a person pay for a provider's
//! peculiarity. It stays good for a diagnosis, not as the ordinary path, and
//! `PUT` still accepts a pasted key for exactly that.

use futures_util::future::BoxFuture;

use super::*;

/// How long a journey lasts.
///
/// Well inside the sixty minutes Last.fm grants its request token: we refuse
/// first, and an expired token never surprises us.
const JOURNEY_TTL: std::time::Duration = std::time::Duration::from_secs(12 * 60);

/// The cookie that pairs a return to the browser that opened the journey.
///
/// A fixed name, so opening a second journey replaces the first: the tab left
/// behind can no longer conclude, and its state expires alone. That is the
/// limit taken rather than corrected — linking an account to a service is done
/// once — and a cookie per state would trade that small surprise for machinery
/// nobody needs.
pub const LASTFM_JOURNEY_COOKIE: &str = "waveflow_lastfm_journey";

/// The path the cookie is scoped to, and the prefix the traces redact.
pub const LASTFM_CALLBACK_PREFIX: &str = "/api/v2/scrobble-links/lastfm/callback/";

/// Where a person goes to authorise.
///
/// A constant, and not derived from the declared destination: that destination
/// is the **API endpoint** a session key will be used against, while this is
/// Last.fm's *website*. They are two different hosts, and deriving one from the
/// other would be a guess this server has no business making.
/// `pub(crate)` since 2026-09-15: the browser-less journey builds this address
/// in the adapter, which is the only place holding the application key.
pub(crate) const LASTFM_AUTHORIZE_URL: &str = "https://www.last.fm/api/auth/";

/// What one journey needs that neither the account nor the destination
/// provides.
///
/// `Some` only when all of it is true at once: an application the operator
/// registered, and an `https` public URL to bring the person back to. Computed
/// once at construction so the three readings of that condition — the
/// destination listing, `authorize`, and the return — cannot disagree.
#[derive(Debug, Clone)]
pub(super) struct LastFmJourney {
    pub(super) api_key: String,
    /// `https://host/api/v2/scrobble-links/lastfm/callback/`, ready for a state
    /// to be appended.
    pub(super) callback_base: String,
}

impl LastFmJourney {
    pub(super) fn from_config(config: &crate::config::Config) -> Option<Self> {
        let application = config.lastfm.as_ref()?;
        // **`https` only, and decision 10's plaintext escape does not reach
        // here.** That escape is about the operator's own network; this journey
        // starts on the internet and crosses a person's browser carrying a
        // token worth a profile. `normalize_public_url` still accepts `http`,
        // deliberately — the public URL also serves shares — so the condition
        // belongs to switching Last.fm on, not to the setting.
        let public_url = config.public_url.as_deref()?;
        if !crate::api::public_url_is_https(Some(public_url)) {
            return None;
        }
        Some(Self {
            api_key: application.api_key.clone(),
            callback_base: format!(
                "{}{LASTFM_CALLBACK_PREFIX}",
                public_url.trim_end_matches('/')
            ),
        })
    }
}

/// Why Last.fm cannot be linked on this server.
///
/// Published beside the destination rather than discovered when somebody tries:
/// a link that fails later without explanation is the silent failure this whole
/// RFC spends itself preventing.
pub(super) fn lastfm_unavailability(
    config: &crate::config::Config,
) -> Option<super::ScrobbleUnavailable> {
    if config.lastfm.is_none() {
        return Some(super::ScrobbleUnavailable::NoApplicationConfigured);
    }
    if !crate::api::public_url_is_https(config.public_url.as_deref()) {
        // Naming the other way out, since 2026-09-15. This said last.fm
        // "needs" an https address, which stopped being true of last.fm the
        // day the command line could link one without any public address at
        // all — it is true of *this* journey. A reason an operator can act on
        // has to name the action that still exists, or it reads as a dead end
        // where there is a door.
        return Some(super::ScrobbleUnavailable::BrowserJourneyNeedsHttps);
    }
    None
}

/// Turns a request token into a session key.
///
/// Implemented in `src/scrobblers/lastfm.rs`, because that is the only place in
/// this server that makes an outbound request. Registered like a
/// [`ScrobbleTarget`] and for the same reason: a test drives the whole journey
/// against a double, and a server with no application registers neither.
pub trait LastFmSessionExchange: Send + Sync + 'static {
    fn exchange<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<String, ServiceError>>;

    /// Asks Last.fm for a request token, for the journey that has no browser
    /// on this machine.
    ///
    /// Both halves of Last.fm's protocol end at [`Self::exchange`], which is
    /// why they share a trait: what differs is only where the request token
    /// comes from. The web journey receives one on the return; here it is
    /// asked for first, and a person carries it back by hand.
    ///
    /// **The address comes back with it.** It carries the application key, and
    /// the service has no other way to reach one — `lastfm_journey` holds a
    /// copy but exists only when a public `https` address is configured, which
    /// is exactly the case this journey is for.
    fn request_token<'a>(&'a self) -> BoxFuture<'a, Result<LastFmApproval, ServiceError>>;
}

/// What an operator has to do by hand, and what they must bring back.
#[derive(Debug)]
pub struct LastFmApproval {
    /// Last.fm's own page, carrying the application key and the request token.
    /// **It is not a public address**: whoever opens it first is who the
    /// session ends up belonging to.
    pub authorize_url: String,
    /// The request token, which [`LastFmSessionExchange::exchange`] turns into
    /// a session key once the person has approved it.
    pub token: String,
}

/// The exchangers `initialize` fills, by destination name.
pub(super) type LastFmExchanges = Arc<dashmap::DashMap<String, Arc<dyn LastFmSessionExchange>>>;

/// What `authorize` hands back to its route.
#[derive(Debug)]
pub struct LastFmJourneyStart {
    /// Where to send the person.
    pub authorize_url: String,
    /// The cookie value the route must set. Never stored: only its digest is.
    pub cookie: String,
    /// How long both the state and the cookie last.
    pub expires_in: std::time::Duration,
}

impl DomainServices {
    /// Registers the exchanger for one declared Last.fm instance.
    pub fn register_lastfm_exchange(
        &self,
        destination: &str,
        exchange: Arc<dyn LastFmSessionExchange>,
    ) {
        self.lastfm_exchanges
            .insert(destination.to_owned(), exchange);
    }

    /// Opens a journey and says where to send the person.
    ///
    /// **The destination is named on the way out.** Last.fm has one instance
    /// today, so it would be tempting to leave it implicit — but decision 10
    /// has just refused every implicit default, and an exception for one
    /// recipient is the first step back onto the slide it forbids. The day
    /// somebody declares two — a household account and their own — nothing here
    /// has to change.
    pub async fn begin_lastfm_authorization(
        &self,
        user_id: Uuid,
        destination: &str,
    ) -> Result<LastFmJourneyStart, ServiceError> {
        let journey = self
            .lastfm_journey
            .as_ref()
            .ok_or(ServiceError::Unavailable)?;
        let fingerprint = self
            .scrobble_destinations
            .get(&(ScrobbleProvider::LastFm, destination.to_owned()))
            .ok_or(ServiceError::NotFound)?
            .clone();

        // Both drawn from the system CSPRNG, and both worth a profile: the
        // state is the only thing separating a legitimate return from a
        // fabricated one, and the cookie is the only thing separating *this*
        // browser's return from anybody who came by the URL afterwards.
        let state = crate::security::generate_token("");
        let cookie = crate::security::generate_token("");
        let now = now_ms();
        let expires_at =
            now.saturating_add(i64::try_from(JOURNEY_TTL.as_millis()).unwrap_or(i64::MAX));

        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        // One journey per account, held by the schema as well: opening a second
        // replaces the first rather than leaving a row nothing can finish.
        sqlx::query("DELETE FROM lastfm_authorization WHERE user_id=?")
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO lastfm_authorization (state, user_id, destination, \
             destination_fingerprint, cookie_hash, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&state)
        .bind(user_id.to_string())
        .bind(destination)
        .bind(&fingerprint)
        .bind(crate::security::token_hash(&cookie).as_slice())
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        // `cb` names `…/callback/{state}`, so the random travels in the path.
        // Last.fm documents that it appends `/?token=…` to the callback, which
        // says nothing about what it would do with one that already carried a
        // `?` — a path segment depends on no assumption about that
        // concatenation.
        let mut authorize_url =
            url::Url::parse(LASTFM_AUTHORIZE_URL).map_err(|_| ServiceError::Unavailable)?;
        authorize_url
            .query_pairs_mut()
            .append_pair("api_key", &journey.api_key)
            .append_pair("cb", &format!("{}{state}", journey.callback_base));
        Ok(LastFmJourneyStart {
            authorize_url: authorize_url.to_string(),
            cookie,
            expires_in: JOURNEY_TTL,
        })
    }

    /// Finishes a journey: checks the state, spends it, exchanges the token and
    /// creates the link.
    ///
    /// **The state is spent before the exchange, not after the link.** What has
    /// served once cannot serve again, even when the attempt fails further on.
    ///
    /// **And both halves are checked together.** The state alone would do if
    /// the return URL never left the browser that asked for it — but it travels
    /// through Last.fm, and a URL that travels lands in a referrer and in a
    /// history. Whoever obtained it could finish the journey with *their*
    /// token, and somebody else's account would start submitting to their
    /// profile.
    ///
    /// **And the return re-checks the destination.** A quarter of an hour
    /// separates the halves and a server can restart in between — which is why
    /// this row lives in a database at all. Destination gone or moved, the
    /// return is refused and nothing is created; otherwise the journey would
    /// manufacture a link to a machine the person never chose, already wrong at
    /// birth.
    ///
    /// Answers the destination that was linked, so the route can say where the
    /// person has arrived without reading the row again.
    pub async fn complete_lastfm_authorization(
        &self,
        state: &str,
        cookie: &str,
        token: &str,
    ) -> Result<String, ServiceError> {
        if self.lastfm_journey.is_none() {
            return Err(ServiceError::Unavailable);
        }
        let now = now_ms();
        let (user_id, destination, fingerprint) = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.pool().begin().await?;
            let row = sqlx::query(
                "SELECT user_id, destination, destination_fingerprint, cookie_hash, expires_at \
                 FROM lastfm_authorization WHERE state=?",
            )
            .bind(state)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(row) = row else {
                return Err(ServiceError::NotFound);
            };
            // Spent first, whatever happens next.
            sqlx::query("DELETE FROM lastfm_authorization WHERE state=?")
                .bind(state)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;

            let expires_at: i64 = row.try_get("expires_at")?;
            let cookie_hash: Vec<u8> = row.try_get("cookie_hash")?;
            // One refusal for every way of being wrong: expired, or the wrong
            // browser. Distinguishing them would tell whoever found the URL
            // which half they are missing.
            if now >= expires_at || !crate::security::token_matches(cookie, &cookie_hash) {
                return Err(ServiceError::NotFound);
            }
            (
                parse_uuid(row.try_get("user_id")?)?,
                row.try_get::<String, _>("destination")?,
                row.try_get::<String, _>("destination_fingerprint")?,
            )
        };

        let declared = self
            .scrobble_destinations
            .get(&(ScrobbleProvider::LastFm, destination.clone()));
        if declared.map(String::as_str) != Some(fingerprint.as_str()) {
            return Err(ServiceError::NotFound);
        }
        let exchange = self
            .lastfm_exchanges
            .get(&destination)
            .map(|found| Arc::clone(found.value()))
            .ok_or(ServiceError::Unavailable)?;
        // Outside the writer gate, deliberately: this is a network call, and
        // the process-wide gate is never held across one.
        let session_key: String = exchange.exchange(token).await?;
        self.link_scrobble(
            user_id,
            ScrobbleProvider::LastFm,
            &destination,
            &session_key,
        )
        .await?;
        Ok(destination)
    }

    /// Opens a Last.fm authorisation with no browser on this machine, and
    /// writes nothing down.
    ///
    /// **This is the journey RFC-010 deferred, and it is deferred no longer.**
    /// The reason it waited was that "writing two journeys at once is two
    /// chances to get the temporary state wrong". This one has no temporary
    /// state at all: the request token is Last.fm's, it expires on Last.fm's
    /// clock, and the person carries it between the two commands. There is no
    /// row, no cookie and no expiry of ours to get wrong — which is why it
    /// could be written second and not first.
    ///
    /// **It needs no `WAVEFLOW_PUBLIC_URL`.** The web journey does, because
    /// Last.fm has to bring a browser back to an address this server answers.
    /// Nothing comes back here, so the only requirement is an application —
    /// which is exactly what makes this the journey for a headless server.
    ///
    /// Answers the address to open and the token to bring back. Both carry the
    /// same secret and neither is ever logged.
    pub async fn begin_lastfm_approval(
        &self,
        destination: &str,
    ) -> Result<LastFmApproval, ServiceError> {
        // Declared, before anything leaves. A token asked for an instance this
        // server does not offer could not be exchanged into a link anyway, and
        // the refusal belongs before the network call rather than after it.
        if !self
            .scrobble_destinations
            .contains_key(&(ScrobbleProvider::LastFm, destination.to_owned()))
        {
            return Err(ServiceError::NotFound);
        }
        let exchange = self
            .lastfm_exchanges
            .get(destination)
            .map(|found| Arc::clone(found.value()))
            .ok_or(ServiceError::Unavailable)?;
        // Cloned out of the map first: the process-wide writer gate is never
        // held across a network call, and neither is a shard lock.
        exchange.request_token().await
    }

    /// Finishes it: exchanges the approved token and creates the link.
    ///
    /// The second half of [`Self::begin_lastfm_approval`], and stateless on
    /// this side — it verifies nothing about *which* journey the token came
    /// from, because there is no journey recorded. What guards it is that a
    /// request token is only worth a session once its owner has approved it in
    /// their own Last.fm account, and that this command names the account the
    /// link is created for.
    ///
    /// Not the web journey's `state` and cookie: those exist because that
    /// token travels back through a browser, through a referrer and through a
    /// history. This one never leaves the operator's terminal.
    pub async fn complete_lastfm_approval(
        &self,
        user_id: Uuid,
        destination: &str,
        token: &str,
    ) -> Result<(), ServiceError> {
        let exchange = self
            .lastfm_exchanges
            .get(destination)
            .map(|found| Arc::clone(found.value()))
            .ok_or(ServiceError::Unavailable)?;
        let session_key = exchange.exchange(token).await?;
        // `link_scrobble` refuses an instance this server does not declare, so
        // the pair is checked there rather than twice here.
        self.link_scrobble(user_id, ScrobbleProvider::LastFm, destination, &session_key)
            .await?;
        Ok(())
    }

    /// Drops the journeys that have run out.
    ///
    /// Called by the retention purge, because it exists anyway — and applying
    /// *each state's own* expiry rather than the queue's thirty-day window.
    /// Borrowing the wrong one of the two would keep a journey alive long after
    /// it should have died, and a revision that ends one table's unbounded
    /// growth would look poor introducing another.
    pub async fn purge_lastfm_authorizations(&self, now_ms: i64) -> Result<u64, ServiceError> {
        let _writer = self.db.writer_guard().await;
        let cut = sqlx::query("DELETE FROM lastfm_authorization WHERE expires_at <= ?")
            .bind(now_ms)
            .execute(self.db.pool())
            .await?;
        Ok(cut.rows_affected())
    }
}
