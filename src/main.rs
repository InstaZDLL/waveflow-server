use anyhow::Context;
use clap::Parser;
use tracing_subscriber::EnvFilter;
use waveflow_server::{cli, laptop, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    // After `.env`, so neither file overrides what the environment or the
    // working directory's `.env` already says.
    let laptop_env = waveflow_server::config::load_laptop_env()?;
    init_tracing()?;

    let cli = cli::Cli::parse();
    let config = Config::from_env()?;
    match cli.command {
        Some(cli::Command::Database {
            command: cli::DatabaseCommand::Restore(args),
        }) => cli::restore(&config, args).await,
        // Both of these run on a database the boot would refuse, so they run
        // instead of the boot rather than after it.
        Some(cli::Command::Key { command }) => cli::key(&config, command).await,
        command => {
            let serving = matches!(command, Some(cli::Command::Serve) | None);
            // Only when serving: a command's standard output is not the place
            // for it, for the reason the warnings in `serve` give.
            if let (true, Some(path)) = (serving, &laptop_env) {
                tracing::info!(path = %path.display(), "settings read from laptop.env");
            }
            // Before `initialize`: a second double-click must not open,
            // migrate or redraw the setup token of the database the first copy
            // is serving. An installed server binds where it always has, after
            // the boot.
            let listener = if serving && config.laptop {
                match laptop::bind(&config, laptop_env.as_deref()).await? {
                    laptop::Startup::Listening(listener) => Some(listener),
                    laptop::Startup::AlreadyRunning(url) => {
                        println!("WaveFlow Server is already running at {url}");
                        return Ok(());
                    }
                }
            } else {
                None
            };
            let state = waveflow_server::initialize(&config).await?;
            match command {
                Some(cli::Command::Serve) | None => serve(config, state, listener).await,
                Some(command) => cli::execute(command, &state).await,
            }
        }
    }
}

async fn serve(
    config: Config,
    state: waveflow_server::AppState,
    listener: Option<tokio::net::TcpListener>,
) -> anyhow::Result<()> {
    let bind_addr = config.bind_addr;
    // A laptop is told what its network can use instead, once the address is
    // known (`laptop::network_hint`).
    if config.public_url.is_none() && !config.laptop {
        tracing::warn!(
            "WAVEFLOW_PUBLIC_URL is not configured; browser origin validation falls back to the request Host header and cookies cannot be marked Secure"
        );
    }
    // Here rather than in `initialize`, with the other thing an operator is
    // told at startup. `initialize` runs for every CLI command too, and one of
    // those promises that a minted secret is alone on standard output — a
    // promise `a_minted_secret_leaves_on_standard_output_by_itself` holds, and
    // which this warning broke when it lived there.
    //
    // The listing already carries the reason for a client to show; this is for
    // the operator who never opens one.
    for destination in &config.destinations {
        if destination.provider == waveflow_server::services::ScrobbleProvider::LastFm
            && config.lastfm.is_none()
        {
            tracing::warn!(
                destination = %destination.name,
                "a last.fm destination is declared with no application; nothing will be sent to it"
            );
        }
    }
    // Here for the same reason as the warning above: a CLI command's standard
    // output is not the place for it. Where to find the token, never the token
    // (RFC-019 decision 1).
    match state.setup.source() {
        Some(waveflow_server::auth::setup_token::TokenSource::File(path)) => tracing::warn!(
            path = %path.display(),
            "no account exists yet: the first administrator is created with the setup token stored in this file"
        ),
        Some(waveflow_server::auth::setup_token::TokenSource::Operator) => tracing::warn!(
            "no account exists yet: the first administrator is created with the value of WAVEFLOW_SETUP_TOKEN"
        ),
        None => {}
    }
    state.scanner.spawn_background(config.scan_interval);
    // A change scanned once it settles, on top of the interval (RFC-021).
    if config.watch {
        waveflow_server::library::watcher::spawn(
            state.scanner.clone(),
            state.db.clone(),
            config.watch_delay,
        );
    }
    // Off unless WAVEFLOW_UNAVAILABLE_RETENTION_DAYS says otherwise.
    state.services.spawn_unavailable_purge();
    // At boot, then whenever a scan finishes or a library is asked to be
    // analysed again; one core at most by default, and off at zero (RFC-016).
    state.services.spawn_quality_analyzer();
    // Abandoned transfers hold the operator's disk, and nothing else would
    // reclaim it until somebody offered another file.
    state.services.spawn_upload_sweeper();
    state.services.spawn_canvas_sweeper();
    state.services.spawn_artwork_sweeper();
    state.services.spawn_library_event_purge();
    // Nothing to drain until an account links a destination, and no adapter is
    // registered yet — a server that was merely upgraded makes no outbound
    // request. RFC-010.
    state.services.spawn_scrobble_drain();
    // The eighth. A queue that never forgets grows by a row per listen and per
    // destination, for ever — RFC-010's retention section.
    state.services.spawn_scrobble_purge();
    // Off unless WAVEFLOW_PODCASTS says otherwise, and then the first pass
    // waits one interval: a boot fetches nothing (RFC-025 decision 9).
    state.services.spawn_podcast_refresher();
    state.db.spawn_authorization_pruning();
    // Refused sign-ins, counted and logged once a minute (RFC-017).
    state.auth.sign_in_guard().spawn_summary();
    let router = waveflow_server::app(&config, state);
    let listener = match listener {
        Some(listener) => listener,
        None => tokio::net::TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("bind failed on {bind_addr}"))?,
    };
    let local_addr = listener.local_addr()?;
    tracing::info!(address = %local_addr, data_dir = %config.data_dir.display(), "WaveFlow Server v2 ready");
    if config.laptop {
        if config.public_url.is_none() {
            tracing::info!("{}", laptop::network_hint(local_addr));
        }
        println!("{}", laptop::banner(local_addr, &config.data_dir));
    }
    // With the peer's address on every request: the sign-in limiter counts
    // failures per client (RFC-017 decision 2).
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(waveflow_server::shutdown_signal())
    .await
    .context("HTTP server failed")
}

fn init_tracing() -> anyhow::Result<()> {
    // Some real-world MP4 libraries contain empty optional `data` atoms. Lofty
    // safely skips them but emits one warning per atom, which can bury the scan
    // summary under thousands of harmless lines. Operators can still opt back
    // into that target explicitly through RUST_LOG when diagnosing tag files.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,lofty::mp4::ilst::read=error"));
    if std::env::var("WAVEFLOW_LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .try_init()
            .map_err(|error| anyhow::anyhow!("tracing init failed: {error}"))?;
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .try_init()
            .map_err(|error| anyhow::anyhow!("tracing init failed: {error}"))?;
    }
    Ok(())
}
