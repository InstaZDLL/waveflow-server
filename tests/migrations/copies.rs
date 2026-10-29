//! The copy taken before a migration, and a key change that reads without
//! migrating.

use super::*;

/// A key change reads without migrating, and copies before it does.
///
/// Here rather than beside the other key tests in `cli/key.rs` for the reason this
/// whole target exists: `test_app()` builds a current schema, and on a current
/// schema a reading that migrates and one that does not are the same reading.
/// Only a database *behind* the binary can tell them apart, and building one
/// is what this file already knows how to do.
///
/// Two promises, both of which only a lagging schema can hold to account:
///
/// - **a reading changes nothing**, so `key rotate` without `--confirm` must
///   leave `_sqlx_migrations` exactly as it found it — applying a migration is
///   a change, whatever else it is;
/// - **the recovery copy is the database as found**, so it must still be at
///   the old schema after the confirmed run brought the live one forward. A
///   copy taken afterwards would restore the operator to a schema they never
///   chose to run.
#[tokio::test]
async fn a_key_change_reads_without_migrating_and_copies_before_it_migrates() {
    let total = MIGRATOR.iter().count();
    let (tag, baseline) = newest_schema_behind();
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = Config::for_data_dir(temp.path().join("data"));

    let database = Database::open(&config).await.expect("open the database");
    let mut connection = database.pool().acquire().await.expect("connection");
    connection
        .ensure_migrations_table(&MIGRATOR.table_name)
        .await
        .expect("migrations table");
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version <= baseline)
    {
        connection
            .apply(&MIGRATOR.table_name, migration)
            .await
            .unwrap_or_else(|error| {
                panic!("{tag}: migration {} failed: {error}", migration.version)
            });
    }
    // The pair that schema was running under. Written by hand because the only
    // thing this test needs from it is that the two halves agree: `rotate`
    // refuses a key the database was not bound to, and that refusal is not
    // what is being measured here.
    let key = [5u8; 32];
    std::fs::write(&config.instance_key_path, key).expect("the key file");
    sqlx::query(
        "INSERT INTO instance_metadata (singleton, key_fingerprint, created_at) VALUES (1, ?, 0)",
    )
    .bind(waveflow_server::auth::security::bytes_hash(&key).as_slice())
    .execute(&mut *connection)
    .await
    .expect("bind the key");
    let behind = count(&mut connection, "SELECT COUNT(*) FROM _sqlx_migrations").await;
    assert!(
        (behind as usize) < total,
        "{tag}: the baseline applied all {total} migrations, so nothing is behind"
    );
    drop(connection);
    database.pool().close().await;
    drop(database);

    // One: the reading. It is told there is an upgrade waiting, and leaves it
    // waiting.
    waveflow_server::cli::key(
        &config,
        waveflow_server::cli::KeyCommand::Rotate(waveflow_server::cli::KeyArgs { confirm: false }),
    )
    .await
    .expect("a reading of a lagging database");
    assert_eq!(
        applied_in(&config.database_path).await,
        behind,
        "a reading applied a migration"
    );
    assert_eq!(
        std::fs::read(&config.instance_key_path).unwrap(),
        key,
        "a reading replaced the key"
    );

    // Two: the change. The copy is the database as it was found.
    waveflow_server::cli::key(
        &config,
        waveflow_server::cli::KeyCommand::Rotate(waveflow_server::cli::KeyArgs { confirm: true }),
    )
    .await
    .expect("a confirmed rotation of a lagging database");
    assert_ne!(
        std::fs::read(&config.instance_key_path).unwrap(),
        key,
        "the rotation kept the old key"
    );
    let recovery = std::fs::read_dir(&config.data_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("pre-key-"))
        })
        .expect("a recovery directory");
    assert_eq!(
        applied_in(&recovery.join("waveflow.db")).await,
        behind,
        "the recovery copy was taken after the schema had already moved"
    );
    assert_eq!(
        applied_in(&config.database_path).await as usize,
        total,
        "the rotation left migrations unapplied"
    );

    // And the pair it leaves behind is one the server opens.
    waveflow_server::initialize(&config)
        .await
        .expect("the rotated pair must boot");
}

