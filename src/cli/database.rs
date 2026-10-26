//! `database backup` and `database restore`: the database and its key, as one
//! bundle.

use super::{BackupArgs, RestoreArgs};
use crate::{database::Database, security, AppState};

pub(super) async fn backup(state: &AppState, args: BackupArgs) -> anyhow::Result<()> {
    if args.output.exists() {
        anyhow::bail!("backup output already exists: {}", args.output.display());
    }
    tokio::fs::create_dir_all(&args.output).await?;
    let database = args.output.join("waveflow.db");
    state.db.backup_to(&database).await?;
    tokio::fs::copy(&state.instance_key_path, args.output.join("instance.key")).await?;
    if !Database::check_file(&database).await? {
        anyhow::bail!("created backup failed integrity check");
    }
    let backup_key = tokio::fs::read(args.output.join("instance.key")).await?;
    if !Database::check_file_instance_key(&database, &security::bytes_hash(&backup_key)).await? {
        anyhow::bail!("created backup database and instance.key do not match");
    }
    println!("Backup created at {}", args.output.display());
    Ok(())
}

pub async fn restore(config: &crate::Config, args: RestoreArgs) -> anyhow::Result<()> {
    let source_db = args.input.join("waveflow.db");
    let source_key = args.input.join("instance.key");
    if !Database::check_file(&source_db).await? {
        anyhow::bail!("backup SQLite integrity check failed");
    }
    let source_key_bytes = tokio::fs::read(&source_key).await?;
    if source_key_bytes.len() != 32 {
        anyhow::bail!("backup instance.key must contain exactly 32 bytes");
    }
    if !Database::check_file_instance_key(&source_db, &security::bytes_hash(&source_key_bytes))
        .await?
    {
        anyhow::bail!("backup database and instance.key do not match");
    }
    tokio::fs::create_dir_all(&config.data_dir).await?;
    let suffix = uuid::Uuid::new_v4();
    let staged_db = config.data_dir.join(format!(".restore-{suffix}.db"));
    let staged_key = config.data_dir.join(format!(".restore-{suffix}.key"));
    tokio::fs::copy(&source_db, &staged_db).await?;
    tokio::fs::copy(&source_key, &staged_key).await?;
    if !Database::check_file(&staged_db).await? {
        anyhow::bail!("staged SQLite restore failed integrity check");
    }
    let recovery = config.data_dir.join(format!(
        "pre-restore-{}",
        chrono::Utc::now().timestamp_millis()
    ));
    tokio::fs::create_dir_all(&recovery).await?;
    if config.database_path.exists() {
        tokio::fs::rename(&config.database_path, recovery.join("waveflow.db")).await?;
    }
    if config.instance_key_path.exists() {
        tokio::fs::rename(&config.instance_key_path, recovery.join("instance.key")).await?;
    }
    tokio::fs::rename(&staged_db, &config.database_path).await?;
    tokio::fs::rename(&staged_key, &config.instance_key_path).await?;
    println!(
        "Backup restored; previous files are recoverable from {}",
        recovery.display()
    );
    Ok(())
}
