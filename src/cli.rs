//! Administrative CLI used before the M4 web administration surface exists.

use std::{path::PathBuf, str::FromStr};

use anyhow::Context;
use clap::{Args, Parser, Subcommand};

use crate::{
    authentication::now_ms,
    catalog::LibraryRecord,
    database::{AccountRole, Database, LibraryRole, LibraryVisibility},
    security, AppState,
};

#[derive(Debug, Parser)]
#[command(name = "waveflow-server", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the HTTP server (the default when no command is supplied).
    Serve,
    /// Manage local accounts.
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// Register libraries and manage their members.
    Library {
        #[command(subcommand)]
        command: LibraryCommand,
    },
    /// Manage per-user Subsonic credentials.
    Credential {
        #[command(subcommand)]
        command: CredentialCommand,
    },
    /// Create long-lived native API tokens.
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    /// Check the SQLite database integrity.
    Database {
        #[command(subcommand)]
        command: DatabaseCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum AccountCommand {
    /// Create the first or an additional administrator.
    CreateAdmin(CreateAccountArgs),
    /// Create a regular account.
    CreateUser(CreateAccountArgs),
}

#[derive(Debug, Args)]
pub struct CreateAccountArgs {
    #[arg(long)]
    username: String,
    /// Environment variable containing the password. The value never appears in argv.
    #[arg(long, default_value = "WAVEFLOW_ACCOUNT_PASSWORD")]
    password_env: String,
}

#[derive(Debug, Subcommand)]
pub enum LibraryCommand {
    Add(AddLibraryArgs),
    SetMember(SetMemberArgs),
    RemoveMember(RemoveMemberArgs),
}

#[derive(Debug, Args)]
pub struct AddLibraryArgs {
    #[arg(long)]
    owner: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    path: PathBuf,
    #[arg(long, default_value = "private")]
    visibility: String,
}

#[derive(Debug, Args)]
pub struct SetMemberArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    library_id: uuid::Uuid,
    #[arg(long)]
    username: String,
    #[arg(long, default_value = "listener")]
    role: String,
}

#[derive(Debug, Args)]
pub struct RemoveMemberArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    library_id: uuid::Uuid,
    #[arg(long)]
    username: String,
}

#[derive(Debug, Subcommand)]
pub enum CredentialCommand {
    Set(SetCredentialArgs),
    Revoke(RevokeCredentialArgs),
}

#[derive(Debug, Args)]
pub struct SetCredentialArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    username: String,
    /// Environment variable containing the dedicated Subsonic password.
    #[arg(long, default_value = "WAVEFLOW_SUBSONIC_PASSWORD")]
    password_env: String,
}

#[derive(Debug, Args)]
pub struct RevokeCredentialArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    username: String,
}

#[derive(Debug, Subcommand)]
pub enum TokenCommand {
    Create(CreateTokenArgs),
}

#[derive(Debug, Args)]
pub struct CreateTokenArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    username: String,
    #[arg(long)]
    name: String,
    #[arg(long, value_delimiter = ',')]
    scopes: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum DatabaseCommand {
    Check,
    /// Create a coherent SQLite + instance-key backup bundle.
    Backup(BackupArgs),
    /// Restore a backup bundle before opening SQLite.
    Restore(RestoreArgs),
}

#[derive(Debug, Args)]
pub struct BackupArgs {
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Debug, Args)]
pub struct RestoreArgs {
    #[arg(long)]
    pub input: PathBuf,
}

pub async fn execute(command: Command, state: &AppState) -> anyhow::Result<()> {
    let db = &state.db;
    match command {
        Command::Serve => anyhow::bail!("serve is handled by the runtime"),
        Command::Account { command } => match command {
            AccountCommand::CreateAdmin(args) => create_account(db, args, AccountRole::Admin).await,
            AccountCommand::CreateUser(args) => create_account(db, args, AccountRole::User).await,
        },
        Command::Library { command } => match command {
            LibraryCommand::Add(args) => add_library(state, args).await,
            LibraryCommand::SetMember(args) => set_member(db, args).await,
            LibraryCommand::RemoveMember(args) => remove_member(db, args).await,
        },
        Command::Credential { command } => match command {
            CredentialCommand::Set(args) => set_credential(db, &state.secret_box, args).await,
            CredentialCommand::Revoke(args) => revoke_credential(db, args).await,
        },
        Command::Token { command } => match command {
            TokenCommand::Create(args) => create_token(state, args).await,
        },
        Command::Database { command } => match command {
            DatabaseCommand::Check => {
                if db.integrity_check().await? {
                    println!("SQLite integrity: ok");
                    Ok(())
                } else {
                    anyhow::bail!("SQLite integrity check failed")
                }
            }
            DatabaseCommand::Backup(args) => backup(state, args).await,
            DatabaseCommand::Restore(_) => {
                anyhow::bail!("restore must run before server initialization")
            }
        },
    }
}