/// The newest released schema the binary is still ahead of.
///
/// Not simply the last line of `RELEASED_SCHEMAS`: the PR that bumps the
/// version for a release adds its line with the newest migration of that
/// moment, so until the next migration lands that schema is the binary's own,
/// and a database at it has nothing to upgrade. Both tests here need a database
/// behind the binary, so they take the newest one that still is.
fn newest_schema_behind() -> (&'static str, i64) {
    let newest = MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .expect("at least one migration");
    RELEASED_SCHEMAS
        .iter()
        .rev()
        .copied()
        .find(|(_, baseline)| *baseline < newest)
        .expect("a released schema behind the binary")
}

/// A database at the newest released schema behind the binary, bound to a key
/// on disk, as an installation that is about to upgrade has it. Answers the key
/// and how many migrations the database has run.
///
/// `extra` runs on the database before it is closed, for a test that needs it
/// shaped some further way.
async fn an_installation_behind(config: &Config, extra: Option<&'static str>) -> ([u8; 32], i64) {
    let (tag, baseline) = newest_schema_behind();
    let database = Database::open(config).await.expect("open the database");
    let mut connection = database.pool().acquire().await.expect("connection");
    connection
        .ensure_migrations_table(&MIGRATOR.table_name)
        .await
        .expect("migrations table");
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version <= baseline)
    {
        connection
            .apply(&MIGRATOR.table_name, migration)
            .await
            .unwrap_or_else(|error| {
                panic!("{tag}: migration {} failed: {error}", migration.version)
            });
    }
    seed_an_installation(&mut connection).await;
    // Drawn, not written down: the test needs the two halves to agree, and a
    // key in the source would be a key in the source.
    let key: [u8; 32] = [
        *uuid::Uuid::new_v4().as_bytes(),
        *uuid::Uuid::new_v4().as_bytes(),
    ]
    .concat()
    .try_into()
    .unwrap();
    std::fs::write(&config.instance_key_path, key).expect("the key file");
    sqlx::query(
        "INSERT INTO instance_metadata (singleton, key_fingerprint, created_at) VALUES (1, ?, 0)",
    )
    .bind(waveflow_server::auth::security::bytes_hash(&key).as_slice())
    .execute(&mut *connection)
    .await
    .expect("bind the key");
    if let Some(statement) = extra {
        sqlx::query(statement)
            .execute(&mut *connection)
            .await
            .unwrap_or_else(|error| panic!("`{statement}` failed: {error}"));
    }
    let behind = count(&mut connection, "SELECT COUNT(*) FROM _sqlx_migrations").await;
    assert!(
        (behind as usize) < MIGRATOR.iter().count(),
        "{tag}: the baseline applied every migration, so nothing is behind"
    );
    drop(connection);
    database.pool().close().await;
    (key, behind)
}

/// The copy directories a boot made before migrating, newest first.
fn pre_migration_copies(config: &Config) -> Vec<std::path::PathBuf> {
    let mut copies: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(&config.data_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let taken_at = path
                .file_name()?
                .to_str()?
                .strip_prefix(waveflow_server::storage::migration_backup::PREFIX)?
                .parse::<u64>()
                .ok()?;
            Some((taken_at, path))
        })
        .collect();
    copies.sort_by_key(|(taken_at, _)| std::cmp::Reverse(*taken_at));
    copies.into_iter().map(|(_, path)| path).collect()
}

