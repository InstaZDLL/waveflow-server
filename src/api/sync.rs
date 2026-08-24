//! The sync journal: changes, snapshot, acknowledgement and the socket.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize)]
pub struct SyncQuery {
    pub after: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SyncAckRequest {
    pub device_id: Uuid,
    pub cursor: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SyncSnapshot {
    pub cursor: i64,
    pub playlists: Vec<crate::services::PlaylistItem>,
    pub favorites: Vec<StarredEntry>,
    pub ratings: Vec<crate::services::RatingItem>,
    pub queue: Option<crate::services::QueueItem>,
    pub history: Vec<crate::services::HistoryItem>,
    pub shares: Vec<ShareResponse>,
    pub bookmarks: Vec<crate::services::BookmarkItem>,
}

#[utoipa::path(
    get,
    path = "/api/v2/sync/changes",
    tag = "sync",
    params(("after" = Option<i64>, Query), ("limit" = Option<i64>, Query)),
    responses(
        (status = 200, body = crate::sync::SyncPage),
        (status = 401, body = ErrorResponse),
        (
            status = 409,
            description = "`code` is `cursor_expired`: the cursor precedes the oldest \
                           retained event, so the gap cannot be replayed. Discard the local \
                           projection, take a fresh /sync/snapshot and resume from its \
                           cursor. Distinct from `conflict`, which is about operation ids.",
            body = ErrorResponse
        ),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn sync_changes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SyncQuery>,
) -> Result<Json<crate::sync::SyncPage>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let after = query.after.unwrap_or(0);
    let limit = query.limit.unwrap_or(crate::sync::DEFAULT_SYNC_LIMIT);
    if after < 0 || limit <= 0 || limit > crate::sync::MAX_SYNC_LIMIT {
        return Err(ApiError::Validation);
    }
    state
        .sync
        .changes(user.id, after, limit)
        .await
        .map(Json)
        .map_err(sync_error)
}

#[utoipa::path(
    get,
    path = "/api/v2/sync/snapshot",
    tag = "sync",
    responses((status = 200, body = SyncSnapshot), (status = 401, body = ErrorResponse))
)]
pub async fn sync_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SyncSnapshot>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let snapshot = state
        .services
        .sync_snapshot(user.id, crate::sync::MAX_SYNC_LIMIT)
        .await
        .map_err(service_error)?;
    let favorites = snapshot
        .favorites
        .into_iter()
        .map(|(entity_type, entity_id, starred_at)| StarredEntry {
            entity_type,
            entity_id,
            starred_at,
        })
        .collect();
    let shares = snapshot
        .shares
        .into_iter()
        .map(|share| share_response(&state, share))
        .collect();
    Ok(Json(SyncSnapshot {
        cursor: snapshot.cursor,
        playlists: snapshot.playlists,
        favorites,
        ratings: snapshot.ratings,
        queue: snapshot.queue,
        history: snapshot.history,
        shares,
        bookmarks: snapshot.bookmarks,
    }))
}