async fn backup(state: &AppState, args: BackupArgs) -> anyhow::Result<()> {
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

async fn create_account(
    db: &Database,
    args: CreateAccountArgs,
    role: AccountRole,
) -> anyhow::Result<()> {
    validate_username(&args.username)?;
    let password = read_secret_env(&args.password_env)?;
    let password_hash = tokio::task::spawn_blocking(move || security::hash_password(&password))
        .await
        .context("password worker failed")??;
    let id = db
        .create_account(&args.username, &password_hash, role, now_ms())
        .await
        .context("create account")?;
    println!("Created {} account {} ({id})", role.as_str(), args.username);
    Ok(())
}

async fn add_library(state: &AppState, args: AddLibraryArgs) -> anyhow::Result<()> {
    let owner = state
        .db
        .account_by_username(&args.owner)
        .await?
        .with_context(|| format!("account not found: {}", args.owner))?;
    let metadata = std::fs::symlink_metadata(&args.path)
        .with_context(|| format!("library path is unavailable: {}", args.path.display()))?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!("library root cannot be a symbolic link");
    }
    if !metadata.is_dir() {
        anyhow::bail!("library path must be a directory");
    }
    let canonical = std::fs::canonicalize(&args.path)?;
    let visibility = LibraryVisibility::from_str(&args.visibility)?;
    let id = state
        .db
        .create_library(owner.id, &args.name, &canonical, visibility, now_ms())
        .await
        .context("register library")?;
    println!("Registered library {} ({id})", args.name);
    let scan_id = state
        .scanner
        .trigger(
            LibraryRecord {
                id,
                name: args.name,
                root_path: canonical,
            },
            Some(owner.id),
            "library_added",
        )
        .await?;
    loop {
        let job = state
            .db
            .scan_job_for_user(owner.id, scan_id)
            .await?
            .context("new library scan disappeared")?;
        match job.status.as_str() {
            "completed" => {
                println!(
                    "Initial scan complete: {} added, {} errors",
                    job.added, job.errors
                );
                break;
            }
            "failed" => anyhow::bail!(
                "initial scan failed: {}",
                job.message.unwrap_or_else(|| "unknown error".into())
            ),
            _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
    Ok(())
}

async fn set_member(db: &Database, args: SetMemberArgs) -> anyhow::Result<()> {
    let actor = require_admin(db, &args.actor).await?;
    let member = db
        .account_by_username(&args.username)
        .await?
        .with_context(|| format!("account not found: {}", args.username))?;
    let role = LibraryRole::from_str(&args.role)?;
    if role == LibraryRole::Owner {
        anyhow::bail!("library ownership cannot be transferred with set-member");
    }
    db.add_library_member(actor.id, args.library_id, member.id, role, now_ms())
        .await?;
    println!("Updated member {} on {}", args.username, args.library_id);
    Ok(())
}

async fn remove_member(db: &Database, args: RemoveMemberArgs) -> anyhow::Result<()> {
    let actor = require_admin(db, &args.actor).await?;
    let member = db
        .account_by_username(&args.username)
        .await?
        .with_context(|| format!("account not found: {}", args.username))?;
    if !db
        .remove_library_member(actor.id, args.library_id, member.id, now_ms())
        .await?
    {
        anyhow::bail!("membership not found or library owner cannot be removed");
    }
    println!("Removed member {} from {}", args.username, args.library_id);
    Ok(())
}

async fn set_credential(
    db: &Database,
    secret_box: &security::SecretBox,
    args: SetCredentialArgs,
) -> anyhow::Result<()> {
    let actor = require_admin(db, &args.actor).await?;
    let user = db
        .account_by_username(&args.username)
        .await?
        .with_context(|| format!("account not found: {}", args.username))?;
    let password = read_secret_env(&args.password_env)?;
    if password.len() < 12 {
        anyhow::bail!("Subsonic password must contain at least 12 characters");
    }
    let encrypted = secret_box.encrypt(password.as_bytes())?;
    let api_key = security::generate_token("wfsk_");
    let api_key_hash = security::token_hash(&api_key);
    db.set_subsonic_credential(actor.id, user.id, &encrypted, &api_key_hash, now_ms())
        .await?;
    println!("Subsonic credential updated for {}", args.username);
    println!("API key (shown once): {api_key}");
    Ok(())
}

async fn revoke_credential(db: &Database, args: RevokeCredentialArgs) -> anyhow::Result<()> {
    let actor = require_admin(db, &args.actor).await?;
    let user = db
        .account_by_username(&args.username)
        .await?
        .with_context(|| format!("account not found: {}", args.username))?;
    if !db
        .revoke_subsonic_credential(actor.id, user.id, now_ms())
        .await?
    {
        anyhow::bail!("no Subsonic credential exists for {}", args.username);
    }
    println!("Revoked Subsonic credential for {}", args.username);
    Ok(())
}

/// Bootstraps a token on an instance with no administrator session yet.
///
/// Issuing one is also an HTTP route now, so this goes through the same
/// domain service rather than writing the row itself: a token minted here and
/// one minted over the API must carry the same validation and the same audit
/// trail, which is exactly what two copies of the insert would not guarantee.
async fn create_token(state: &AppState, args: CreateTokenArgs) -> anyhow::Result<()> {
    let actor = require_admin(&state.db, &args.actor).await?;
    let (record, token) = state
        .services
        .create_api_token(actor.id, &args.username, &args.name, &args.scopes)
        .await?;
    println!("Created API token {} for {}", record.id, args.username);
    println!("Token (shown once): {token}");
    Ok(())
}

async fn require_admin(
    db: &Database,
    username: &str,
) -> anyhow::Result<crate::database::AccountRecord> {
    let account = db
        .account_by_username(username)
        .await?
        .with_context(|| format!("account not found: {username}"))?;
    if account.role != AccountRole::Admin || account.disabled {
        anyhow::bail!("account is not an active administrator: {username}");
    }
    Ok(account)
}

fn read_secret_env(name: &str) -> anyhow::Result<String> {
    let value = std::env::var(name).with_context(|| format!("{name} is required"))?;
    if value.is_empty() {
        anyhow::bail!("{name} cannot be empty");
    }
    Ok(value)
}

fn validate_username(username: &str) -> anyhow::Result<()> {
    let username = username.trim();
    if !(3..=64).contains(&username.len()) {
        anyhow::bail!("username must contain between 3 and 64 characters");
    }
    if !username
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        anyhow::bail!("username may only contain letters, numbers, '.', '-' and '_'");
    }
    Ok(())
}
