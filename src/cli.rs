//! Administrative CLI used before the M4 web administration surface exists.

use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use sqlx::Connection;

use crate::{
    authentication::now_ms,
    catalog::LibraryRecord,
    database::{
        AccountRole, Database, KeyMaterial, LibraryRole, LibraryVisibility, Rekey, SchemaState,
    },
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
    /// Replace the instance key, with or without the old one in hand.
    ///
    /// Both run before the server opens SQLite, because the boot they exist to
    /// unblock is the one that aborts on a mismatched pair.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
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
    /// Begin a Last.fm authorisation from a machine with no browser. Prints
    /// the address to open and the token to bring back to `exchange`.
    Authorize(AuthorizeLastFmArgs),
    /// Finish one: turn the approved request token into a link.
    Exchange(ExchangeLastFmArgs),
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
    /// Which declared instance of it. Named rather than defaulted: an operator
    /// running two Maloja instances must say which one this account submits to,
    /// and a silent default would pick one for them.
    #[arg(long)]
    destination: String,
    /// Environment variable containing the token. The value never appears in
    /// argv, for the same reason the account password does not: a shell history
    /// and a process list are both readable by people this credential is not
    /// for.
    #[arg(long, default_value = "WAVEFLOW_SCROBBLE_TOKEN")]
    token_env: String,
}

/// Last.fm hands out no secret a person can paste, so `link` has nothing to be
/// given for it. These two are `link` for that recipient, split where the
/// person has to leave and come back.
#[derive(Debug, Args)]
pub struct AuthorizeLastFmArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    username: String,
    /// Which declared Last.fm instance, for the reason `link` takes one.
    #[arg(long)]
    destination: String,
}

#[derive(Debug, Args)]
pub struct ExchangeLastFmArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    username: String,
    #[arg(long)]
    destination: String,
    /// Environment variable holding the token `authorize` printed. Out of
    /// `argv` for the reason `link`'s secret is: a shell history and a process
    /// list are both readable by people this is not for, and whoever exchanges
    /// this token first is who the session ends up belonging to.
    #[arg(long, default_value = ExchangeLastFmArgs::TOKEN_ENV)]
    token_env: String,
}

impl ExchangeLastFmArgs {
    /// Named once, because `authorize` prints it and this declares it — and a
    /// default spelled in two places is a default that eventually disagrees
    /// with the instructions for using it.
    const TOKEN_ENV: &'static str = "WAVEFLOW_LASTFM_TOKEN";
}

#[derive(Debug, Args)]
pub struct UnlinkScrobbleArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    username: String,
    #[arg(long)]
    provider: String,
    /// Which declared instance of it, for the same reason `link` takes one.
    #[arg(long)]
    destination: String,
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

/// Two commands rather than one command and a flag.
///
/// They are not two ways of doing the same thing: `rotate` carries every
/// encrypted value onto the new key, `adopt` deletes the ones that can no
/// longer be read. Naming them apart is what keeps the second from being one
/// mistyped flag away from the first — and `adopt` refuses outright on a
/// database whose key is sitting next to it, since that is a rotation.
#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    /// Mint a new instance key and move every encrypted value onto it.
    Rotate(KeyArgs),
    /// Take a database over under a new key, discarding what the old one protected.
    Adopt(KeyArgs),
}

