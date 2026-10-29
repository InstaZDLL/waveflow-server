//! The scrobbling commands: the queue, authorisations, destinations and the
//! secrets they mint.

use super::*;

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
            "--destination",
            "default",
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
            "--destination",
            "default",
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
            "default",
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
            "--destination",
            "default",
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
            "--destination",
            "default",
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
            "--destination",
            "default",
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

/// A double for the half of the Last.fm protocol that would leave this machine.
///
/// Small and local rather than shared with `tests/scrobbling/lastfm_journey.rs`: what this
/// file needs is an exchange that exists, and a token it can recognise coming
/// back out.
struct Approves;

impl waveflow_server::services::LastFmSessionExchange for Approves {
    fn exchange<'a>(
        &'a self,
        token: &'a str,
    ) -> futures_util::future::BoxFuture<'a, Result<String, waveflow_server::services::ServiceError>>
    {
        Box::pin(async move { Ok(format!("session-for-{token}")) })
    }

    fn request_token<'a>(
        &'a self,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<waveflow_server::services::LastFmApproval, waveflow_server::services::ServiceError>,
    > {
        Box::pin(async move {
            Ok(waveflow_server::services::LastFmApproval {
                authorize_url: "https://www.last.fm/api/auth/?api_key=k&token=t".to_owned(),
                token: "t".to_owned(),
            })
        })
    }
}

/// Last.fm, from a host with no browser on it — which is this file's premise.
///
/// `link` cannot serve here: Last.fm hands out no secret a person can paste,
/// so the two commands split where the person has to leave and come back.
/// `authorize` writes nothing down — the request token is Last.fm's, and it
/// expires on Last.fm's clock — which is why this test can assert that running
/// it leaves the account exactly as unlinked as it found it.
///
/// `exchange` is exercised at the seam that needs no environment, for the
/// reason the test above gives at length: mutating the process environment
/// while sibling threads read it is the race this file exists without.
#[tokio::test]
async fn the_cli_authorises_last_fm_without_a_browser() {
    let (_temp, _config, state) = test_app().await;
    inserted_account(&state, "cli-lastfm-admin", "admin").await;
    let user = inserted_account(&state, "cli-lastfm-user", "user").await;
    state.services.register_lastfm_exchange(
        "default",
        std::sync::Arc::new(Approves)
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );

    // The whole first command, end to end: it needs no secret from the
    // environment, so nothing here has to be mutated for it to run.
    run_cli(
        &state,
        &[
            "scrobble",
            "authorize",
            "--actor",
            "cli-lastfm-admin",
            "--username",
            "cli-lastfm-user",
            "--destination",
            "default",
        ],
    )
    .await
    .unwrap();

    // And it left nothing behind. Asking Last.fm for a token is not half a
    // link: an authorisation nobody completes has to cost this account
    // nothing, and there is no row of ours for a purge to have to find.
    assert!(state
        .services
        .scrobble_links(user)
        .await
        .unwrap()
        .is_empty());

    // An instance nobody declared is refused, and by the command rather than
    // by the service being lenient.
    assert!(run_cli(
        &state,
        &[
            "scrobble",
            "authorize",
            "--actor",
            "cli-lastfm-admin",
            "--username",
            "cli-lastfm-user",
            "--destination",
            "not-declared",
        ],
    )
    .await
    .is_err());

    let missing = run_cli(
        &state,
        &[
            "scrobble",
            "exchange",
            "--actor",
            "cli-lastfm-admin",
            "--username",
            "cli-lastfm-user",
            "--destination",
            "default",
            "--token-env",
            "WAVEFLOW_TEST_LASTFM_TOKEN_THAT_IS_NEVER_SET",
        ],
    )
    .await
    .unwrap_err();
    assert!(
        missing
            .to_string()
            .contains("WAVEFLOW_TEST_LASTFM_TOKEN_THAT_IS_NEVER_SET"),
        "{missing}"
    );

    // The flag surface, pinned by parsing rather than by running — including
    // the default variable name, which is what an operator reads in `--help`
    // and types into their shell.
    let parsed = format!(
        "{:?}",
        waveflow_server::cli::Cli::try_parse_from([
            "waveflow-server",
            "scrobble",
            "exchange",
            "--actor",
            "a",
            "--username",
            "b",
            "--destination",
            "default",
        ])
        .expect("the scrobble exchange flags have to parse — that is what this pins")
        .command
        .unwrap()
    );
    assert!(parsed.contains("WAVEFLOW_LASTFM_TOKEN"), "{parsed}");

    // The second half, through the service the command calls: the token comes
    // back, and the link is made under the session key it was exchanged for.
    state
        .services
        .complete_lastfm_approval(user, "default", "t")
        .await
        .unwrap();
    let links = state.services.scrobble_links(user).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].provider.as_str(), "lastfm");
    assert_eq!(links[0].destination, "default");
}

/// A server that never asked for Last.fm boots without being told its Last.fm
/// destination has no application; one whose operator named a Last.fm
/// instance still is.
///
/// Through the real executable, because the warning is written by `serve` in
/// `main.rs`, which no in-process test reaches. The port is held, so each boot
/// gets as far as its warnings and stops at the bind.
#[test]
fn the_default_last_fm_instance_is_not_reported_at_boot() {
    const WARNING: &str = "a last.fm destination is declared with no application";
    let boot = |named: Option<&str>| {
        let working = tempfile::tempdir().unwrap();
        let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut command = cli_command();
        command
            .current_dir(working.path())
            .env("WAVEFLOW_DATA_DIR", working.path().join("data"))
            .env("WAVEFLOW_BIND", holder.local_addr().unwrap().to_string())
            .env("RUST_LOG", "info")
            .stdin(std::process::Stdio::null());
        if let Some(url) = named {
            command.env("WAVEFLOW_SCROBBLE_LASTFM_URL", url);
        }
        let output = command.output().unwrap();
        drop(holder);
        let written = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            written.contains("bind failed"),
            "the boot did not get as far as the bind: {written}"
        );
        written
    };

    let default = boot(None);
    assert!(
        !default.contains(WARNING),
        "a server that never asked for Last.fm was warned about it: {default}"
    );
    let named = boot(Some("https://ws.audioscrobbler.com"));
    assert!(
        named.contains(WARNING),
        "a Last.fm instance the operator named, with no application, went unreported: {named}"
    );
}
