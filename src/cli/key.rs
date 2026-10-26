//! `key rotate` and `key adopt`: replacing the instance key, with or without
//! the old one in hand, after copying the pair aside.

use std::path::{Path, PathBuf};

use anyhow::Context;
use sqlx::Connection;

use super::{KeyArgs, KeyCommand};
use crate::{
    authentication::now_ms,
    database::{Database, KeyMaterial, Rekey, SchemaState},
    security,
};

pub async fn key(config: &crate::Config, command: KeyCommand) -> anyhow::Result<()> {
    match command {
        KeyCommand::Rotate(args) => rotate_key(config, args).await,
        KeyCommand::Adopt(args) => adopt_key(config, args).await,
    }
}

/// The key file as it stands, or nothing where there is no file.
///
/// Unvalidated on purpose. `rotate` has to open the thing and answers for it
/// itself; `adopt` only ever compares a fingerprint, and a file of the wrong
/// length cannot match one. Refusing it here would turn away exactly the
/// operator `adopt` exists for — the one holding something that is no longer
/// a key.
fn read_instance_key(config: &crate::Config) -> anyhow::Result<Option<Vec<u8>>> {
    match std::fs::read(&config.instance_key_path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Which of the two commands is speaking.
///
/// Wording only. What each one *does* is the `Rekey` it hands to
/// [`install_new_key`]; this decides nothing, and exists so the two inventories
/// sit side by side instead of drifting apart in two copies.
#[derive(Clone, Copy)]
enum Intent {
    Rotate,
    Adopt,
}

/// The database, held alone — and not migrated.
///
/// The migration belongs to the half that has been confirmed. A reading says
/// it changes nothing, and applying a migration is a change: it rewrites
/// `_sqlx_migrations` and whatever the migration itself does, on a database
/// the operator has not yet agreed to touch. [`Database::schema_state`] is
/// asked instead, and each command answers for what it finds.
async fn open_for_key_change(
    config: &crate::Config,
) -> anyhow::Result<(sqlx::SqliteConnection, SchemaState)> {
    let mut connection =
        Database::open_alone(&config.database_path, config.sqlite_busy_timeout).await?;
    let schema = Database::schema_state(&mut connection).await?;
    if schema == SchemaState::Empty {
        anyhow::bail!(
            "{} holds no WaveFlow schema; start the server once and it will create one",
            config.database_path.display()
        );
    }
    Ok((connection, schema))
}

async fn rotate_key(config: &crate::Config, args: KeyArgs) -> anyhow::Result<()> {
    let Some(old_bytes) = read_instance_key(config)? else {
        anyhow::bail!(
            "there is no {} to rotate from; if it is gone for good, `key adopt` takes the database over under a new one",
            config.instance_key_path.display()
        );
    };
    let old_key = security::SecretBox::from_key_bytes(&old_bytes).with_context(|| {
        format!(
            "{} is not a usable key; if the real one is gone, `key adopt` takes the database over under a new one",
            config.instance_key_path.display()
        )
    })?;
    let (mut connection, schema) = open_for_key_change(config).await?;
    if let Some(stored) = Database::stored_key_fingerprint(&mut connection).await? {
        if !security::constant_time_bytes_eq(&stored, &security::bytes_hash(&old_bytes)) {
            anyhow::bail!(
                "the key beside this database is not the one it was bound to; restore the matching pair, or `key adopt` if the real one is gone"
            );
        }
    }
    if !args.confirm {
        return report_reading(&mut connection, schema, Intent::Rotate, config).await;
    }
    let recovery = copy_the_pair_aside(config, &mut connection).await?;
    Database::migrate_on(&mut connection).await?;
    let material = Database::key_material(&mut connection).await?;
    print_inventory(Intent::Rotate, &config.database_path, &material);
    let moved = install_new_key(config, connection, Rekey::Reencrypt(&old_key)).await?;
    println!(
        "Done. {} Subsonic credential(s) and {} scrobbling link(s) moved to the new key; {} share(s) re-addressed.",
        moved.subsonic_credentials, moved.scrobble_links, moved.shares
    );
    report_done(&recovery);
    Ok(())
}

async fn adopt_key(config: &crate::Config, args: KeyArgs) -> anyhow::Result<()> {
    let present = read_instance_key(config)?;
    let (mut connection, schema) = open_for_key_change(config).await?;
    let stored = Database::stored_key_fingerprint(&mut connection).await?;
    if let (Some(bytes), Some(stored)) = (&present, &stored) {
        if security::constant_time_bytes_eq(stored, &security::bytes_hash(bytes)) {
            anyhow::bail!(
                "this database already opens with the key beside it; `key rotate` replaces it without losing anything"
            );
        }
    }
    if !args.confirm {
        return report_reading(&mut connection, schema, Intent::Adopt, config).await;
    }
    let recovery = copy_the_pair_aside(config, &mut connection).await?;
    Database::migrate_on(&mut connection).await?;
    let material = Database::key_material(&mut connection).await?;
    print_inventory(Intent::Adopt, &config.database_path, &material);
    let moved = install_new_key(config, connection, Rekey::Discard).await?;
    println!(
        "Done. {} Subsonic credential(s) and {} scrobbling link(s) deleted, with {} queued listen(s); {} share(s) re-addressed.",
        moved.subsonic_credentials, moved.scrobble_links, moved.queued_listens, moved.shares
    );
    report_done(&recovery);
    Ok(())
}

/// What the command answers when it has not been told to go ahead.
///
/// A database behind this binary gets no inventory, and that is the honest
/// answer rather than a missing feature: counting means reading tables a
/// pending migration may not have created yet, and creating them is the change
/// this half promises not to make.
async fn report_reading(
    connection: &mut sqlx::SqliteConnection,
    schema: SchemaState,
    intent: Intent,
    config: &crate::Config,
) -> anyhow::Result<()> {
    if let SchemaState::Behind(pending) = schema {
        println!(
            "{pending} migration(s) are pending on {}.",
            config.database_path.display()
        );
        println!("Applying them is a change, and this half makes none, so there is no inventory");
        println!("to give yet. Re-run with --confirm: the pair is copied aside as it stands, the");
        println!("schema is brought up to date, and only then is the key replaced.");
        return Ok(());
    }
    let material = Database::key_material(connection).await?;
    print_inventory(intent, &config.database_path, &material);
    println!("Nothing has been changed. Re-run with --confirm.");
    Ok(())
}

fn print_inventory(intent: Intent, database: &Path, material: &KeyMaterial) {
    match intent {
        Intent::Rotate => {
            println!("Rotating the instance key of {}.", database.display());
            println!();
            println!(
                "  {} Subsonic credential(s) — moved to the new key; password and API key both keep working",
                material.subsonic_credentials
            );
            println!(
                "  {} scrobbling link(s) — moved to the new key, unchanged",
                material.scrobble_links
            );
            println!(
                "  {} share(s) — kept, but every public URL changes",
                material.shares
            );
        }
        Intent::Adopt => {
            println!("Adopting {} under a new instance key.", database.display());
            println!();
            println!("The old key is gone, so nothing sealed under it can be carried over:");
            println!();
            // The API key is not encrypted — it is a SHA-256 hash, which no key
            // change touches — but it shares the row with the password that is,
            // and the columns are `NOT NULL`. So the row goes whole, and the
            // operator who reads "password" and thinks only of a password finds
            // their clients locked out too.
            println!(
                "  {} Subsonic credential(s) — deleted, password and `wfsk_` API key together; `credential set` mints a new pair to hand to each client",
                material.subsonic_credentials
            );
            println!(
                "  {} scrobbling link(s) — deleted, and the {} queued listen(s) behind them",
                material.scrobble_links, material.queued_listens
            );
            println!(
                "  {} share(s) — kept, but every public URL changes",
                material.shares
            );
            println!();
            println!("Everything else is untouched: every account and its web password, the whole");
            println!("catalogue, favourites, ratings, playlists, queues, history and bookmarks.");
        }
    }
    println!();
}

/// Puts a file on disk, rather than in the page cache.
///
/// Opened differently on the two platforms, and neither spelling works on
/// both. Windows refuses `FlushFileBuffers` on a handle with no write access
/// and answers "Access is denied" to a call only asking for durability —
/// measured, after three tests failed on it. Unix allows `fsync` on a
/// read-only descriptor, and has to be asked that way: `fs::copy` carries the
/// source's permissions over, so a hardened `instance.key` at mode 0400 makes
/// its own copy unopenable for writing, and demanding write access here would
/// turn a sensible precaution into a server that cannot rotate its key.
async fn sync_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        tokio::fs::File::open(path).await?.sync_all().await
    }
    #[cfg(not(unix))]
    {
        tokio::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .await?
            .sync_all()
            .await
    }
}

