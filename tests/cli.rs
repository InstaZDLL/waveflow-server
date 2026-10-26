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
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_waveflow-server"));
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

// ---------------------------------------------------------------------------
// Replacing the instance key
//
// These four run against `cli::key` rather than `cli::execute`, because that is
// where the commands live: they answer the boot that aborts on a mismatched
// pair, so they cannot be reached through an `AppState` the boot would never
// have built. The suite's other CLI tests all start from one.
//
// Every fixture below closes the pool and drops the state before the command
// runs. That is not tidiness: `open_alone` refuses a database another
// connection is holding, and leaving one open is how three of these would pass
// by refusing for the wrong reason.
// ---------------------------------------------------------------------------

/// One installation holding every value an instance key governs.
struct Sealed {
    owner: uuid::Uuid,
    track: uuid::Uuid,
    /// Drawn here rather than written in, so an assertion that it came back
    /// cannot be met by a constant the code could have carried.
    subsonic_password: String,
    scrobble_secret: String,
    share: uuid::Uuid,
    /// The public URL as it stood under the first key.
    share_token: String,
}

/// How many of each the fixture seeds.
///
/// Four different numbers, on purpose. An earlier version seeded one of
/// everything, and an assertion on the line the command prints then held just
/// as well with two of its four fields swapped — the inversion that found it
/// passed. A count can only be checked against a count nothing else shares.
const SEALED_CREDENTIALS: usize = 2;
const SEALED_LISTENS: usize = 3;
const SEALED_SHARES: usize = 4;

async fn sealed_installation(
    config: &waveflow_server::Config,
    state: &waveflow_server::AppState,
    name: &str,
) -> Sealed {
    let owner = inserted_account(state, name, "admin").await;

    let music = config.data_dir.join(name);
    std::fs::create_dir_all(&music).unwrap();
    generate_audio_fixture(&music.join("sealed.flac"), "flac", "flac");
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            name,
            &root,
            waveflow_server::storage::database::LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        state,
        owner,
        waveflow_server::storage::catalog::LibraryRecord {
            id: library,
            name: name.into(),
            root_path: root,
        },
    )
    .await;
    let track: uuid::Uuid =
        sqlx::query_scalar::<_, String>("SELECT id FROM track WHERE library_id=?")
            .bind(library.to_string())
            .fetch_one(state.db.pool())
            .await
            .expect("the fixture file must have been scanned")
            .parse()
            .unwrap();

    let subsonic_password = format!("subsonic-{}", uuid::Uuid::new_v4());
    state
        .services
        .set_subsonic_credential(owner, name, &subsonic_password)
        .await
        .unwrap();
    for extra in 1..SEALED_CREDENTIALS {
        let other = format!("{name}-{extra}");
        inserted_account(state, &other, "user").await;
        state
            .services
            .set_subsonic_credential(owner, &other, &format!("subsonic-{}", uuid::Uuid::new_v4()))
            .await
            .unwrap();
    }

    let scrobble_secret = format!("listenbrainz-{}", uuid::Uuid::new_v4());
    state
        .services
        .link_scrobble(
            owner,
            waveflow_server::services::ScrobbleProvider::ListenBrainz,
            "default",
            &scrobble_secret,
        )
        .await
        .unwrap();
    // Listens behind the link, which `ON DELETE CASCADE` takes with it. Dated
    // apart so none of them is the same listen twice.
    for listen in 0..SEALED_LISTENS {
        state
            .services
            .scrobble(
                owner,
                track,
                true,
                Some(now_ms() - (listen as i64 + 1) * 600_000),
            )
            .await
            .unwrap();
    }

    let mut share = state
        .services
        .create_share(owner, &[track], Some("carried over"), None)
        .await
        .unwrap();
    for extra in 1..SEALED_SHARES {
        state
            .services
            .create_share(owner, &[track], Some(&format!("also {extra}")), None)
            .await
            .unwrap();
    }
    let share_token = share.url_token.take().expect("a new share names its URL");
    assert!(
        state.services.public_share(&share_token).await.is_ok(),
        "the fixture share must resolve before the key changes"
    );
    for (table, query, expected) in [
        (
            "subsonic_credential",
            "SELECT COUNT(*) FROM subsonic_credential",
            SEALED_CREDENTIALS,
        ),
        ("scrobble_link", "SELECT COUNT(*) FROM scrobble_link", 1),
        (
            "scrobble_outbox",
            "SELECT COUNT(*) FROM scrobble_outbox",
            SEALED_LISTENS,
        ),
        ("share", "SELECT COUNT(*) FROM share", SEALED_SHARES),
    ] {
        assert_eq!(
            count(state, query).await as usize,
            expected,
            "the fixture must seed {expected} row(s) in {table}"
        );
    }

    Sealed {
        owner,
        track,
        subsonic_password,
        scrobble_secret,
        share: share.id,
        share_token,
    }
}

