//! `library`: registering libraries, changing them through the domain
//! services, managing their members, and waiting out the scans that follow.

use std::str::FromStr;

use anyhow::Context;

use super::{
    require_admin, AddLibraryArgs, DeleteLibraryArgs, PurgeArgs, RemoveMemberArgs,
    RenameLibraryArgs, SetCanvasArgs, SetLinksArgs, SetMemberArgs, SetPathArgs, SetUploadsArgs,
    SetWatchArgs, UnavailableArgs,
};
use crate::{
    auth::authentication::now_ms,
    storage::catalog::LibraryRecord,
    storage::database::{Database, LibraryRole, LibraryVisibility},
    AppState,
};

pub(super) async fn add_library(state: &AppState, args: AddLibraryArgs) -> anyhow::Result<()> {
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
    state.scanner.libraries_changed();
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

/// What a refused library operation says on a terminal. The services answer
/// in their own vocabulary; an operator needs a sentence.
fn library_refusal(error: crate::services::ServiceError, library_id: uuid::Uuid) -> anyhow::Error {
    use crate::services::ServiceError;
    match error {
        ServiceError::NotFound | ServiceError::Forbidden => {
            anyhow::anyhow!("library not found: {library_id}")
        }
        ServiceError::Conflict => anyhow::anyhow!(
            "library {library_id} is being scanned, or another library already has that path; \
             try again once the scan is over"
        ),
        ServiceError::Invalid => anyhow::anyhow!("the request was refused as invalid"),
        other => anyhow::anyhow!(other),
    }
}

/// Waits for a scan this command queued, and prints how it ended. The process
/// is about to exit, and the scan is a task of this process: leaving it would
/// kill it halfway.
async fn wait_for_scan(state: &AppState, scan_id: uuid::Uuid) -> anyhow::Result<()> {
    loop {
        let job = state
            .db
            .scan_job(scan_id)
            .await?
            .context("the scan disappeared")?;
        match job.status.as_str() {
            "completed" => {
                println!(
                    "Scan complete: {} added, {} updated, {} moved, {} missing, {} errors",
                    job.added, job.updated, job.moved, job.unavailable, job.errors
                );
                return Ok(());
            }
            "failed" => anyhow::bail!(
                "scan failed: {}",
                job.message.unwrap_or_else(|| "unknown error".into())
            ),
            _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
}

pub(super) async fn rename_library(
    state: &AppState,
    args: RenameLibraryArgs,
) -> anyhow::Result<()> {
    let actor = require_admin(&state.db, &args.actor).await?;
    state
        .services
        .update_library(
            crate::services::LibraryActor::operator(actor.id),
            args.library_id,
            crate::services::LibraryUpdate {
                name: Some(args.name.clone()),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| library_refusal(error, args.library_id))?;
    println!(
        "Library {} is now named {}",
        args.library_id,
        args.name.trim()
    );
    Ok(())
}

pub(super) async fn set_library_path(state: &AppState, args: SetPathArgs) -> anyhow::Result<()> {
    let actor = require_admin(&state.db, &args.actor).await?;
    let updated = state
        .services
        .update_library(
            crate::services::LibraryActor::operator(actor.id),
            args.library_id,
            crate::services::LibraryUpdate {
                path: Some(args.path.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| match error {
            crate::services::ServiceError::Invalid => anyhow::anyhow!(
                "library path must be an existing directory and not a symbolic link: {}",
                args.path.display()
            ),
            other => library_refusal(other, args.library_id),
        })?;
    match updated.scan_id {
        Some(scan_id) => {
            println!("Library {} moved; scanning it", args.library_id);
            wait_for_scan(state, scan_id).await
        }
        None => {
            println!("Library {} already has that path", args.library_id);
            Ok(())
        }
    }
}

pub(super) async fn set_library_links(state: &AppState, args: SetLinksArgs) -> anyhow::Result<()> {
    let actor = require_admin(&state.db, &args.actor).await?;
    let updated = state
        .services
        .update_library(
            crate::services::LibraryActor::operator(actor.id),
            args.library_id,
            crate::services::LibraryUpdate {
                follow_links: Some(args.follow),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| library_refusal(error, args.library_id))?;
    println!(
        "Library {} {} symbolic links",
        args.library_id,
        if args.follow {
            "follows"
        } else {
            "does not follow"
        }
    );
    match updated.scan_id {
        Some(scan_id) => wait_for_scan(state, scan_id).await,
        None => Ok(()),
    }
}

pub(super) async fn delete_library(
    state: &AppState,
    args: DeleteLibraryArgs,
) -> anyhow::Result<()> {
    let actor = require_admin(&state.db, &args.actor).await?;
    let deleted = state
        .services
        .delete_library(
            crate::services::LibraryActor::operator(actor.id),
            args.library_id,
            &args.confirm,
        )
        .await
        .map_err(|error| match error {
            crate::services::ServiceError::Invalid => {
                anyhow::anyhow!("--confirm must be the library's current name; nothing was deleted")
            }
            other => library_refusal(other, args.library_id),
        })?;
    println!(
        "Deleted library {}: {} tracks, {} albums, {} artists removed from the catalogue; \
         no file was touched",
        args.library_id, deleted.tracks, deleted.albums, deleted.artists
    );
    Ok(())
}

pub(super) async fn list_unavailable(
    state: &AppState,
    args: UnavailableArgs,
) -> anyhow::Result<()> {
    let actor = require_admin(&state.db, &args.actor).await?;
    let mut offset = 0;
    loop {
        let page = crate::services::BrowsePage::new(Some(offset), Some(200))
            .map_err(|error| library_refusal(error, args.library_id))?;
        let listed = state
            .services
            .unavailable_tracks(
                crate::services::LibraryActor::operator(actor.id),
                args.library_id,
                page,
            )
            .await
            .map_err(|error| library_refusal(error, args.library_id))?;
        if offset == 0 {
            println!("{} missing track(s)", listed.total);
        }
        for track in &listed.tracks {
            println!(
                "{}\t{}\t{} playlist entries, {} favourites, {} plays",
                track.id, track.relative_path, track.playlist_entries, track.favorites, track.plays
            );
        }
        if listed.tracks.len() < 200 {
            return Ok(());
        }
        offset += 200;
    }
}

pub(super) async fn purge_unavailable(state: &AppState, args: PurgeArgs) -> anyhow::Result<()> {
    let actor = require_admin(&state.db, &args.actor).await?;
    if !args.all && args.track_id.is_empty() {
        anyhow::bail!("name the tracks to purge with --track-id, or pass --all");
    }
    let outcome = state
        .services
        .purge_unavailable_tracks(
            crate::services::LibraryActor::operator(actor.id),
            args.library_id,
            crate::services::PurgeRequest {
                track_ids: (!args.all).then_some(args.track_id),
                all: args.all,
            },
        )
        .await
        .map_err(|error| library_refusal(error, args.library_id))?;
    match outcome.restorable_until {
        Some(until) => println!(
            "Purged {} missing track(s); restorable until {until} (epoch ms) with `library restore`",
            outcome.purged
        ),
        None => println!("Purged {} missing track(s)", outcome.purged),
    }
    Ok(())
}

pub(super) async fn restore_purged(state: &AppState, args: PurgeArgs) -> anyhow::Result<()> {
    let actor = require_admin(&state.db, &args.actor).await?;
    if !args.all && args.track_id.is_empty() {
        anyhow::bail!("name the tracks to restore with --track-id, or pass --all");
    }
    let outcome = state
        .services
        .restore_purged_tracks(
            crate::services::LibraryActor::operator(actor.id),
            args.library_id,
            crate::services::PurgeRequest {
                track_ids: (!args.all).then_some(args.track_id),
                all: args.all,
            },
        )
        .await
        .map_err(|error| library_refusal(error, args.library_id))?;
    println!("Restored {} purged track(s)", outcome.restored);
    Ok(())
}

pub(super) async fn set_library_watch(state: &AppState, args: SetWatchArgs) -> anyhow::Result<()> {
    let actor = require_admin(&state.db, &args.actor).await?;
    state
        .services
        .update_library(
            crate::services::LibraryActor::operator(actor.id),
            args.library_id,
            crate::services::LibraryUpdate {
                watch: Some(args.watch),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| library_refusal(error, args.library_id))?;
    println!(
        "Library {} is {}",
        args.library_id,
        if args.watch {
            "watched for changes"
        } else {
            "left to the periodic scan"
        }
    );
    Ok(())
}

pub(super) async fn set_member(db: &Database, args: SetMemberArgs) -> anyhow::Result<()> {
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

pub(super) async fn set_uploads(db: &Database, args: SetUploadsArgs) -> anyhow::Result<()> {
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

pub(super) async fn set_canvas(db: &Database, args: SetCanvasArgs) -> anyhow::Result<()> {
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

pub(super) async fn remove_member(db: &Database, args: RemoveMemberArgs) -> anyhow::Result<()> {
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
