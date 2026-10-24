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
//!   without its key restores to a server that refuses to start.
//! - **`VACUUM INTO`**, through [`Database::backup_to`], which reads one
//!   consistent snapshot, the write-ahead log included.
//! - **The last `WAVEFLOW_MIGRATION_BACKUPS`**, three by default, are kept;
//!   older copies made here are deleted, and nothing else in the data
//!   directory is touched. Zero takes none.
//! - **A copy that cannot be made stops the boot** before anything is applied.
//!   An upgrade that promised a way back and went ahead without one would be
//!   worse than one that said so; the operator who does not want the copy can
//!   set the count to zero.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::config::Config;
use crate::database::{Database, SchemaState};

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
        .join(format!("{PREFIX}{}", crate::authentication::now_ms()));
    let refused = |what: &str| {
        format!(
            "could not copy the {what} before applying {pending} migration(s) to {}; \
             nothing has been applied. Free some space, or set \
             WAVEFLOW_MIGRATION_BACKUPS=0 to upgrade without the copy",
            directory.display()
        )
    };
    db.backup_to(&directory.join("waveflow.db"))
        .await
        .with_context(|| refused("database"))?;
    match tokio::fs::copy(&config.instance_key_path, directory.join("instance.key")).await {
        Ok(_) => {}
        // A database no server has opened yet has no key beside it, and there
        // is nothing to pair the copy with.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| refused("instance key")),
    }
    tracing::info!(
        path = %directory.display(),
        pending,
        "copied the database and its key before applying migrations"
    );
    if let Err(error) = prune(&config.data_dir, config.migration_backups).await {
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

/// Deletes the oldest copies made here, keeping `keep`.
async fn prune(data_dir: &Path, keep: usize) -> anyhow::Result<()> {
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
        if entry.file_type().await?.is_dir() {
            copies.push((taken_at, entry.path()));
        }
    }
    copies.sort_unstable_by_key(|(taken_at, _)| std::cmp::Reverse(*taken_at));
    for (_, path) in copies.into_iter().skip(keep) {
        tokio::fs::remove_dir_all(&path)
            .await
            .with_context(|| format!("could not delete {}", path.display()))?;
    }
    Ok(())
}