/// Puts a *directory entry* on disk — a file created, or renamed into place.
///
/// Writing a file and synchronising it says nothing about the directory that
/// now claims to hold it, and the event these two commands guard against is
/// exactly the one that does not ask politely: a machine that stops between
/// the database committing to a new key and the file holding that key
/// becoming real.
///
/// Unix only, and not an oversight: a Windows directory cannot be opened as a
/// file to be synchronised, and `fs::rename` there goes through `MoveFileEx`,
/// which is atomic but carries no equivalent promise. Nothing in this
/// repository can test any of it — a power cut is not an inversion — so it is
/// written for the property rather than for a test, and said so here.
async fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        tokio::fs::File::open(path).await?.sync_all().await?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// The pair as this command found it, before a single statement touches it.
///
/// Before the migration as well as before the key, which is the whole reason
/// it is its own step: a copy taken afterwards would be a database at a schema
/// the operator never ran, and restoring it would not undo what they asked to
/// undo. `database restore` leaves the same kind of directory behind.
async fn copy_the_pair_aside(
    config: &crate::Config,
    connection: &mut sqlx::SqliteConnection,
) -> anyhow::Result<PathBuf> {
    let recovery = config
        .data_dir
        .join(format!("pre-key-{}", chrono::Utc::now().timestamp_millis()));
    tokio::fs::create_dir_all(&recovery).await?;
    let saved = recovery.join("waveflow.db");
    let saved = saved
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("the recovery path is not valid UTF-8"))?;
    sqlx::query("VACUUM INTO ?")
        .bind(saved)
        .execute(&mut *connection)
        .await
        .map_err(|error| anyhow::anyhow!("could not write the recovery copy: {error}"))?;
    sync_file(Path::new(saved)).await?;
    if config.instance_key_path.exists() {
        let saved_key = recovery.join("instance.key");
        tokio::fs::copy(&config.instance_key_path, &saved_key).await?;
        sync_file(&saved_key).await?;
    }
    // The two files, and then the directory that is supposed to contain them.
    // This happens before anything is changed, so a failure here is a refusal
    // rather than a half-finished operation — which is the right way round for
    // the one copy the operator would reach for.
    sync_directory(&recovery).await?;
    Ok(recovery)
}

