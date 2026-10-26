//! `scrobble`: linking an account to a destination, the Last.fm approval
//! split across a browser, and reading a link's queue.

use anyhow::Context;

use super::{
    read_secret_env, require_admin, AuthorizeLastFmArgs, ExchangeLastFmArgs, LinkScrobbleArgs,
    ScrobbleStatusArgs, UnlinkScrobbleArgs,
};
use crate::AppState;

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
pub(super) async fn link_scrobble(state: &AppState, args: LinkScrobbleArgs) -> anyhow::Result<()> {
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
pub(super) async fn authorize_lastfm(
    state: &AppState,
    args: AuthorizeLastFmArgs,
) -> anyhow::Result<()> {
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
pub(super) async fn exchange_lastfm(
    state: &AppState,
    args: ExchangeLastFmArgs,
) -> anyhow::Result<()> {
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

pub(super) async fn unlink_scrobble(
    state: &AppState,
    args: UnlinkScrobbleArgs,
) -> anyhow::Result<()> {
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

pub(super) async fn scrobble_status(
    state: &AppState,
    args: ScrobbleStatusArgs,
) -> anyhow::Result<()> {
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
