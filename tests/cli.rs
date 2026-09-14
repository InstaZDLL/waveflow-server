//! The command line: what an operator can do on a host with no browser.
//!
//! Split out of `service.rs`, which names probes, the embedded client and the
//! backup bundle and had quietly accumulated these as well. A test belongs to
//! the surface it exercises, and this one is reached by argv and answered on
//! two streams — not by a request.

use clap::Parser;
use waveflow_server::authentication::now_ms;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
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
fn cli_command() -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_waveflow-server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("WAVEFLOW_") {
            command.env_remove(name);
        }
    }
    command
}

/// The CLI reads a queue and withdraws an authorisation, and says which
/// variable it wanted when the token is not there.
///
/// **Named for what it does.** It was called `..._links_and_unlinks_...` while
/// the link itself was posed through `DomainServices` — so it would have passed
/// with `cli::link_scrobble` deleted, which is the defect class this file's
/// commit set out to remove, reintroduced one file over.
///
/// What `link` *is* covered for: the admin check, the account lookup and the
/// provider parse, all of which run before the secret is read, and which the
/// absent-variable case below therefore traverses. What it is not covered for is
/// the two lines after that — the successful `read_secret_env` and the service
/// call. Driving those from here would need the process environment mutated
/// while sibling threads read it, which is the race this test exists without.
/// `a_minted_secret_leaves_on_standard_output_by_itself`, below, closes it with
/// a subprocess against `CARGO_BIN_EXE_waveflow-server`, where an environment
/// variable reaches the command without any sibling thread reading it.
#[tokio::test]
async fn the_cli_reads_a_queue_and_withdraws_an_authorisation() {
    let (_temp, _config, state) = test_app().await;
    inserted_account(&state, "cli-scrobble-admin", "admin").await;
    let user = inserted_account(&state, "cli-scrobble-user", "user").await;

    // **No `set_var` here, and that is the whole design of this test.** A review
    // pointed out that a unique variable name solves collision and not the
    // race: `cargo test` runs these as threads in one process, and mutating the
    // environment while a sibling thread reads it — `tempfile::tempdir()` reads
    // `TMPDIR` on every one of them — is undefined behaviour, which is why
    // edition 2024 made `set_var` unsafe. So `link` is exercised at the one
    // seam that needs no environment at all: a variable that is *absent*, which
    // is the default state of the process.
    let missing = run_cli(
        &state,
        &[
            "scrobble",
            "link",
            "--actor",
            "cli-scrobble-admin",
            "--username",
            "cli-scrobble-user",
            "--provider",
            "listenbrainz",
            "--token-env",
            "WAVEFLOW_TEST_TOKEN_THAT_IS_NEVER_SET",
        ],
    )
    .await
    .unwrap_err();
    // Naming the variable is the whole job of that message: the person who has
    // to fix this cannot see which one the command looked for.
    assert!(
        missing
            .to_string()
            .contains("WAVEFLOW_TEST_TOKEN_THAT_IS_NEVER_SET"),
        "{missing}"
    );

    // And the flag surface is pinned by parsing rather than by running, so the
    // names and the default are held still without the process being touched.
    let parsed = format!(
        "{:?}",
        waveflow_server::cli::Cli::try_parse_from([
            "waveflow-server",
            "scrobble",
            "link",
            "--actor",
            "a",
            "--username",
            "b",
            "--provider",
            "listenbrainz",
        ])
        .expect("the scrobble link flags have to parse — that is what this pins")
        .command
        .unwrap()
    );
    assert!(parsed.contains("WAVEFLOW_SCROBBLE_TOKEN"), "{parsed}");

    // The link itself is posed through the service, which is what the command
    // calls anyway — the assertions below then read it back the same way.
    state
        .services
        .link_scrobble(
            user,
            waveflow_server::services::ScrobbleProvider::ListenBrainz,
            "lb-token-from-the-shell",
        )
        .await
        .unwrap();

    let links = state.services.scrobble_links(user).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].provider.as_str(), "listenbrainz");
    assert_eq!(links[0].health, "healthy");

    // Reading the queue is a third gesture and must not need the secret.
    run_cli(
        &state,
        &[
            "scrobble",
            "status",
            "--actor",
            "cli-scrobble-admin",
            "--username",
            "cli-scrobble-user",
        ],
    )
    .await
    .unwrap();

    run_cli(
        &state,
        &[
            "scrobble",
            "unlink",
            "--actor",
            "cli-scrobble-admin",
            "--username",
            "cli-scrobble-user",
            "--provider",
            "listenbrainz",
        ],
    )
    .await
    .unwrap();
    assert!(state
        .services
        .scrobble_links(user)
        .await
        .unwrap()
        .is_empty());
}

