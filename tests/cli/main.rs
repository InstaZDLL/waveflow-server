//! The command line: what an operator can do on a host with no browser.
//!
//! Split out of `service.rs`, which names probes, the embedded client and the
//! backup bundle and had quietly accumulated these as well. A test belongs to
//! the surface it exercises, and this one is reached by argv and answered on
//! two streams — not by a request.

use clap::Parser;
use waveflow_server::auth::authentication::now_ms;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

/// One command line, parsed the way a shell would hand it over.
///
/// Through `Cli::parse_from` rather than by building the argument struct: the
/// fields are private, which is the hint, and this way the flag names and the
/// defaults are part of what these tests hold still. A command surface gets
/// those wrong far more easily than it gets the call wrong.
async fn run_cli(state: &waveflow_server::AppState, argv: &[&str]) -> anyhow::Result<()> {
    let mut full = vec!["waveflow-server"];
    full.extend_from_slice(argv);
    let cli = waveflow_server::cli::Cli::try_parse_from(full)?;
    waveflow_server::cli::execute(cli.command.expect("a command was given"), state).await
}

/// An account inserted rather than created, with a placeholder where a hash
/// would be.
///
/// These never authenticate: the CLI resolves them by username and checks a
/// role, and nothing in this file logs in as one. Calling
/// `security::hash_password` for them would add another occurrence of the
/// repository's fixture password, which CodeQL raises as a hard-coded
/// credential on every new one — there are already seventy-seven of those
/// dismissed on `main`, and adding to that pile for accounts that cannot log in
/// would be spending somebody else's attention.
async fn inserted_account(
    state: &waveflow_server::AppState,
    username: &str,
    role: &str,
) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', ?, 0, ?, ?)",
    )
    .bind(id.to_string())
    .bind(username)
    .bind(role)
    .bind(now)
    .bind(now)
    .execute(state.db.pool())
    .await
    .unwrap();
    id
}

/// The binary, with nothing `WAVEFLOW_*` inherited from whoever ran the suite.
///
/// `Config::from_env` runs before anything else and verifies the FFmpeg paths
/// it finds there, so one stray `WAVEFLOW_FFPROBE_PATH` on a developer's
/// machine fails these commands before they reach a single assertion. A test
/// that passes or fails on the environment of the person running it is
/// measuring that, and not the code.
///
/// Not `env_clear`: on Windows that takes `SystemRoot` with it, and the process
/// then cannot start at all. Only this server's own namespace is removed, and
/// each caller puts back the variables its command actually needs.
///
/// Matched without regard to case, because Windows keeps the name as it was
/// first spelled and resolves it case-insensitively: `set waveflow_ffprobe_path`
/// stores that spelling, `std::env::var("WAVEFLOW_FFPROBE_PATH")` in the child
/// still finds it, and an exact-case filter would walk straight past it. On Unix
/// the two are different variables and only the upper one is ever read, so
/// dropping the other costs nothing.
fn cli_command() -> std::process::Command {
    command_for(std::path::Path::new(env!("CARGO_BIN_EXE_waveflow-server")))
}

/// [`cli_command`] for a copy of the binary somewhere else: laptop mode reads
/// the `laptop.env` beside whichever executable runs.
fn command_for(program: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("WAVEFLOW_")
        {
            command.env_remove(name);
        }
    }
    command
}

async fn count(state: &waveflow_server::AppState, query: &'static str) -> i64 {
    sqlx::query_scalar::<_, i64>(query)
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

mod key;
mod laptop;
mod library;
mod scrobbling;
