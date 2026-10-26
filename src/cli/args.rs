//! The command line as clap reads it: every command, subcommand and argument.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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
    pub(super) username: String,
    /// Environment variable containing the password. The value never appears in argv.
    #[arg(long, default_value = "WAVEFLOW_ACCOUNT_PASSWORD")]
    pub(super) password_env: String,
}

#[derive(Debug, Subcommand)]
pub enum LibraryCommand {
    Add(AddLibraryArgs),
    SetMember(SetMemberArgs),
    RemoveMember(RemoveMemberArgs),
    SetUploads(SetUploadsArgs),
    SetCanvas(SetCanvasArgs),
    /// Give a library a new name.
    Rename(RenameLibraryArgs),
    /// Point a library at a new root, then scan it. Tracks are matched by
    /// their path relative to the root, so a tree moved intact keeps them all.
    SetPath(SetPathArgs),
    /// Follow symbolic links whose target stays inside the library, or stop
    /// following them, then scan it.
    SetLinks(SetLinksArgs),
    /// Remove a library from the catalogue, with every track, album and
    /// artist in it and everything that hangs off them for every member.
    /// Nothing is removed from the disk.
    Delete(DeleteLibraryArgs),
    /// List the tracks whose file the scans can no longer find.
    Unavailable(UnavailableArgs),
    /// Purge missing tracks: out of sight at once, and removed from the
    /// catalogue for good once the grace window (`WAVEFLOW_PURGE_GRACE_HOURS`)
    /// has passed, or at once when it is zero.
    Purge(PurgeArgs),
    /// Take a purge back, for tracks still within the grace window.
    Restore(PurgeArgs),
    /// Watch the library for changes, or stop watching it.
    SetWatch(SetWatchArgs),
}

#[derive(Debug, Args)]
pub struct SetWatchArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
    /// `true` to watch, `false` to leave the library to the periodic scan.
    #[arg(long, action = clap::ArgAction::Set)]
    pub(super) watch: bool,
}

#[derive(Debug, Args)]
pub struct RenameLibraryArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
    #[arg(long)]
    pub(super) name: String,
}

#[derive(Debug, Args)]
pub struct SetPathArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
    #[arg(long)]
    pub(super) path: PathBuf,
}

/// Off until an operator says otherwise: without `--follow`, links are not
/// followed.
#[derive(Debug, Args)]
pub struct SetLinksArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
    #[arg(long)]
    pub(super) follow: bool,
}

#[derive(Debug, Args)]
pub struct DeleteLibraryArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
    /// The library's current name, typed out: the deletion goes ahead only
    /// when it matches.
    #[arg(long)]
    pub(super) confirm: String,
}

#[derive(Debug, Args)]
pub struct UnavailableArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
}

/// Exactly one of `--all` and `--track-id`.
#[derive(Debug, Args)]
pub struct PurgeArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
    /// Every missing track of the library.
    #[arg(long, conflicts_with = "track_id")]
    pub(super) all: bool,
    /// One missing track; repeat for several.
    #[arg(long = "track-id")]
    pub(super) track_id: Vec<uuid::Uuid>,
}

#[derive(Debug, Args)]
pub struct AddLibraryArgs {
    #[arg(long)]
    pub(super) owner: String,
    #[arg(long)]
    pub(super) name: String,
    #[arg(long)]
    pub(super) path: PathBuf,
    #[arg(long, default_value = "private")]
    pub(super) visibility: String,
}

#[derive(Debug, Args)]
pub struct SetMemberArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
    #[arg(long)]
    pub(super) username: String,
    #[arg(long, default_value = "listener")]
    pub(super) role: String,
}

/// Whether a library will receive files at all.
///
/// Off until an operator says otherwise: upgrading a server must not turn a
/// read-only installation into one that grows.
#[derive(Debug, Args)]
pub struct SetUploadsArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
    #[arg(long)]
    pub(super) accept: bool,
}

