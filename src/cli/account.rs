//! Accounts and the credentials minted for them: `account`, `credential` and
//! `token`.

use anyhow::Context;

use super::{
    read_secret_env, require_admin, validate_username, CreateAccountArgs, CreateTokenArgs,
    RevokeCredentialArgs, SetCredentialArgs,
};
use crate::{
    auth::authentication::now_ms,
    auth::security,
    storage::database::{AccountRole, Database},
    AppState,
};

pub(super) async fn create_account(
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

pub(super) async fn set_credential(
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

pub(super) async fn revoke_credential(
    db: &Database,
    args: RevokeCredentialArgs,
) -> anyhow::Result<()> {
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
pub(super) async fn create_token(state: &AppState, args: CreateTokenArgs) -> anyhow::Result<()> {
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
