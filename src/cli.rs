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
    /// Link an account to a scrobbling destination, or read its queue.
    ///
    /// The same gestures the native API offers, for an operator preparing a
    /// server nobody has opened a browser on yet — exactly the reason the
    /// dedicated Subsonic password has a command here too.
    Scrobble {
        #[command(subcommand)]
        command: ScrobbleCommand,
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
    SetUploads(SetUploadsArgs),
    SetCanvas(SetCanvasArgs),
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

/// Whether a library will receive files at all.
///
/// Off until an operator says otherwise: upgrading a server must not turn a
/// read-only installation into one that grows.
#[derive(Debug, Args)]
pub struct SetUploadsArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    library_id: uuid::Uuid,
    #[arg(long)]
    accept: bool,
}

/// Off until an operator says otherwise, and separate from the upload flag on
/// purpose: a server that refuses to grow in audio may still take a few hundred
/// kilobytes of video loop, which is the most common installation there is.
#[derive(Debug, Args)]
pub struct SetCanvasArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    library_id: uuid::Uuid,
    #[arg(long)]
    accept: bool,
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

#[derive(Debug, Subcommand)]
pub enum ScrobbleCommand {
    /// Authorise an account at a destination, replacing any authorisation it
    /// already had there.
    Link(LinkScrobbleArgs),
    /// Withdraw it. Listens already queued under it stay queued under it and
    /// are never sent to whatever is linked next.
    Unlink(UnlinkScrobbleArgs),
    /// What each of an account's links is doing.
    Status(ScrobbleStatusArgs),
}

#[derive(Debug, Args)]
pub struct LinkScrobbleArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    username: String,
    /// `listenbrainz`, `maloja` or `lastfm`.
    #[arg(long)]
    provider: String,
    /// Environment variable containing the token. The value never appears in
    /// argv, for the same reason the account password does not: a shell history
    /// and a process list are both readable by people this credential is not
    /// for.
    #[arg(long, default_value = "WAVEFLOW_SCROBBLE_TOKEN")]
    token_env: String,
}

#[derive(Debug, Args)]
pub struct UnlinkScrobbleArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    username: String,
    #[arg(long)]
    provider: String,
}

