//! Administrative CLI used before the M4 web administration surface exists.

mod account;
mod args;
mod database;
mod key;
mod library;
mod scrobble;

pub use args::{
    AccountCommand, AddLibraryArgs, AuthorizeLastFmArgs, BackupArgs, Cli, Command,
    CreateAccountArgs, CreateTokenArgs, CredentialCommand, DatabaseCommand, DeleteLibraryArgs,
    ExchangeLastFmArgs, KeyArgs, KeyCommand, LibraryCommand, LinkScrobbleArgs, PurgeArgs,
    RemoveMemberArgs, RenameLibraryArgs, RestoreArgs, RevokeCredentialArgs, ScrobbleCommand,
    ScrobbleStatusArgs, SetCanvasArgs, SetCredentialArgs, SetLinksArgs, SetMemberArgs, SetPathArgs,
    SetUploadsArgs, SetWatchArgs, TokenCommand, UnavailableArgs, UnlinkScrobbleArgs,
};
pub use database::restore;
pub use key::key;

use anyhow::Context;

use self::account::{create_account, create_token, revoke_credential, set_credential};
use self::database::backup;
use self::library::{
    add_library, delete_library, list_unavailable, purge_unavailable, remove_member,
    rename_library, restore_purged, set_canvas, set_library_links, set_library_path,
    set_library_watch, set_member, set_uploads,
};
use self::scrobble::{
    authorize_lastfm, exchange_lastfm, link_scrobble, scrobble_status, unlink_scrobble,
};
use crate::{
    storage::database::{AccountRole, Database},
    AppState,
};

pub async fn execute(command: Command, state: &AppState) -> anyhow::Result<()> {
    let db = &state.db;
    match command {
        Command::Serve => anyhow::bail!("serve is handled by the runtime"),
        Command::Account { command } => {
            match command {
                AccountCommand::CreateAdmin(args) => {
                    create_account(db, args, AccountRole::Admin).await?
                }
                AccountCommand::CreateUser(args) => {
                    create_account(db, args, AccountRole::User).await?
                }
            }
            // An account exists now, so first-run setup is over: the token this
            // process's boot drew, or found, opens nothing and is deleted
            // (RFC-019 decision 1).
            state.setup.close();
            Ok(())
        }
        Command::Library { command } => match command {
            LibraryCommand::Add(args) => add_library(state, args).await,
            LibraryCommand::SetMember(args) => set_member(db, args).await,
            LibraryCommand::RemoveMember(args) => remove_member(db, args).await,
            LibraryCommand::SetUploads(args) => set_uploads(db, args).await,
            LibraryCommand::SetCanvas(args) => set_canvas(db, args).await,
            LibraryCommand::Rename(args) => rename_library(state, args).await,
            LibraryCommand::SetPath(args) => set_library_path(state, args).await,
            LibraryCommand::SetLinks(args) => set_library_links(state, args).await,
            LibraryCommand::Delete(args) => delete_library(state, args).await,
            LibraryCommand::Unavailable(args) => list_unavailable(state, args).await,
            LibraryCommand::Purge(args) => purge_unavailable(state, args).await,
            LibraryCommand::Restore(args) => restore_purged(state, args).await,
            LibraryCommand::SetWatch(args) => set_library_watch(state, args).await,
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
            DatabaseCommand::RescanFull => {
                let libraries = db.request_full_scan_everywhere().await?;
                println!(
                    "The next scan of {libraries} librar{} will read every file.",
                    if libraries == 1 { "y" } else { "ies" }
                );
                Ok(())
            }
        },
        Command::Key { .. } => {
            anyhow::bail!("a key change must run before server initialization")
        }
    }
}

async fn require_admin(
    db: &Database,
    username: &str,
) -> anyhow::Result<crate::storage::database::AccountRecord> {
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
mod tests;