/// RFC-019 decision 3: an upgrade copies the database **as it found it**, with
/// the key it was bound to, keeps the newest copies only, touches nothing it
/// did not make — and a boot with nothing to apply copies nothing.
#[tokio::test]
async fn an_upgrade_copies_the_database_and_its_key_before_migrating() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = Config::for_data_dir(temp.path().join("data"));
    let (key, behind) = an_installation_behind(&config, None).await;
    // Four older copies, and two directories that only look like one.
    for stale in ["1", "2", "3", "4"] {
        std::fs::create_dir_all(config.data_dir.join(format!(
            "{}{stale}",
            waveflow_server::storage::migration_backup::PREFIX
        )))
        .unwrap();
    }
    let lookalike = config.data_dir.join(format!(
        "{}notes",
        waveflow_server::storage::migration_backup::PREFIX
    ));
    let restore = config.data_dir.join("pre-restore-5");
    std::fs::create_dir_all(&lookalike).unwrap();
    std::fs::create_dir_all(&restore).unwrap();

    let state = waveflow_server::initialize(&config)
        .await
        .expect("an installation behind boots");
    state.db.pool().close().await;
    drop(state);

    let copies = pre_migration_copies(&config);
    assert_eq!(
        copies.len(),
        config.migration_backups,
        "kept {copies:?}, expected the newest {}",
        config.migration_backups
    );
    let newest = &copies[0];
    assert_eq!(
        applied_in(&newest.join("waveflow.db")).await,
        behind,
        "the copy was taken after the schema had moved"
    );
    assert_eq!(
        std::fs::read(newest.join("instance.key")).unwrap(),
        key,
        "the copy does not carry the key its database is bound to"
    );
    assert!(
        Database::check_file_instance_key(
            &newest.join("waveflow.db"),
            &waveflow_server::auth::security::bytes_hash(&key)
        )
        .await
        .unwrap(),
        "the copied pair does not agree"
    );
    let prefix = waveflow_server::storage::migration_backup::PREFIX;
    for gone in ["1", "2"] {
        assert!(!config.data_dir.join(format!("{prefix}{gone}")).exists());
    }
    for kept in ["3", "4"] {
        assert!(config.data_dir.join(format!("{prefix}{kept}")).exists());
    }
    assert!(
        lookalike.exists(),
        "pruning deleted a directory it did not make"
    );
    assert!(
        restore.exists(),
        "pruning deleted a restore's recovery copy"
    );

    // Nothing left to apply: nothing copied.
    waveflow_server::initialize(&config)
        .await
        .expect("an upgraded installation boots");
    assert_eq!(pre_migration_copies(&config), copies);
}

/// A migration that fails says where the copy is, and the copy is whole.
#[tokio::test]
async fn a_failed_upgrade_names_the_copy_it_can_be_restored_from() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = Config::for_data_dir(temp.path().join("data"));
    // Recording a migration as applied is refused, so the first one pending
    // fails whatever it contains, and its transaction rolls back with it.
    let (key, behind) = an_installation_behind(
        &config,
        Some(
            "CREATE TRIGGER no_migration_lands BEFORE INSERT ON _sqlx_migrations \
             BEGIN SELECT RAISE(ABORT, 'staged failure'); END",
        ),
    )
    .await;

    let error = match waveflow_server::initialize(&config).await {
        Ok(_) => panic!("a migration that cannot apply booted"),
        Err(error) => format!("{error:#}"),
    };
    let copies = pre_migration_copies(&config);
    assert_eq!(copies.len(), 1, "{copies:?}");
    let copy = copies[0].display().to_string();
    assert!(
        error.contains(&format!("database restore --input {copy}")),
        "the failure does not say how to go back: {error}"
    );
    assert!(error.contains("v2 migration failed"), "{error}");
    assert_eq!(applied_in(&copies[0].join("waveflow.db")).await, behind);
    assert_eq!(std::fs::read(copies[0].join("instance.key")).unwrap(), key);
}

/// A copy that cannot be made stops the boot before anything is applied, and
/// leaves no half-made directory for pruning to count as a whole one.
#[tokio::test]
async fn a_copy_that_cannot_be_made_leaves_nothing_behind() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let mut config = Config::for_data_dir(temp.path().join("data"));
    let (_, behind) = an_installation_behind(&config, None).await;
    // The database copies; the key cannot, because a directory stands where
    // the key should be.
    config.instance_key_path = config.data_dir.join("key-is-a-directory");
    std::fs::create_dir_all(&config.instance_key_path).unwrap();

    let error = match waveflow_server::initialize(&config).await {
        Ok(_) => panic!("a boot without its copy went ahead"),
        Err(error) => format!("{error:#}"),
    };
    assert!(error.contains("could not copy the instance key"), "{error}");
    assert!(
        pre_migration_copies(&config).is_empty(),
        "an incomplete copy was left behind: {:?}",
        pre_migration_copies(&config)
    );
    assert_eq!(
        applied_in(&config.database_path).await,
        behind,
        "a migration ran without its copy"
    );
}

