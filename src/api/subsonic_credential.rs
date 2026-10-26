//! An account's own Subsonic credential.
//!
//! The administrative twins live in [`super::users`]; `mod.rs` re-exports both,
//! so `crate::api::*` paths are unchanged.
//!
//! **Two of the three verbs, and the missing one is deliberate.** An account
//! may ask whether it holds a credential, replace it, or switch it off. It may
//! not create one: an account without a credential cannot reach the Subsonic
//! facade at all, so creating one opens a door rather than narrowing one
//! already open, and whether every account may open that door onto the server
//! is the operator's decision. Rotating or switching off a credential that
//! exists takes nothing from the operator — the door is open, and these change
//! the key or shut it.
//!
//! Why it had to exist: the dedicated Subsonic password is typed into
//! third-party clients, on phones, which is the ordinary way an application
//! password leaks. Its owner could neither change it nor stop it — both
//! services called `require_admin`, the only routes were under
//! `/api/v2/admin/users/{username}/subsonic-credential`, and the CLI is a shell
//! on the host. So an account could not answer "my key has leaked" without
//! finding an operator.

use super::*;

/// Whether this account holds a Subsonic credential.
///
/// `null` when it holds none, rather than a `404` or a `{"present": false}`.
/// A `404` would be indistinguishable from a route that is not there, and a
/// flag beside the timestamps would tell one fact twice — the body already
/// says it by being absent.
#[utoipa::path(get, path = "/api/v2/subsonic-credential", tag = "account", responses((status = 200, body = Option<crate::storage::database::SubsonicCredentialStatus>), (status = 401, body = ErrorResponse)))]
pub async fn own_subsonic_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Option<crate::storage::database::SubsonicCredentialStatus>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .own_subsonic_credential(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Replaces this account's Subsonic password and API key, and returns the new
/// key once.
///
/// **Rotation, never creation**: an account holding none answers `404` and the
/// facade stays shut to it. And the key moves with the password, because a
/// leaked `wfsk_` key authenticates on its own — rotating one half would
/// answer "my key has leaked" with a new password and the same working key.
///
/// **403 for a credential that could not hand on what this returns.** A
/// `wfsk_` key carries neither scopes nor expiry, since the facade reads
/// neither, so unlike an API token it cannot be narrowed to match its caller.
/// A scoped or expiring credential is therefore refused rather than widened —
/// see [`crate::services::DomainServices::rotate_own_subsonic_credential`].
#[utoipa::path(put, path = "/api/v2/subsonic-credential", tag = "account", request_body = SetSubsonicCredentialRequest, responses((status = 200, body = SubsonicCredentialResponse), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse, description = "Forbidden. `code` is `forbidden`: the credential is scoped or expires, and the key this returns carries neither restriction, so handing one over would widen it. Retry from a signed-in session."), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn rotate_own_subsonic_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SetSubsonicCredentialRequest>,
) -> Result<Json<SubsonicCredentialResponse>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let api_key = state
        .services
        .rotate_own_subsonic_credential(
            user.id,
            &user.scopes,
            user.credential_expires_at,
            &request.password,
        )
        .await
        // 403 rather than the blanket 404, as the self-scoped token route does
        // it: nothing is being looked up when a credential is refused the
        // right to hand on more than it holds, and a 404 would tell the caller
        // its own credential does not exist.
        .map_err(|error| match error {
            crate::services::ServiceError::Forbidden => ApiError::Forbidden,
            other => service_error(other),
        })?;
    Ok(Json(SubsonicCredentialResponse { api_key }))
}

/// Switches this account's Subsonic credential off.
///
/// Bounded by nothing beyond the write access every mutation needs, where the
/// rotation above is bounded twice. The asymmetry is the point: this hands on
/// no authority, it takes some away. Refusing a narrowed credential the right
/// to close a door its own account holds would leave a compromised account
/// unable to act on the news with what it has in hand.
///
/// An account holding no credential answers `404`: it asked for something that
/// is not there, and the state it wanted already holds.
#[utoipa::path(delete, path = "/api/v2/subsonic-credential", tag = "account", responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn revoke_own_subsonic_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .revoke_own_subsonic_credential(user.id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
