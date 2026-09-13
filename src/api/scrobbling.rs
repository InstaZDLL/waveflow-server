//! Where an account links a destination, and answers for what could not be sent.
//!
//! RFC-010 decision 9: a destination's base URL belongs to the deployment and
//! lives in `src/config.rs`, but an *authorisation* belongs to a person, so it
//! is posed here rather than by an environment variable. The same gestures exist
//! on the CLI for an operator preparing a server without a browser — both call
//! the same `DomainServices` methods, so the two surfaces cannot drift.
//!
//! **Self-scoped, not administrative.** A dedicated Subsonic password is set by
//! an administrator *for* someone; a scrobbling link is the account's own, like
//! a bookmark or a share. `Access::Write` for the mutations, `Access::Read` for
//! the two listings.

use super::*;

/// The credential an account presents at a destination.
#[derive(Deserialize, ToSchema)]
pub struct LinkScrobbleRequest {
    /// Never read back. Like every other secret here it is replaced rather than
    /// shown, and the service seals it under the instance key.
    pub secret: String,
}

/// Written by hand so the guarantee is structural rather than circumstantial.
///
/// Nothing formats this today, which is exactly when it costs nothing to hold
/// still. Deriving `Debug` would mean the next `tracing::debug!` somebody adds
/// to this handler prints a member's token — and the previous slice of this RFC
/// spent a commit removing that same shape of leak from a startup error one file
/// away.
impl std::fmt::Debug for LinkScrobbleRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LinkScrobbleRequest")
            .field("secret", &"[redacted]")
            .finish()
    }
}

/// What a deliberate retry produced.
#[derive(Debug, Serialize, ToSchema)]
pub struct RetriedScrobbleResponse {
    /// The new entry. The ambiguous one it came from stays where it was — it
    /// must, because erasing it would falsify the only trace explaining why a
    /// duplicate exists.
    pub id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The secret does not survive being formatted.
    ///
    /// The failure message deliberately carries neither the secret nor the
    /// formatted output: if this guard were removed, that output would *be* the
    /// token, and a panic message is a log line. The previous slice of this RFC
    /// earned a CodeQL `rust/cleartext-logging` alert for exactly that, on a
    /// test whose subject was also non-disclosure.
    #[test]
    fn a_link_request_does_not_print_its_secret() {
        let request = LinkScrobbleRequest {
            secret: "a-token-nobody-should-read".to_owned(),
        };
        let shown = format!("{request:?}");
        assert!(
            !shown.contains("a-token-nobody-should-read"),
            "the debug output carried the secret"
        );
        assert!(shown.contains("[redacted]"), "the secret was not replaced");
    }
}

/// The destination named in a path, or a 422.
///
/// Parsed through `FromStr` rather than by deriving `Deserialize` on the enum:
/// the wire name, the database `CHECK`, `as_str` and `FromStr` are one fact held
/// together by a test, and a fifth spelling is exactly the drift it exists to
/// stop.
fn provider(raw: &str) -> Result<crate::services::ScrobbleProvider, ApiError> {
    crate::services::ScrobbleProvider::from_str(raw).map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/scrobble-links", tag = "scrobbling", responses((status = 200, body = [crate::services::ScrobbleLinkState]), (status = 401, body = ErrorResponse)))]
pub async fn list_scrobble_links(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::ScrobbleLinkState>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .scrobble_links(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

// Decision 4 makes the row the generation, which is why relinking inserts a new
// one instead of updating the old. Out of the `///` because utoipa publishes
// that as the operation description and "decision 4" names nothing a caller can
// look up.

/// Links a destination, replacing any authorisation this account already had
/// there.
///
/// `PUT` because the destination names the resource: presenting a second token
/// leaves one link, not two. Listens already queued stay attached to the
/// authorisation they were queued under, and are never submitted under the new
/// one.
#[utoipa::path(put, path = "/api/v2/scrobble-links/{provider}", tag = "scrobbling", params(("provider" = String, Path)), request_body = LinkScrobbleRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn link_scrobble(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(request): Json<LinkScrobbleRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .link_scrobble(user.id, provider(&name)?, &request.secret)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Unlinking a destination that is not linked succeeds: the caller asked for
/// this account to have no authorisation there, and it has none.
#[utoipa::path(delete, path = "/api/v2/scrobble-links/{provider}", tag = "scrobbling", params(("provider" = String, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn unlink_scrobble(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .unlink_scrobble(user.id, provider(&name)?)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

// Under `scrobble-queue` rather than beside `{provider}`: there, the word
// `uncertain` would sit in the same position as a destination's name and the
// two would be one segment read two ways. Kept out of the `///` block because
// utoipa publishes that verbatim as the operation description, and a caller
// does not need the routing argument.
/// Every listen whose fate nobody knows, and which is still asking.
#[utoipa::path(get, path = "/api/v2/scrobble-queue/uncertain", tag = "scrobbling", responses((status = 200, body = [crate::services::UncertainScrobble]), (status = 401, body = ErrorResponse)))]
pub async fn list_uncertain_scrobbles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::UncertainScrobble>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .uncertain_scrobbles(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Throws the entry away. The listen is never submitted, and the queue stops
/// asking about it.
#[utoipa::path(delete, path = "/api/v2/scrobble-queue/uncertain/{entry_id}", tag = "scrobbling", params(("entry_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn discard_uncertain_scrobble(
    State(state): State<AppState>,
    Path(entry_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .discard_uncertain_scrobble(user.id, entry_id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

// Everything in the `///` block below is published verbatim as this operation's
// description: utoipa maps the first paragraph to `summary` and the rest to
// `description`, so every generated client ships it. Reasoning about how the
// code came to be shaped this way belongs here, in a `//` comment nobody
// generates a client from.
//
// This route used to declare a `409` as well. Nothing could produce one: the
// service excludes anything already retried with `NOT EXISTS`, so a spent entry
// stops being findable and answers 404 — and `db_error` maps every sqlx failure
// to 503 rather than to a conflict, so even the unique index could not surface
// as one. Removing the declaration was not enough on its own, because
// `annotate_mutation_headers` injects a 409 into every `user-data` write; these
// routes carry the `scrobbling` tag instead, which is what decision 12 says they
// are — operational state that never travels through synchronisation.
// `the_scrobbling_routes_advertise_no_operation_id_protocol` holds that still.

/// Sends one ambiguous listen again, accepting that the destination may already
/// hold it. Granted once per entry.
///
/// A second attempt answers `404`, and so does an entry belonging to somebody
/// else: once the single acceptance is spent, the entry stops being findable at
/// all, and this route does not distinguish "already answered" from "not yours".
#[utoipa::path(post, path = "/api/v2/scrobble-queue/uncertain/{entry_id}/retry", tag = "scrobbling", params(("entry_id" = Uuid, Path)), responses((status = 200, body = RetriedScrobbleResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn retry_uncertain_scrobble(
    State(state): State<AppState>,
    Path(entry_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<RetriedScrobbleResponse>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .retry_uncertain_scrobble(user.id, entry_id)
        .await
        .map(|id| Json(RetriedScrobbleResponse { id }))
        .map_err(service_error)
}