/// Closes every connection, so the command can take the database.
async fn stop_server(state: waveflow_server::AppState) {
    state.db.pool().close().await;
    drop(state);
}

async fn count(state: &waveflow_server::AppState, query: &'static str) -> i64 {
    sqlx::query_scalar::<_, i64>(query)
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

/// The sealed scrobbling secret, read the way the drain reads it.
///
/// There is no public accessor for it — `due_scrobbles` is private and joins
/// the outbox — so the row is read here and opened with the same `SecretBox`
/// the drain would use. What is being checked is the ciphertext, not the join.
///
/// `None` rather than a panic when it will not open, because that is the
/// failure a rotation that skipped this table produces, and a test must fail
/// on the assertion that names the defect rather than on the line before it.
async fn scrobble_secret(state: &waveflow_server::AppState) -> Option<Vec<u8>> {
    let (nonce, ciphertext) = sqlx::query_as::<_, (Vec<u8>, Vec<u8>)>(
        "SELECT credential_nonce, credential_ciphertext FROM scrobble_link",
    )
    .fetch_one(state.db.pool())
    .await
    .expect("the link must still be there");
    state.secret_box.decrypt(&nonce, &ciphertext).ok()
}

/// Whether the command refused, and said why.
///
/// Read as a value rather than through `unwrap_err`: a command that went
/// ahead when it should have refused must fail on the assertion that names the
/// refusal, not on the unwrapping above it.
fn refused_with(outcome: &anyhow::Result<()>, reason: &str) -> bool {
    outcome
        .as_ref()
        .err()
        .is_some_and(|error| error.to_string().contains(reason))
}

/// The single `pre-key-*` directory the command left behind.
fn recovery_directory(config: &waveflow_server::Config) -> std::path::PathBuf {
    let mut found: Vec<_> = std::fs::read_dir(&config.data_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("pre-key-"))
        })
        .collect();
    assert_eq!(found.len(), 1, "exactly one recovery directory: {found:?}");
    found.pop().unwrap()
}

/// Rotation carries every secret onto the new key, and re-points the shares.
///
/// The share half is the one worth stating plainly. A share token is derived
/// from the instance key rather than stored, so a key change that left
/// `token_hash` alone would not re-address the share — it would strand it,
/// behind a URL its owner is still shown and nobody can open. "Shares survive
/// with a new URL" is only true because `rekey` recomputes the hash, and the
/// two assertions below are what separate that from the wish.
#[tokio::test]
async fn rotating_the_key_keeps_every_secret_and_re_points_the_shares() {
    let (_temp, config, state) = test_app().await;
    let sealed = sealed_installation(&config, &state, "rotate-owner").await;
    let key_before = std::fs::read(&config.instance_key_path).unwrap();
    let queued_before = count(&state, "SELECT COUNT(*) FROM scrobble_outbox").await;
    stop_server(state).await;

    waveflow_server::cli::key(
        &config,
        waveflow_server::cli::KeyCommand::Rotate(waveflow_server::cli::KeyArgs { confirm: true }),
    )
    .await
    .unwrap();

    let key_after = std::fs::read(&config.instance_key_path).unwrap();
    assert_ne!(key_after, key_before, "the key file was not replaced");
    let recovery = recovery_directory(&config);
    assert_eq!(
        std::fs::read(recovery.join("instance.key")).ok().as_deref(),
        Some(key_before.as_slice()),
        "the recovery bundle must hold the key as it was"
    );
    assert!(
        waveflow_server::storage::database::Database::check_file(&recovery.join("waveflow.db"))
            .await
            .unwrap_or(false),
        "the recovery copy must be a sound database"
    );

    let state = waveflow_server::initialize(&config).await.unwrap();
    let credential = state
        .services
        .credential_by_username("rotate-owner")
        .await
        .unwrap()
        .expect("the credential must still be there");
    assert_eq!(
        state
            .services
            .decrypt_subsonic_password(&credential)
            .ok()
            .as_deref(),
        Some(sealed.subsonic_password.as_bytes()),
        "the Subsonic password did not survive the rotation"
    );
    assert_eq!(
        scrobble_secret(&state).await.as_deref(),
        Some(sealed.scrobble_secret.as_bytes()),
        "the scrobbling secret did not survive the rotation"
    );
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM scrobble_outbox").await,
        queued_before,
        "rotation must not touch the queue"
    );

    let moved = state.secret_box.derive_share_token(sealed.share);
    assert_ne!(moved, sealed.share_token, "the URL did not change");
    assert!(
        matches!(
            state.services.public_share(&sealed.share_token).await,
            Err(waveflow_server::services::ServiceError::NotFound)
        ),
        "a link handed out under the old key still opens"
    );
    let reached = state
        .services
        .public_share(&moved)
        .await
        .expect("the share must answer on its new URL");
    assert_eq!(reached.id, sealed.share);
    assert_eq!(
        reached.songs.len(),
        1,
        "the share must still carry its track"
    );
}

