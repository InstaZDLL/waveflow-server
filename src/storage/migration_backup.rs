//! A copy of the database, and of its key, before a boot changes the schema
//! (RFC-019 decision 3).
//!
//! Migrations are embedded and applied by the boot, so an upgrade is the moment
//! a server is most likely to be left unable to start — on a database that is
//! already somebody's, found by the person who has just updated. The operator
//! may have a backup; nothing guaranteed it was recent. This takes one that is.
//!
//! - **Only when there is something to apply.** A fresh, empty database, and
//!   one already at this binary's schema, are not copied: a copy on every boot
//!   would cost the disk for nothing and push out the copies that matter.
//! - **The database and the key together**, in the bundle layout
//!   `database backup` writes, so `database restore --input <copy>` takes it as
//!   it is. `waveflow.db` and `instance.key` are one unit: a database copied
//!   without its key restores to a server that refuses to start. So a
//!   database bound to a key that is missing, or that does not match, is
//!   refused before anything is applied; only a database never bound to a key
//!   is copied alone.
//! - **`VACUUM INTO`**, through [`Database::backup_to`], which reads one
//!   consistent snapshot, the write-ahead log included.
//! - **The last `WAVEFLOW_MIGRATION_BACKUPS`**, three by default, are kept;
//!   older copies made here are deleted, and nothing else in the data
//!   directory is touched. Zero takes none.
//! - **A copy that cannot be made stops the boot** before anything is
//!   applied, and what it had written is deleted.
//!   An upgrade that promised a way back and went ahead without one would be
//!   worse than one that said so; the operator who does not want the copy can
//!   set the count to zero.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::config::Config;
use crate::storage::database::{Database, SchemaState};

/// What every copy directory made here is named after, followed by the epoch
/// milliseconds it was taken at. Pruning recognises nothing else.
pub const PREFIX: &str = "pre-migration-";

/// Copies the database and its key when this boot is about to apply
/// migrations, and answers where the copy is.
pub async fn before_migrating(db: &Database, config: &Config) -> anyhow::Result<Option<PathBuf>> {
    if config.migration_backups == 0 {
        return Ok(None);
    }
    let state = {
        let mut connection = db.pool().acquire().await?;
        Database::schema_state(&mut connection).await?
    };
    let SchemaState::Behind(pending) = state else {
        return Ok(None);
    };
    let directory = config
        .data_dir
        .join(format!("{PREFIX}{}", crate::auth::authentication::now_ms()));
    let refused = |what: &str| {
        format!(
            "could not copy the {what} before applying {pending} migration(s) to {}; \
             nothing has been applied. Free some space, or set \
             WAVEFLOW_MIGRATION_BACKUPS=0 to upgrade without the copy",
            directory.display()
        )
    };
    // Created here, and exclusively: a directory this attempt did not create
    // is not one it may delete when it fails.
    tokio::fs::create_dir(&directory)
        .await
        .with_context(|| refused("database"))?;
    if let Err(error) = copy_pair(db, config, &directory, &refused).await {
        // A half-made copy is worse than none: pruning counts it by its name,
        // and would push out a whole copy to keep it.
        if let Err(cleanup) = tokio::fs::remove_dir_all(&directory).await {
            tracing::warn!(
                path = %directory.display(),
                error = %cleanup,
                "could not delete an incomplete pre-migration copy; delete it by hand"
            );
        }
        return Err(error);
    }
    tracing::info!(
        path = %directory.display(),
        pending,
        "copied the database and its key before applying migrations"
    );
    if let Err(error) = prune(&config.data_dir, config.migration_backups, &directory).await {
        // The copy this boot needs exists; an older one left behind costs disk,
        // not safety, and is no reason to refuse the upgrade.
        tracing::warn!(error = %error, "could not delete older pre-migration copies");
    }
    Ok(Some(directory))
}

/// The message a failed migration carries when a copy was taken before it.
pub fn restore_hint(copy: &Path) -> String {
    format!(
        "the database and key as they stood before this upgrade are in {path}; \
         to go back, run `waveflow-server database restore --input {path}` and \
         start the previous release",
        path = copy.display()
    )
}

/// The database and its key, into `directory`, as a pair `database restore`
/// will take.
///
/// A database bound to a key must have that key beside it, and the copy is
/// checked the way a restore checks it. A pair that fails that check is no
/// way back, and the boot would refuse it anyway once the migrations had run —
/// so it is refused here, before they do, leaving the schema as it was for
/// the commands that deal with a lost or wrong key.
///
/// The one database allowed no key is one never bound to any: no server has
/// finished a boot on it, so nothing in it was ever encrypted.
async fn copy_pair(
    db: &Database,
    config: &Config,
    directory: &Path,
    refused: &impl Fn(&str) -> String,
) -> anyhow::Result<()> {
    let copied_database = directory.join("waveflow.db");
    let copied_key = directory.join("instance.key");
    db.backup_to(&copied_database)
        .await
        .with_context(|| refused("database"))?;
    let bound = {
        let mut connection = db.pool().acquire().await?;
        Database::stored_key_fingerprint(&mut connection)
            .await?
            .is_some()
    };
    match tokio::fs::copy(&config.instance_key_path, &copied_key).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !bound => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => anyhow::bail!(
            "{} is bound to an instance key and there is none at {}; nothing has been \
             applied. Restore the database and key from the same backup bundle, or, if \
             the key is gone for good, run `waveflow-server key adopt`",
            config.database_path.display(),
            config.instance_key_path.display()
        ),
        Err(error) => return Err(error).with_context(|| refused("instance key")),
    }
    if bound {
        let key = tokio::fs::read(&copied_key)
            .await
            .with_context(|| refused("instance key"))?;
        if !Database::check_file_instance_key(
            &copied_database,
            &crate::auth::security::bytes_hash(&key),
        )
        .await
        .with_context(|| refused("database"))?
        {
            anyhow::bail!(
                "instance.key does not match waveflow.db; nothing has been applied. \
                 Restore the database and key from the same backup bundle"
            );
        }
    }
    Ok(())
}

/// Deletes the oldest copies made here, keeping `keep` with `current` among
/// them.
///
/// `current` is kept whatever its name says. The names are clock readings,
/// and a clock that went back — a VM with no RTC, an NTP correction — would
/// otherwise sort the copy this boot just made below older ones and delete
/// the one copy the failure message is about to point at.
async fn prune(data_dir: &Path, keep: usize, current: &Path) -> anyhow::Result<()> {
    let mut copies = Vec::new();
    let mut entries = tokio::fs::read_dir(data_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let Some(taken_at) = name
            .to_str()
            .and_then(|name| name.strip_prefix(PREFIX))
            .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|digits| digits.parse::<u64>().ok())
        else {
            continue;
        };
        if entry.path() != current && entry.file_type().await?.is_dir() {
            copies.push((taken_at, entry.path()));
        }
    }
    copies.sort_unstable_by_key(|(taken_at, _)| std::cmp::Reverse(*taken_at));
    for (_, path) in copies.into_iter().skip(keep.saturating_sub(1)) {
        tokio::fs::remove_dir_all(&path)
            .await
            .with_context(|| format!("could not delete {}", path.display()))?;
    }
    Ok(())
}