/// Moves the values, then the key file — and answers with what was moved.
///
/// Not with what was predicted a moment earlier: the inventory comes from
/// reading the tables, this comes from the statements that touched them, and a
/// line printed from the second is the only one reporting the operation rather
/// than the plan.
async fn install_new_key(
    config: &crate::Config,
    mut connection: sqlx::SqliteConnection,
    plan: Rekey<'_>,
) -> anyhow::Result<KeyMaterial> {
    // Staged rather than written in place. The key file is the last thing to
    // move: one that arrived before the commit would name a database that
    // never changed.
    let staged = config
        .data_dir
        .join(format!(".key-{}.tmp", uuid::Uuid::new_v4()));
    let new_bytes = security::SecretBox::create_instance_key(&staged)?;
    let new_key = security::SecretBox::from_key_bytes(&new_bytes)?;
    // Two files now exist in the data directory that nothing has yet promised
    // will still be there: the `pre-key-…` directory and this staged key. The
    // copy made each of *its* entries durable, and neither made durable the
    // entry naming it. So the data directory is synchronised here — after the
    // staged key exists and before the commit below, because after the commit
    // is too late: a machine stopping in that window would come back with a
    // database bound to a new key, no file holding it, and no copy to go back
    // to. Which is the whole of what this command exists to prevent, arrived
    // at by the command itself.
    sync_directory(&config.data_dir).await?;
    let moved = match Database::rekey(
        &mut connection,
        plan,
        &new_key,
        &security::bytes_hash(&new_bytes),
        now_ms(),
    )
    .await
    {
        Ok(moved) => moved,
        Err(error) => {
            tokio::fs::remove_file(&staged).await.ok();
            return Err(error);
        }
    };
    // Releases the exclusive lock and checkpoints the write-ahead log, so the
    // database is settled on disk before the other half of the pair moves.
    //
    // Past the commit above, a failure here is not a failed operation but an
    // unfinished one: the database already answers to the new key, and the
    // only thing missing is the file. So the error says where that file is,
    // rather than leaving an operator with a server that will not start and
    // nothing naming the thing that would start it.
    let unfinished = || {
        format!(
            "the database is already bound to the new key; move {} onto {} to finish",
            staged.display(),
            config.instance_key_path.display()
        )
    };
    connection.close().await.with_context(unfinished)?;
    tokio::fs::rename(&staged, &config.instance_key_path)
        .await
        .with_context(unfinished)?;
    // The rename is atomic; the directory entry it made is not durable until
    // the directory itself is synchronised. Reported rather than returned as
    // an error: the change is complete by now, and failing here would hide the
    // recovery path the caller is about to print — which is the one thing an
    // operator would need next.
    if let Some(parent) = config.instance_key_path.parent() {
        if let Err(error) = sync_directory(parent).await {
            eprintln!(
                "warning: {} is in place but its directory could not be synchronised ({error}); take a backup before anything else",
                config.instance_key_path.display()
            );
        }
    }
    Ok(moved)
}

fn report_done(recovery: &Path) {
    println!(
        "The previous pair is recoverable from {}",
        recovery.display()
    );
    println!("Start the server; it opens on the new key.");
    // The command exists because somebody lost a key. Saying this at the one
    // moment they are looking at a new one costs a line.
    println!("Then take a fresh `database backup`: the new key is as unrecoverable as the last.");
}