/// Adoption discards what cannot be read, and only that.
///
/// The queue is the part an operator would not predict: the outbox hangs off
/// `scrobble_link` by `ON DELETE CASCADE`, which only fires with foreign keys
/// enforced — and the migration this command runs first leaves them off.
///
/// Three things would have to go for a stranded queue to reach anybody: the
/// `PRAGMA` `rekey` asks for itself, the orphan count it rolls back on, and
/// the `foreign_key_check` the next boot runs. Removing the first fails this
/// test on the command; removing the first two fails it on the boot below. So
/// the count further down never gets to be the line that fails, and is kept
/// as the statement of what an adoption leaves behind rather than as the
/// guard — which is where the guard actually is.
#[tokio::test]
async fn adopting_a_database_discards_what_the_lost_key_sealed_and_nothing_else() {
    let (_temp, config, state) = test_app().await;
    let sealed = sealed_installation(&config, &state, "adopt-owner").await;
    stop_server(state).await;
    // The situation this command exists for, in its harder form: not a file
    // that is gone but one that is no longer a key. `adopt` has to accept it
    // — refusing to read it would refuse the operator holding it — and move
    // it aside with the rest of the pair.
    std::fs::write(&config.instance_key_path, [1u8; 7]).unwrap();

    waveflow_server::cli::key(
        &config,
        waveflow_server::cli::KeyCommand::Adopt(waveflow_server::cli::KeyArgs { confirm: true }),
    )
    .await
    .unwrap();

    let state = waveflow_server::initialize(&config).await.unwrap();
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM subsonic_credential").await,
        0,
        "an unreadable Subsonic password was kept"
    );
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM scrobble_link").await,
        0,
        "an unreadable scrobbling secret was kept"
    );
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM scrobble_outbox").await,
        0,
        "the queue outlived the links it belongs to"
    );

    // Everything the key never touched.
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM account").await,
        SEALED_CREDENTIALS as i64,
        "an account was lost with the secret it held"
    );
    assert_eq!(count(&state, "SELECT COUNT(*) FROM track").await, 1);
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM share").await,
        SEALED_SHARES as i64,
        "a share was lost rather than re-addressed"
    );
    let songs = state
        .services
        .songs_by_ids(sealed.owner, &[sealed.track])
        .await
        .unwrap();
    assert_eq!(songs.len(), 1, "the catalogue must come back untouched");

    let moved = state.secret_box.derive_share_token(sealed.share);
    assert_ne!(moved, sealed.share_token);
    let reached = state
        .services
        .public_share(&moved)
        .await
        .expect("a share survives an adoption, with a new URL");
    assert_eq!(reached.id, sealed.share);

    let recovery = recovery_directory(&config);
    assert_eq!(
        std::fs::read(recovery.join("instance.key")).ok().as_deref(),
        Some([1u8; 7].as_slice()),
        "the unusable key was dropped instead of being moved aside"
    );
}