/// What the command refuses, and who it refuses.
///
/// Both failures happen before any secret is read or sealed, which is the point
/// of checking them: a destination this server cannot name and an actor who is
/// not an administrator are both answered by the command rather than by the
/// database refusing a `CHECK` afterwards.
#[tokio::test]
async fn the_cli_refuses_an_unknown_destination_and_a_non_administrator() {
    let (_temp, _config, state) = test_app().await;
    inserted_account(&state, "cli-refusing-admin", "admin").await;
    inserted_account(&state, "cli-refusing-user", "user").await;

    let unknown = run_cli(
        &state,
        &[
            "scrobble",
            "unlink",
            "--actor",
            "cli-refusing-admin",
            "--username",
            "cli-refusing-user",
            "--provider",
            "spotify",
        ],
    )
    .await
    .unwrap_err();
    // The message names the three this server knows, because the person who
    // typed the fourth cannot be expected to guess them.
    let said = unknown.to_string();
    assert!(said.contains("listenbrainz"), "{said}");
    assert!(said.contains("maloja"), "{said}");
    assert!(said.contains("lastfm"), "{said}");

    // An ordinary account cannot pose an authorisation for somebody else, which
    // is the whole difference between this surface and the self-scoped route.
    let forbidden = run_cli(
        &state,
        &[
            "scrobble",
            "status",
            "--actor",
            "cli-refusing-user",
            "--username",
            "cli-refusing-user",
        ],
    )
    .await
    .unwrap_err();
    assert!(
        forbidden.to_string().contains("administrator"),
        "{forbidden}"
    );

    // **Admin before secret, and nothing else pins it.** `link` checks the
    // admin, resolves the account, parses the provider, and only then reads the
    // secret. The absent-variable case above traverses all four but asserts
    // about the last, so hoisting `read_secret_env` to the top of
    // `link_scrobble` would leave every CLI test green. This is what notices.
    //
    // It pins that one edge and not the whole order: moving the read between
    // the admin check and the account lookup still fails the admin first, and
    // this would stay green. Said plainly rather than claimed wider.
    let too_early = run_cli(
        &state,
        &[
            "scrobble",
            "link",
            "--actor",
            "cli-refusing-user",
            "--username",
            "cli-refusing-user",
            "--provider",
            "listenbrainz",
            "--token-env",
            "WAVEFLOW_TEST_TOKEN_THAT_IS_NEVER_SET",
        ],
    )
    .await
    .unwrap_err();
    let said = too_early.to_string();
    assert!(said.contains("administrator"), "{said}");
    assert!(
        !said.contains("WAVEFLOW_TEST_TOKEN_THAT_IS_NEVER_SET"),
        "the token lookup must not have happened yet"
    );
}

/// Every secret the CLI mints goes to standard output alone, prose elsewhere.
///
/// Through a subprocess because that is the only place the two streams are
/// separable: `cli::execute` called in-process writes both into the test
/// harness's own output, where nothing can tell them apart. This is the gap
/// `the_cli_reads_a_queue_and_withdraws_an_authorisation` names and leaves open.
///
/// What it buys is not tidiness. `token create … > secret` now yields a file
/// holding the token and nothing else, so the one copy of a freshly minted
/// secret need never be lifted out of a terminal that keeps its scrollback.
#[tokio::test]
async fn a_minted_secret_leaves_on_standard_output_by_itself() {
    let (temp, config, state) = test_app().await;
    inserted_account(&state, "stream-split-admin", "admin").await;
    inserted_account(&state, "stream-split-user", "user").await;
    // Closed before the subprocess opens the same database: one writer at a
    // time is this server's rule between threads, and it is no weaker between
    // processes.
    state.db.pool().close().await;

    let run = cli_command()
        .current_dir(temp.path())
        .env("WAVEFLOW_DATA_DIR", &config.data_dir)
        .args([
            "token",
            "create",
            "--actor",
            "stream-split-admin",
            "--username",
            "stream-split-user",
            "--name",
            "a name for the listing",
            "--scopes",
            "library:read",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8(run.stdout).unwrap();
    let stderr = String::from_utf8(run.stderr).unwrap();
    assert!(
        run.status.success(),
        "minting a token failed; its diagnostics were: {stderr}"
    );
    // One line, and that line is the token. Counted before trimming: trimming
    // first would fold `token\n\n` into `token` and call it one line, and a
    // blank line after a secret is exactly the kind of stray output this is
    // here to notice.
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "standard output should hold one line, and held: {lines:?}"
    );
    let token = lines[0];
    assert!(
        token.starts_with("wfapi_"),
        "standard output should hold the token itself, not a sentence about it"
    );

    // The prose is on the other stream, and — the point of the whole change —
    // the secret is not repeated there. A split that announced the token on
    // both streams would pass every assertion above and protect nobody.
    assert!(
        stderr.contains("stream-split-user"),
        "the human-readable half should say whose token this is"
    );
    assert!(
        !stderr.contains(token),
        "standard error must not repeat the secret"
    );

    // The other site that mints a secret, split the same way — and reached
    // through the same subprocess, which is also what lets the password arrive
    // in an environment variable without racing the sibling threads that read
    // the process environment in-process.
    let run = cli_command()
        .current_dir(temp.path())
        .env("WAVEFLOW_DATA_DIR", &config.data_dir)
        .env("WAVEFLOW_SUBSONIC_PASSWORD", "a long enough password")
        .args([
            "credential",
            "set",
            "--actor",
            "stream-split-admin",
            "--username",
            "stream-split-user",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8(run.stdout).unwrap();
    let stderr = String::from_utf8(run.stderr).unwrap();
    assert!(
        run.status.success(),
        "setting a credential failed; its diagnostics were: {stderr}"
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "standard output should hold one line, and held: {lines:?}"
    );
    let api_key = lines[0];
    assert!(
        api_key.starts_with("wfsk_"),
        "standard output should hold the API key and nothing else"
    );
    assert!(
        !stderr.contains(api_key),
        "standard error must not repeat the API key"
    );
    // The password it was handed is a secret too, and nothing prints it.
    assert!(
        !stderr.contains("a long enough password") && !stdout.contains("a long enough password"),
        "neither stream may echo the password it was given"
    );
}
