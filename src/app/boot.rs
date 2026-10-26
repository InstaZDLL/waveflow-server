//! Bringing the state up — the migrations, the instance key, every service
//! and outbound adapter — and the signal that takes the server down.

use std::{sync::Arc, time::Duration};

use super::state::{chunk_body_limit, negotiation_body_limit, AppState};
use crate::{
    authentication, database, enrichment, link_roots, lrclib, media, migration_backup, quality,
    scanner, scrobblers, security, services, setup_token, sync, Config,
};

pub async fn initialize(config: &Config) -> anyhow::Result<AppState> {
    let db = database::Database::open(config).await?;
    let booted = boot(db.clone(), config).await;
    if booted.is_err() {
        // A refused boot closes what it opened before it says so. Dropping the
        // pool would not be enough: sqlx closes each connection on its own
        // thread after the drop has returned, and the last one to close
        // checkpoints the WAL under an exclusive lock. Whoever opens the file
        // next — the operator retrying, or a test that has just watched the
        // refusal — races that close and can find the database locked.
        db.pool().close().await;
    }
    booted
}

async fn boot(db: database::Database, config: &Config) -> anyhow::Result<AppState> {
    // Before anything is applied, and only when something will be (RFC-019
    // decision 3): the copy is the database as this boot found it.
    let copy = migration_backup::before_migrating(&db, config).await?;
    if let Err(error) = db.migrate().await {
        return Err(match copy {
            Some(copy) => error.context(migration_backup::restore_hint(&copy)),
            None => error,
        });
    }
    let secret_box = Arc::new(security::SecretBox::load_or_create(
        &config.instance_key_path,
    )?);
    let instance_key = std::fs::read(&config.instance_key_path)?;
    let fingerprint = security::bytes_hash(&instance_key);
    if !db
        .bind_instance_key(&fingerprint, authentication::now_ms())
        .await?
    {
        anyhow::bail!(
            "instance.key does not match waveflow.db; restore the database and key from the same backup bundle"
        );
    }
    // After the key check, not before: a mismatched pair aborts the boot, and
    // scheduling a rescan on a database this instance is about to refuse would
    // be writing to a catalogue that is not ours.
    db.reconcile_catalog_identity(&config.pid).await?;
    // After the key check too: a boot that is about to be refused has no
    // business writing a token into a data directory that is not its own.
    let setup = Arc::new(setup_token::SetupGate::prepare(
        config,
        db.setup_required().await?,
    )?);
    let auth = authentication::AuthService::new(db.clone(), config);
    let scanner = scanner::ScanManager::new(
        db.clone(),
        config.artwork_dir.clone(),
        config.scan_parallelism,
    )
    .with_link_roots(link_roots::LinkRoots::new(config.link_roots.clone()));
    let media = media::MediaService::initialize(config).await?;
    let sync = sync::SyncService::new(db.clone());
    let services = services::DomainServices::new(
        db.clone(),
        Arc::clone(&secret_box),
        sync.clone(),
        scanner.clone(),
        config,
    );
    // One of the two outbound surfaces this server has — metadata enrichment,
    // below, is the other — and it is switched on by the operator naming a
    // destination rather than by anybody linking an account.
    // Nothing leaves until both are true: an adapter with no authorisation
    // behind it is never handed a listen. RFC-010 decision 11.
    for destination in &config.destinations {
        // `Config::from_env` refuses to boot on a malformed destination, so the
        // URL reaching here has already been through `validate_destination` —
        // and `for_data_dir` builds an empty list rather than an unchecked one.
        //
        // Building the client is a different matter and really can fail at
        // runtime, for reasons that have nothing to do with the operator's URL.
        let target = match destination.provider {
            services::ScrobbleProvider::ListenBrainz => {
                scrobblers::outbound_client(config.scrobbling.request_timeout)
                    .and_then(|client| {
                        scrobblers::listenbrainz::ListenBrainz::new(client, &destination.url)
                    })
                    .map(|target| Arc::new(target) as Arc<dyn services::ScrobbleTarget>)
            }
            services::ScrobbleProvider::Maloja => {
                scrobblers::outbound_client(config.scrobbling.request_timeout)
                    .and_then(|client| scrobblers::maloja::Maloja::new(client, &destination.url))
                    .map(|target| Arc::new(target) as Arc<dyn services::ScrobbleTarget>)
            }
            // **Last.fm needs more than a URL**, and a destination declared
            // without it is not an error. Decision 4 forbids shipping any
            // provider credential in an AGPL binary, so the application belongs
            // to the operator; without one, the name stays listed — with the
            // reason, which `GET /api/v2/scrobble-destinations` publishes — and
            // nothing drains for it.
            services::ScrobbleProvider::LastFm => {
                let Some(application) = config.lastfm.clone() else {
                    continue;
                };
                scrobblers::outbound_client(config.scrobbling.request_timeout)
                    .and_then(|client| {
                        scrobblers::lastfm::LastFm::new(
                            client,
                            &destination.url,
                            application,
                            config.scrobbling.request_timeout,
                        )
                    })
                    .map(|target| {
                        let target = Arc::new(target);
                        // The same object in both registries: submitting a
                        // listen and exchanging a token are two calls to one
                        // destination, signed by one application, and splitting
                        // them would be two places for that to drift.
                        services.register_lastfm_exchange(
                            &destination.name,
                            Arc::clone(&target) as Arc<dyn services::LastFmSessionExchange>,
                        );
                        target as Arc<dyn services::ScrobbleTarget>
                    })
            }
        };
        match target {
            Ok(target) => {
                services.register_scrobble_target(destination.provider, &destination.name, target)
            }
            Err(error) => {
                // **Never the URL itself.** `validate_destination` refuses a
                // destination that carries credentials, so the one thing this
                // branch is most likely to be handed is exactly the thing that
                // must not be printed: `http://user:secret@host` would land in
                // a startup log, or a support paste, in full. The variant names
                // the fault precisely enough without it, and this repository's
                // rule about secrets has no exception for an error path.
                anyhow::bail!(
                    "the configured {} destination {:?} is unusable: {error}",
                    destination.provider.as_str(),
                    destination.name
                )
            }
        }
    }
    // Catching up and reconciling, once, under the writer gate — and *before*
    // `main` starts the drain. The order is half the rule: reconciling while
    // the background task already runs leaves a window, short and sufficient,
    // in which listens leave for yesterday's destination. RFC-010 decision 10.
    services.reconcile_scrobble_links().await?;
    // A portrait search left fetching has no task across a restart; left at
    // `complete = 0` it would hold its account's next search at 429 until its
    // deadline passed (RFC-013 decision 4).
    services.forget_unfinished_image_searches().await?;
    // A podcast download the last process left running has no task any more
    // (RFC-025): it goes back to `new`, and its staging file goes.
    services.reconcile_podcasts().await?;
    // Off unless the operator named a provider, and then it still makes no
    // request until somebody asks about an artist (RFC-011 decisions 3 and 6).
    services.use_enrichment(enrichment::Enrichment::new(
        db.clone(),
        &config.metadata,
        config.artwork_dir.clone(),
    )?);
    // Off unless the operator named `lrclib`, and then it still asks nothing
    // until somebody asks for the lyrics of a track that has none (RFC-024
    // decisions 1 and 3).
    services.use_lyrics_lookup(lrclib::LyricsLookup::new(db.clone(), &config.lyrics)?);
    // Before anything reads `track_quality`: a measurement another analyser or
    // another FFmpeg made is deleted here, so every read can call a row current
    // on its content hash alone (RFC-016 decision 5).
    services
        .use_quality_decoder(quality::Decoder {
            digest: media.ffmpeg_digest().to_owned(),
            webp: quality::probe_webp(&config.ffmpeg_path).await,
        })
        .await?;
    Ok(AppState {
        db,
        auth,
        secret_box,
        scanner,
        media,
        services,
        sync,
        artwork_dir: config.artwork_dir.clone(),
        instance_key_path: config.instance_key_path.clone(),
        public_url: config.public_url.clone(),
        stream_ticket_ttl: config.stream_ticket_ttl,
        refresh_token_ttl: config.refresh_token_ttl,
        upload_batch_body_limit: negotiation_body_limit(&config.uploads),
        upload_chunk_body_limit: chunk_body_limit(&config.uploads),
        // Refused at startup if it does not fit, so this conversion cannot be
        // the place that decides what a ceiling falls back to.
        canvas_body_limit: usize::try_from(config.canvas.max_bytes).unwrap_or(usize::MAX),
        trusted_proxies: config.trusted_proxies.clone().into(),
        setup,
    })
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %error, "failed to install Ctrl-C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(error = %error, "failed to install terminate handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tokio::time::sleep(Duration::from_millis(25)).await;
}