/// The three situations in which a key change must not happen at all.
#[tokio::test]
async fn a_key_change_refuses_a_live_server_a_matching_key_and_a_foreign_one() {
    let (_temp, config, state) = test_app().await;
    sealed_installation(&config, &state, "refusal-owner").await;
    let key_before = std::fs::read(&config.instance_key_path).unwrap();

    // One: the server is still holding the database.
    let refused = waveflow_server::cli::key(
        &config,
        waveflow_server::cli::KeyCommand::Rotate(waveflow_server::cli::KeyArgs { confirm: true }),
    )
    .await;
    assert!(
        refused_with(&refused, "open in another process"),
        "a live server was not detected: {refused:?}"
    );
    stop_server(state).await;

    // Two: the key is sitting beside the database, so nothing is lost and
    // nothing may be destroyed.
    let refused = waveflow_server::cli::key(
        &config,
        waveflow_server::cli::KeyCommand::Adopt(waveflow_server::cli::KeyArgs { confirm: true }),
    )
    .await;
    assert!(
        refused_with(&refused, "key rotate"),
        "adoption did not send the operator to rotation: {refused:?}"
    );

    // Three: the key present is some other installation's.
    std::fs::write(&config.instance_key_path, [3u8; 32]).unwrap();
    let refused = waveflow_server::cli::key(
        &config,
        waveflow_server::cli::KeyCommand::Rotate(waveflow_server::cli::KeyArgs { confirm: true }),
    )
    .await;
    assert!(
        refused_with(&refused, "not the one it was bound to"),
        "a foreign key was accepted for rotation: {refused:?}"
    );

    std::fs::write(&config.instance_key_path, &key_before).unwrap();
    let state = waveflow_server::initialize(&config).await.unwrap();
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM subsonic_credential").await,
        SEALED_CREDENTIALS as i64,
        "a refused key change still changed something"
    );
}

/// Without `--confirm` the command is a reading, and leaves no trace.
#[tokio::test]
async fn a_key_change_without_confirmation_changes_nothing() {
    let (_temp, config, state) = test_app().await;
    let sealed = sealed_installation(&config, &state, "dry-run-owner").await;
    let key_before = std::fs::read(&config.instance_key_path).unwrap();
    stop_server(state).await;

    waveflow_server::cli::key(
        &config,
        waveflow_server::cli::KeyCommand::Rotate(waveflow_server::cli::KeyArgs { confirm: false }),
    )
    .await
    .unwrap();

    assert_eq!(
        std::fs::read(&config.instance_key_path).unwrap(),
        key_before,
        "a reading replaced the key"
    );
    assert!(
        !std::fs::read_dir(&config.data_dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("pre-key-")
        }),
        "a reading left a recovery directory behind"
    );

    let state = waveflow_server::initialize(&config).await.unwrap();
    assert!(
        state
            .services
            .public_share(&sealed.share_token)
            .await
            .is_ok(),
        "a reading moved the shares"
    );
    let credential = state
        .services
        .credential_by_username("dry-run-owner")
        .await
        .unwrap()
        .expect("the credential must still be there");
    assert_eq!(
        state
            .services
            .decrypt_subsonic_password(&credential)
            .ok()
            .as_deref(),
        Some(sealed.subsonic_password.as_bytes())
    );
}