#[derive(Debug, Args)]
pub struct KeyArgs {
    /// Do the work. Without it the command reads the database, prints what the
    /// change costs, and stops.
    #[arg(long)]
    pub confirm: bool,
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
            ScrobbleCommand::Authorize(args) => authorize_lastfm(state, args).await,
            ScrobbleCommand::Exchange(args) => exchange_lastfm(state, args).await,
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
        Command::Key { .. } => {
            anyhow::bail!("a key change must run before server initialization")
        }
    }
}

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
        moved.subsonic_logins, moved.scrobble_links, moved.shares
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
        moved.subsonic_logins, moved.scrobble_links, moved.queued_listens, moved.shares
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
                material.subsonic_logins
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
                material.subsonic_logins
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
    // The API key goes to standard output on its own, and everything a person
    // reads goes to standard error. That way `set-credential … > key` yields the
    // key and nothing else, and an operator never has to lift a secret out of a
    // scrollback that then keeps it. Interactively both streams still land on
    // the same terminal, so this reads the way it always did.
    eprintln!("Subsonic credential updated for {}", args.username);
    eprintln!("API key (shown once, on standard output):");
    println!("{api_key}");
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
        .create_api_token(
            actor.id,
            // The command line holds the whole account authority: it is a
            // shell on the host, not a credential that could have been
            // narrowed.
            &[],
            &args.username,
            &args.name,
            &args.scopes,
        )
        .await?;
    // Standard output carries the token alone; see `set_credential` for why.
    eprintln!("Created API token {} for {}", record.id, args.username);
    eprintln!("Token (shown once, on standard output):");
    println!("{token}");
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
        .link_scrobble(user.id, provider, &args.destination, &secret)
        .await?;
    println!(
        "Linked {} to {}/{} (any previous authorisation there is withdrawn)",
        args.username,
        provider.as_str(),
        args.destination
    );
    Ok(())
}

/// Asks Last.fm for a request token and prints what to do with it.
///
/// The account is resolved before anything leaves this machine: a token minted
/// for a username that does not exist would be a call made for nothing, and
/// the refusal is cheaper first.
async fn authorize_lastfm(state: &AppState, args: AuthorizeLastFmArgs) -> anyhow::Result<()> {
    require_admin(&state.db, &args.actor).await?;
    state
        .db
        .account_by_username(&args.username)
        .await?
        .with_context(|| format!("account not found: {}", args.username))?;
    let approval = state
        .services
        .begin_lastfm_approval(&args.destination)
        .await?;
    // The address and the token are the same secret twice — the token is in
    // the address's query string. Both go to standard output because that is
    // where the operator reads them, and nowhere else: nothing here is traced.
    println!("Open this address as {}, and approve it:", args.username);
    println!();
    println!("  {}", approval.authorize_url);
    println!();
    println!("Then put this token in {}:", ExchangeLastFmArgs::TOKEN_ENV);
    println!();
    println!("  {}", approval.token);
    println!();
    println!("…and run:");
    println!();
    // **Deliberately not a line to paste with the token in it.** A first
    // version printed `WAVEFLOW_LASTFM_TOKEN=… waveflow-server scrobble exchange …`,
    // ready to copy — and a copied command line lands in a shell history,
    // which is one of the two things `--token-env` exists to keep this token
    // out of. Printing a recipe that undoes the flag's own reason would have
    // been worse than having no flag.
    //
    // How the variable gets set is left to the operator and their shell: a
    // silent `read`, a password manager, a file sourced and deleted. This
    // server has no business choosing among those, and it runs on three
    // platforms whose answers differ.
    println!(
        "  waveflow-server scrobble exchange --actor {} --username {} --destination {}",
        args.actor, args.username, args.destination
    );
    println!();
    println!("The token is Last.fm's and expires on their clock; this server wrote nothing down.");
    Ok(())
}

/// Turns an approved request token into a link.
async fn exchange_lastfm(state: &AppState, args: ExchangeLastFmArgs) -> anyhow::Result<()> {
    require_admin(&state.db, &args.actor).await?;
    let user = state
        .db
        .account_by_username(&args.username)
        .await?
        .with_context(|| format!("account not found: {}", args.username))?;
    let token = read_secret_env(&args.token_env)?;
    state
        .services
        .complete_lastfm_approval(user.id, &args.destination, &token)
        .await?;
    println!(
        "Linked {} to lastfm/{} (any previous authorisation there is withdrawn)",
        args.username, args.destination
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
    if state
        .services
        .unlink_scrobble(user.id, provider, &args.destination)
        .await?
    {
        println!(
            "Unlinked {} from {}/{}",
            args.username,
            provider.as_str(),
            args.destination
        );
    } else {
        println!(
            "{} had no link to {}/{}",
            args.username,
            provider.as_str(),
            args.destination
        );
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
            "{:<13} {:<16} {:<9} pending {:<5} retrying {:<5} uncertain {:<5} {}",
            link.provider.as_str(),
            link.destination,
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