#[derive(Debug, Args)]
pub struct ScrobbleStatusArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    username: String,
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
            LibraryCommand::SetUploads(args) => set_uploads(db, args).await,
            LibraryCommand::SetCanvas(args) => set_canvas(db, args).await,
        },
        Command::Credential { command } => match command {
            CredentialCommand::Set(args) => set_credential(db, &state.secret_box, args).await,
            CredentialCommand::Revoke(args) => revoke_credential(db, args).await,
        },
        Command::Token { command } => match command {
            TokenCommand::Create(args) => create_token(state, args).await,
        },
        Command::Scrobble { command } => match command {
            ScrobbleCommand::Link(args) => link_scrobble(state, args).await,
            ScrobbleCommand::Unlink(args) => unlink_scrobble(state, args).await,
            ScrobbleCommand::Status(args) => scrobble_status(state, args).await,
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

async fn set_uploads(db: &Database, args: SetUploadsArgs) -> anyhow::Result<()> {
    let actor = require_admin(db, &args.actor).await?;
    if !db
        .set_library_accepts_uploads(actor.id, args.library_id, args.accept, now_ms())
        .await?
    {
        anyhow::bail!("library not found: {}", args.library_id);
    }
    println!(
        "Library {} {} uploads",
        args.library_id,
        if args.accept { "accepts" } else { "refuses" }
    );
    Ok(())
}

async fn set_canvas(db: &Database, args: SetCanvasArgs) -> anyhow::Result<()> {
    let actor = require_admin(db, &args.actor).await?;
    if !db
        .set_library_accepts_canvas(actor.id, args.library_id, args.accept, now_ms())
        .await?
    {
        anyhow::bail!("library not found: {}", args.library_id);
    }
    println!(
        "Library {} {} canvases",
        args.library_id,
        if args.accept { "accepts" } else { "refuses" }
    );
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

/// The destination named on the command line.
///
/// Through `FromStr`, which is the same reading the API and the `CHECK`
/// constraint use. A `ValueEnum` here would be a fifth spelling of a fact that
/// already has four, and one test holds those four together.
fn scrobble_provider(raw: &str) -> anyhow::Result<crate::services::ScrobbleProvider> {
    <crate::services::ScrobbleProvider as std::str::FromStr>::from_str(raw)
        .map_err(|_| anyhow::anyhow!("unknown destination: {raw} (listenbrainz, maloja or lastfm)"))
}

/// Through `DomainServices`, never around it.
///
/// The same reasoning `create_token` carries: this mutation is also an HTTP
/// route, and a link made here must carry the validation, the sealing and the
/// generation semantics a link made there carries. Two copies of the insert
/// would not guarantee that.
async fn link_scrobble(state: &AppState, args: LinkScrobbleArgs) -> anyhow::Result<()> {
    require_admin(&state.db, &args.actor).await?;
    let user = state
        .db
        .account_by_username(&args.username)
        .await?
        .with_context(|| format!("account not found: {}", args.username))?;
    let provider = scrobble_provider(&args.provider)?;
    let secret = read_secret_env(&args.token_env)?;
    state
        .services
        .link_scrobble(user.id, provider, &secret)
        .await?;
    println!(
        "Linked {} to {} (any previous authorisation there is withdrawn)",
        args.username,
        provider.as_str()
    );
    Ok(())
}

async fn unlink_scrobble(state: &AppState, args: UnlinkScrobbleArgs) -> anyhow::Result<()> {
    require_admin(&state.db, &args.actor).await?;
    let user = state
        .db
        .account_by_username(&args.username)
        .await?
        .with_context(|| format!("account not found: {}", args.username))?;
    let provider = scrobble_provider(&args.provider)?;
    // Saying so rather than failing: the caller asked for this account to hold
    // no authorisation there, and it holds none. The API answers the same way.
    if state.services.unlink_scrobble(user.id, provider).await? {
        println!("Unlinked {} from {}", args.username, provider.as_str());
    } else {
        println!("{} had no link to {}", args.username, provider.as_str());
    }
    Ok(())
}

async fn scrobble_status(state: &AppState, args: ScrobbleStatusArgs) -> anyhow::Result<()> {
    require_admin(&state.db, &args.actor).await?;
    let user = state
        .db
        .account_by_username(&args.username)
        .await?
        .with_context(|| format!("account not found: {}", args.username))?;
    let links = state.services.scrobble_links(user.id).await?;
    if links.is_empty() {
        println!("{} has no scrobbling links", args.username);
        return Ok(());
    }
    for link in links {
        // Counters, never content: decision 12 governs what this prints exactly
        // as it governs what the route publishes.
        println!(
            "{:<13} {:<9} pending {:<5} retrying {:<5} uncertain {:<5} {}",
            link.provider.as_str(),
            link.health,
            link.pending,
            link.retrying,
            link.uncertain,
            link.last_failure.as_deref().unwrap_or("-")
        );
    }
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
    non_blank(name, value)
}

/// The rule, kept apart from the lookup so it can be exercised without one.
///
/// Judged on the trimmed value, returned untrimmed. A variable holding nothing
/// but a newline used to pass and fail three layers down as a bare "invalid
/// input", which tells the person who pasted it nothing. The value itself is
/// handed back as it was found, because this same path reads account and
/// Subsonic passwords, and silently trimming one of those would change a
/// credential that already works.
///
/// Split out because a review pointed out that the guard was testable after all
/// — this PR had claimed it could not be, on the grounds that exercising it
/// needed `std::env::set_var`, which was removed as a data race. It needed no
/// environment at all; it needed the predicate to stop being welded to the
/// lookup.
fn non_blank(name: &str, value: String) -> anyhow::Result<String> {
    if value.trim().is_empty() {
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

#[cfg(test)]
mod tests {
    use super::non_blank;

    #[test]
    fn a_secret_of_whitespace_is_refused_and_the_variable_is_named() {
        for blank in ["", " ", "\n", "  \t\r\n "] {
            let refused = non_blank("WAVEFLOW_EXAMPLE_SECRET", blank.to_owned()).unwrap_err();
            assert!(
                refused.to_string().contains("WAVEFLOW_EXAMPLE_SECRET"),
                "the message has to name the variable the person must fix"
            );
        }
    }

    /// Returned as it was found, never trimmed.
    ///
    /// The same path reads account and Subsonic passwords; trimming what it
    /// hands back would silently alter a credential that already works.
    #[test]
    fn a_secret_with_padding_survives_intact() {
        let kept = non_blank("WAVEFLOW_EXAMPLE_SECRET", "  a token  ".to_owned()).unwrap();
        assert_eq!(kept, "  a token  ");
    }
}