/// What the operator actually reads, reached the way they reach it.
///
/// Through the binary rather than through `cli::key`, because the subject here
/// is the surface: the `key` group and its `--confirm` flag as clap parses
/// them, and the two counts printed — the one read from the tables before the
/// decision, and the one answered by the statements that carried it out. The
/// in-process tests above hold what the commands *do*; nothing in them would
/// notice `--confirm` renamed, or a count printed from the wrong field.
#[tokio::test]
async fn a_key_change_prints_what_it_costs_and_then_what_it_did() {
    let (temp, config, state) = test_app().await;
    sealed_installation(&config, &state, "printed-owner").await;
    stop_server(state).await;
    // Adoption refuses a key that still opens the database, so this is the
    // situation it is for.
    std::fs::remove_file(&config.instance_key_path).unwrap();

    let reading = cli_command()
        .current_dir(temp.path())
        .env("WAVEFLOW_DATA_DIR", &config.data_dir)
        .args(["key", "adopt"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(reading.stdout).unwrap();
    let stderr = String::from_utf8(reading.stderr).unwrap();
    assert!(reading.status.success(), "the reading failed: {stderr}");
    for expected in [
        "2 Subsonic credential(s) — deleted, password and `wfsk_` API key together",
        "1 scrobbling link(s) — deleted, and the 3 queued listen(s) behind them",
        "4 share(s) — kept, but every public URL changes",
        "Nothing has been changed. Re-run with --confirm.",
    ] {
        assert!(
            stdout.contains(expected),
            "the reading never said {expected:?}; it said: {stdout}"
        );
    }
    assert!(!config.instance_key_path.exists(), "a reading minted a key");

    let done = cli_command()
        .current_dir(temp.path())
        .env("WAVEFLOW_DATA_DIR", &config.data_dir)
        .args(["key", "adopt", "--confirm"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(done.stdout).unwrap();
    let stderr = String::from_utf8(done.stderr).unwrap();
    assert!(done.status.success(), "the adoption failed: {stderr}");
    assert!(
        stdout.contains(
            "Done. 2 Subsonic credential(s) and 1 scrobbling link(s) deleted, \
             with 3 queued listen(s); 4 share(s) re-addressed."
        ),
        "the closing line did not report the operation: {stdout}"
    );
    assert!(
        config.instance_key_path.exists(),
        "the adoption left no key behind"
    );
}

/// RFC-018: an incremental scan skips a file whose size and modification time
/// are unchanged, so the operator needs a way to have every file read again.
/// The request is stored on every library and spent by its next scan.
#[tokio::test]
async fn database_rescan_full_asks_every_library_to_read_everything() {
    let (_temp, config, state) = test_app().await;
    let owner = inserted_account(&state, "rescan-owner", "admin").await;
    let music = config.data_dir.join("rescan-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Rescan",
            &std::fs::canonicalize(&music).unwrap(),
            waveflow_server::storage::database::LibraryVisibility::Private,
            waveflow_server::auth::authentication::now_ms(),
        )
        .await
        .unwrap();
    assert!(!state.db.full_scan_requested(library).await.unwrap());

    run_cli(&state, &["database", "rescan-full"]).await.unwrap();

    assert!(state.db.full_scan_requested(library).await.unwrap());
}

/// RFC-021 decision 5 from the command line: rename, move with its scan,
/// purge the missing, and a deletion that refuses until the name is typed.
#[tokio::test]
async fn the_cli_renames_moves_purges_and_deletes_a_library() {
    let (temp, config, state) = test_app().await;
    let admin = inserted_account(&state, "lifecycle-admin", "admin").await;
    let music = config.data_dir.join("lifecycle-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Kept.wav"));
    write_test_wav_of_len(&music.join("Gone.wav"), 900);
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            admin,
            "Lifecycle",
            &root,
            waveflow_server::storage::database::LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let id = library.to_string();
    scan_once(
        &state,
        admin,
        waveflow_server::storage::catalog::LibraryRecord {
            id: library,
            name: "Lifecycle".into(),
            root_path: root.clone(),
        },
    )
    .await;

    run_cli(
        &state,
        &[
            "library",
            "rename",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--name",
            "Renamed",
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM library WHERE name = 'Renamed'"
        )
        .await,
        1
    );

    // Moved with one file fewer: the scan the move queues finds it missing.
    let moved = temp.path().join("lifecycle-moved");
    std::fs::rename(&music, &moved).unwrap();
    std::fs::remove_file(moved.join("Gone.wav")).unwrap();
    let moved_text = moved.to_string_lossy().into_owned();
    run_cli(
        &state,
        &[
            "library",
            "set-path",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--path",
            &moved_text,
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE is_available = 0").await,
        1
    );

    let unnamed = run_cli(
        &state,
        &[
            "library",
            "purge",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
        ],
    )
    .await;
    assert!(unnamed.is_err(), "a purge naming nothing must refuse");
    run_cli(
        &state,
        &[
            "library",
            "purge",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--all",
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE purged_at IS NULL").await,
        1
    );
    // Within the grace window, a purge is taken back.
    run_cli(
        &state,
        &[
            "library",
            "restore",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--all",
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM track WHERE purged_at IS NOT NULL"
        )
        .await,
        0
    );
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE is_available = 0").await,
        1
    );

    let mistyped = run_cli(
        &state,
        &[
            "library",
            "delete",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--confirm",
            "Lifecycle",
        ],
    )
    .await;
    assert!(
        mistyped.is_err(),
        "the old name must not confirm a deletion"
    );
    assert_eq!(count(&state, "SELECT COUNT(*) FROM library").await, 1);
    run_cli(
        &state,
        &[
            "library",
            "delete",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--confirm",
            "Renamed",
        ],
    )
    .await
    .unwrap();
    assert_eq!(count(&state, "SELECT COUNT(*) FROM library").await, 0);
    assert_eq!(count(&state, "SELECT COUNT(*) FROM track").await, 0);
    assert!(moved.join("Kept.wav").exists(), "a file was touched");
}
