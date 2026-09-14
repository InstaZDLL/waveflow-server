use anyhow::Context;
use clap::Parser;
use tracing_subscriber::EnvFilter;
use waveflow_server::{cli, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    init_tracing()?;

    let cli = cli::Cli::parse();
    let config = Config::from_env()?;
    match cli.command {
        Some(cli::Command::Database {
            command: cli::DatabaseCommand::Restore(args),
        }) => cli::restore(&config, args).await,
        command => {
            let state = waveflow_server::initialize(&config).await?;
            match command {
                Some(cli::Command::Serve) | None => serve(config, state).await,
                Some(command) => cli::execute(command, &state).await,
            }
        }
    }
}

async fn serve(config: Config, state: waveflow_server::AppState) -> anyhow::Result<()> {
    let bind_addr = config.bind_addr;
    if config.public_url.is_none() {
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
    state.scanner.spawn_background(config.scan_interval);
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
    state.db.spawn_authorization_pruning();
    let router = waveflow_server::app(&config, state);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind failed on {bind_addr}"))?;
    let local_addr = listener.local_addr()?;
    tracing::info!(address = %local_addr, data_dir = %config.data_dir.display(), "WaveFlow Server v2 ready");
    axum::serve(listener, router)
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
