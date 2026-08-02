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
    state.scanner.spawn_background(config.scan_interval);
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
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
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