#[utoipa::path(
    put,
    path = "/api/v2/sync/ack",
    tag = "sync",
    request_body = SyncAckRequest,
    responses(
        (status = 204),
        (status = 401, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn sync_ack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SyncAckRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let acknowledged = state
        .sync
        .acknowledge(user.id, request.device_id, request.cursor)
        .await
        .map_err(db_error)?;
    if !acknowledged {
        return Err(ApiError::Validation);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The socket is an edge-triggered wake-up channel. A client always follows a
/// notice with `GET /sync/changes`; the durable cursor, not socket delivery, is
/// the synchronization guarantee.
#[utoipa::path(
    get,
    path = "/api/v2/sync/socket",
    tag = "sync",
    params(("after" = Option<i64>, Query)),
    responses(
        (status = 101, description = "WebSocket cursor notifications"),
        (status = 401, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn sync_socket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SyncQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let after = query.after.unwrap_or(0);
    if after < 0 {
        return Err(ApiError::Validation);
    }
    Ok(upgrade
        .on_upgrade(move |socket| serve_sync_socket(socket, state, user.id, after))
        .into_response())
}

pub(super) async fn serve_sync_socket(
    socket: WebSocket,
    state: AppState,
    user_id: Uuid,
    after: i64,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut notices = state.sync.subscribe();
    if let Ok(cursor) = state.sync.latest_user_cursor(user_id).await {
        if cursor > after && send_sync_notice(&mut sender, cursor).await.is_err() {
            return;
        }
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let mut awaiting_pong = false;
    loop {
        tokio::select! {
            incoming = receiver.next() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(Message::Pong(_))) => awaiting_pong = false,
                Some(Ok(Message::Ping(payload))) => {
                    if sender.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(_)) => {}
            },
            notice = notices.recv() => match sync_notice_action(&state.sync, user_id, notice).await {
                Ok(SyncNoticeAction::Send(cursor)) => {
                    if send_sync_notice(&mut sender, cursor).await.is_err() {
                        break;
                    }
                }
                Ok(SyncNoticeAction::Continue) => {}
                Ok(SyncNoticeAction::Close) | Err(_) => break,
            },
            _ = heartbeat.tick() => {
                if awaiting_pong || sender.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
                awaiting_pong = true;
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SyncNoticeAction {
    Send(i64),
    Continue,
    Close,
}

pub(super) async fn sync_notice_action(
    sync: &crate::sync::SyncService,
    user_id: Uuid,
    notice: Result<(Uuid, crate::sync::SyncNotice), tokio::sync::broadcast::error::RecvError>,
) -> Result<SyncNoticeAction, sqlx::Error> {
    match notice {
        Ok((notice_user, notice)) if notice_user == user_id => {
            Ok(SyncNoticeAction::Send(notice.cursor))
        }
        Ok(_) => Ok(SyncNoticeAction::Continue),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => sync
            .latest_user_cursor(user_id)
            .await
            .map(SyncNoticeAction::Send),
        Err(tokio::sync::broadcast::error::RecvError::Closed) => Ok(SyncNoticeAction::Close),
    }
}

pub(super) async fn send_sync_notice(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    cursor: i64,
) -> Result<(), axum::Error> {
    let body =
        serde_json::to_string(&crate::sync::SyncNotice { cursor }).expect("sync notice serializes");
    sender.send(Message::Text(body.into())).await
}

#[cfg(test)]
mod tests {
    use super::{sync_notice_action, SyncNoticeAction};

    /// A lagged socket recovers the cursor from the journal, not from a default.
    ///
    /// The distinction needs a non-zero cursor to be visible at all: a user with
    /// no events answers 0, which is also what returning a constant would give,
    /// so the empty case alone proves only that the branch is wired to
    /// something. The event below is written through the real journal path, so
    /// the expected value is one the test never chose.
    #[tokio::test]
    async fn lagged_sync_socket_recovers_from_the_durable_cursor() {
        let temp = tempfile::tempdir().unwrap();
        let config = crate::Config::for_data_dir(temp.path().join("data"));
        let db = crate::database::Database::open(&config).await.unwrap();
        db.migrate().await.unwrap();
        let user_id = db
            .create_account(
                "lagged",
                "hash",
                crate::database::AccountRole::User,
                crate::authentication::now_ms(),
            )
            .await
            .unwrap();
        let sync = crate::sync::SyncService::new(db.clone());

        let context = crate::sync::MutationContext::server_generated();
        let writer = db.writer_guard().await;
        let mut tx = db.pool().begin().await.unwrap();
        sync.claim_operation(
            &writer,
            &mut tx,
            user_id,
            context,
            crate::sync::MutationIntent::new("set-rating", "track:seed", &serde_json::json!({})),
        )
        .await
        .unwrap();
        let receipt = sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "rating",
                uuid::Uuid::new_v4(),
                "upsert",
                &serde_json::json!({}),
                None,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        drop(writer);
        assert_ne!(receipt.cursor, 0);

        let action = sync_notice_action(
            &sync,
            user_id,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(3)),
        )
        .await
        .unwrap();
        assert_eq!(action, SyncNoticeAction::Send(receipt.cursor));

        // An account with nothing in the journal still falls back to the base
        // cursor rather than failing.
        let action = sync_notice_action(
            &sync,
            uuid::Uuid::new_v4(),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(3)),
        )
        .await
        .unwrap();
        assert_eq!(action, SyncNoticeAction::Send(0));
    }
}