/// Off until an operator says otherwise, and separate from the upload flag on
/// purpose: a server that refuses to grow in audio may still take a few hundred
/// kilobytes of video loop, which is the most common installation there is.
#[derive(Debug, Args)]
pub struct SetCanvasArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
    #[arg(long)]
    pub(super) accept: bool,
}

#[derive(Debug, Args)]
pub struct RemoveMemberArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) library_id: uuid::Uuid,
    #[arg(long)]
    pub(super) username: String,
}

#[derive(Debug, Subcommand)]
pub enum CredentialCommand {
    Set(SetCredentialArgs),
    Revoke(RevokeCredentialArgs),
}

#[derive(Debug, Args)]
pub struct SetCredentialArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) username: String,
    /// Environment variable containing the dedicated Subsonic password.
    #[arg(long, default_value = "WAVEFLOW_SUBSONIC_PASSWORD")]
    pub(super) password_env: String,
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
    pub(super) actor: String,
    #[arg(long)]
    pub(super) username: String,
    /// `listenbrainz`, `maloja` or `lastfm`.
    #[arg(long)]
    pub(super) provider: String,
    /// Which declared instance of it. Named rather than defaulted: an operator
    /// running two Maloja instances must say which one this account submits to,
    /// and a silent default would pick one for them.
    #[arg(long)]
    pub(super) destination: String,
    /// Environment variable containing the token. The value never appears in
    /// argv, for the same reason the account password does not: a shell history
    /// and a process list are both readable by people this credential is not
    /// for.
    #[arg(long, default_value = "WAVEFLOW_SCROBBLE_TOKEN")]
    pub(super) token_env: String,
}

/// Last.fm hands out no secret a person can paste, so `link` has nothing to be
/// given for it. These two are `link` for that recipient, split where the
/// person has to leave and come back.
#[derive(Debug, Args)]
pub struct AuthorizeLastFmArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) username: String,
    /// Which declared Last.fm instance, for the reason `link` takes one.
    #[arg(long)]
    pub(super) destination: String,
}

#[derive(Debug, Args)]
pub struct ExchangeLastFmArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) username: String,
    #[arg(long)]
    pub(super) destination: String,
    /// Environment variable holding the token `authorize` printed. Out of
    /// `argv` for the reason `link`'s secret is: a shell history and a process
    /// list are both readable by people this is not for, and whoever exchanges
    /// this token first is who the session ends up belonging to.
    #[arg(long, default_value = ExchangeLastFmArgs::TOKEN_ENV)]
    pub(super) token_env: String,
}

impl ExchangeLastFmArgs {
    /// Named once, because `authorize` prints it and this declares it — and a
    /// default spelled in two places is a default that eventually disagrees
    /// with the instructions for using it.
    pub(super) const TOKEN_ENV: &'static str = "WAVEFLOW_LASTFM_TOKEN";
}

#[derive(Debug, Args)]
pub struct UnlinkScrobbleArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) username: String,
    #[arg(long)]
    pub(super) provider: String,
    /// Which declared instance of it, for the same reason `link` takes one.
    #[arg(long)]
    pub(super) destination: String,
}

#[derive(Debug, Args)]
pub struct ScrobbleStatusArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) username: String,
}

#[derive(Debug, Args)]
pub struct RevokeCredentialArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) username: String,
}

#[derive(Debug, Subcommand)]
pub enum TokenCommand {
    Create(CreateTokenArgs),
}

#[derive(Debug, Args)]
pub struct CreateTokenArgs {
    #[arg(long)]
    pub(super) actor: String,
    #[arg(long)]
    pub(super) username: String,
    #[arg(long)]
    pub(super) name: String,
    #[arg(long, value_delimiter = ',')]
    pub(super) scopes: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum DatabaseCommand {
    Check,
    /// Create a coherent SQLite + instance-key backup bundle.
    Backup(BackupArgs),
    /// Restore a backup bundle before opening SQLite.
    Restore(RestoreArgs),
    /// Read every file again on the next scan of every library, skipping
    /// nothing (RFC-018): for a file rewritten in place with its size and
    /// modification time both kept, which an ordinary scan does not open.
    RescanFull,
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