/// A database bound to a key is copied only with that key: missing or wrong,
/// the pair would be no way back, so the boot refuses before migrating — and
/// before minting a key of its own beside a database that has one. A
/// database never bound to a key is still copied alone; the released-schema
/// test in `upgrades.rs` stands on that.
#[tokio::test]
async fn a_bound_database_is_not_copied_without_its_own_key() {
    for (case, damage) in [
        ("missing", None),
        ("wrong", Some(*uuid::Uuid::new_v4().as_bytes())),
    ] {
        let temp = tempfile::tempdir().expect("temporary directory");
        let config = Config::for_data_dir(temp.path().join("data"));
        let (_, behind) = an_installation_behind(&config, None).await;
        match damage {
            None => std::fs::remove_file(&config.instance_key_path).unwrap(),
            Some(other) => {
                std::fs::write(&config.instance_key_path, [other, other].concat()).unwrap()
            }
        }

        let error = match waveflow_server::initialize(&config).await {
            Ok(_) => panic!("{case}: a boot with no matching key went ahead"),
            Err(error) => format!("{error:#}"),
        };
        assert!(
            error.contains("nothing has been applied"),
            "{case}: {error}"
        );
        assert!(
            pre_migration_copies(&config).is_empty(),
            "{case}: a pair restore would refuse was kept: {:?}",
            pre_migration_copies(&config)
        );
        assert_eq!(
            applied_in(&config.database_path).await,
            behind,
            "{case}: migrated a database whose key is not there"
        );
        if damage.is_none() {
            assert!(error.contains("key adopt"), "{error}");
            assert!(
                !config.instance_key_path.exists(),
                "a key was minted beside a database bound to another"
            );
        }
    }
}

/// The copy a boot has just made is kept even when its name sorts below the
/// others — a clock that went back, or copies dated in the future.
#[tokio::test]
async fn the_copy_just_made_survives_a_clock_that_went_back() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = Config::for_data_dir(temp.path().join("data"));
    let (_, behind) = an_installation_behind(&config, None).await;
    let prefix = waveflow_server::storage::migration_backup::PREFIX;
    // Three copies dated long after anything this boot can read off its clock.
    let future = ["99999999999991", "99999999999992", "99999999999993"];
    for stamp in future {
        std::fs::create_dir_all(config.data_dir.join(format!("{prefix}{stamp}"))).unwrap();
    }

    let state = waveflow_server::initialize(&config)
        .await
        .expect("an installation behind boots");
    state.db.pool().close().await;
    drop(state);

    let copies = pre_migration_copies(&config);
    assert_eq!(copies.len(), config.migration_backups, "{copies:?}");
    let made = copies
        .iter()
        .find(|copy| copy.join("waveflow.db").exists())
        .unwrap_or_else(|| panic!("the copy this boot made was deleted: {copies:?}"));
    assert_eq!(applied_in(&made.join("waveflow.db")).await, behind);
    // The room it needed came from the oldest of the others.
    assert!(!config
        .data_dir
        .join(format!("{prefix}{}", future[0]))
        .exists());
    for kept in &future[1..] {
        assert!(config.data_dir.join(format!("{prefix}{kept}")).exists());
    }
}

/// Zero copies is a setting: an operator with backups of their own upgrades
/// without one.
#[tokio::test]
async fn no_copy_is_taken_when_the_operator_set_none() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let mut config = Config::for_data_dir(temp.path().join("data"));
    config.migration_backups = 0;
    an_installation_behind(&config, None).await;
    waveflow_server::initialize(&config)
        .await
        .expect("an installation behind boots");
    assert!(pre_migration_copies(&config).is_empty());

    // And a fresh install, with copies on, has nothing to copy.
    let fresh = tempfile::tempdir().expect("temporary directory");
    let config = Config::for_data_dir(fresh.path().join("data"));
    waveflow_server::initialize(&config)
        .await
        .expect("a fresh install boots");
    assert!(pre_migration_copies(&config).is_empty());
}

/// How many migrations a database on disk has run.
///
/// Opened alone and closed again between readings: the key commands take the
/// file exclusively, and a connection this test forgot to close would make the
/// next one fail for a reason that has nothing to do with what it asserts.
async fn applied_in(path: &std::path::Path) -> i64 {
    let mut connection = Database::open_alone(path, std::time::Duration::from_secs(5))
        .await
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let applied = count(&mut connection, "SELECT COUNT(*) FROM _sqlx_migrations").await;
    connection.close().await.expect("close");
    applied
}
