//! Changing the instance key: rotating it, adopting a database without it,
//! and what each refuses.

use super::*;

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
