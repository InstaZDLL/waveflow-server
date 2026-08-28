//! Who is calling, and what their token is allowed to do.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

/// What a route needs of the credential it was called with.
///
/// Chosen at every call of [`authenticated`], which is the only way into a
/// route, so a new route cannot be written without deciding: the compiler asks
/// the question. That is the whole reason this is a parameter rather than a
/// second helper a handler may forget to call — which is exactly what happened
/// to the scope list, stored since the foundations and read by nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// Reads the caller's own catalogue and user data.
    Read,
    /// Writes on the caller's behalf: playlists, favorites, ratings, the
    /// queue, bookmarks, shares, scrobbles and scans.
    Write,
    /// Acts on the instance: accounts, libraries, memberships, credentials.
    Admin,
}

/// The scope that admits the administrative routes.
pub(super) const ADMIN_SCOPE: &str = "admin";

/// The scope that admits any mutation.
pub(super) const WRITE_SCOPE: &str = "write";

impl Access {
    /// Whether a credential carrying `scopes` may do this.
    ///
    /// An empty list is unrestricted: a session, an OAuth grant and a token
    /// issued without scopes all carry the account's full authority, so nothing
    /// that works today stops working.
    ///
    /// A non-empty list grants only what it names, and a name this server does
    /// not know grants nothing — so `catalog:read` reads and does no more,
    /// without needing a vocabulary of every possible scope. `admin` implies
    /// `write`: a credential trusted to create accounts is not usefully barred
    /// from creating a playlist, and the surprise would be the other way round.
    fn granted_by(self, scopes: &[String]) -> bool {
        if scopes.is_empty() {
            return true;
        }
        let holds = |wanted: &str| scopes.iter().any(|scope| scope == wanted);
        match self {
            Self::Read => true,
            Self::Write => holds(WRITE_SCOPE) || holds(ADMIN_SCOPE),
            Self::Admin => holds(ADMIN_SCOPE),
        }
    }
}

/// Resolves the caller and checks, in one place, that the credential may do
/// what the route is about to do.
///
/// Both halves of administrative authority live here: an active administrator,
/// on a credential that has not been narrowed away from it. A token cannot
/// promote an ordinary account, and an administrator's token is not widened by
/// whose account it belongs to.
///
/// It could widen itself, once: minting a session through the authorization
/// code flow returned one carrying the account's whole authority, whatever the
/// credential that asked. That is closed where it belongs now — the grant
/// records the caller's scopes and the session inherits them — rather than by
/// a rule this function has to know about.
pub(crate) async fn authenticated(
    state: &AppState,
    headers: &HeaderMap,
    access: Access,
) -> Result<crate::authentication::AuthUser, ApiError> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    let user = state
        .auth
        .authenticate(token)
        .await
        .map_err(ApiError::from)?;
    let role_ok = access != Access::Admin || user.role == crate::database::AccountRole::Admin;
    if role_ok && access.granted_by(&user.scopes) {
        Ok(user)
    } else {
        Err(ApiError::Forbidden)
    }
}

/// The bearer token, whatever case the client spelled the scheme in.
///
/// RFC 7235 §2.1 makes the scheme name case-insensitive, so `bearer` is as
/// valid as `Bearer`. Matching the spelling exactly turned a conforming client
/// away as unauthenticated.
pub(super) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let (scheme, token) = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("Bearer")
        .then_some(token)
        .filter(|token| !token.is_empty())
}

pub(super) async fn mutation_context(
    state: &AppState,
    headers: &HeaderMap,
    user_id: Uuid,
) -> Result<crate::sync::MutationContext, ApiError> {
    let operation_id =
        optional_uuid_header(headers, OPERATION_ID_HEADER)?.unwrap_or_else(Uuid::new_v4);
    let origin_device_id = origin_device(state, headers, user_id).await?;
    Ok(crate::sync::MutationContext {
        operation_id,
        origin_device_id,
    })
}

/// The device a caller says it is, once the server has agreed it is theirs.
///
/// Split out of [`mutation_context`] because library changes need the device
/// and have no use for an operation id: nothing about a scan or a correction is
/// replayed, so minting one would be inventing a fact. The ownership check is
/// the part that matters and it is shared — an unchecked header would let one
/// account attribute its writes to another account's device, and every client
/// filtering its own changes out of the feed would then drop somebody else's.
pub(super) async fn origin_device(
    state: &AppState,
    headers: &HeaderMap,
    user_id: Uuid,
) -> Result<Option<Uuid>, ApiError> {
    let origin_device_id = optional_uuid_header(headers, DEVICE_ID_HEADER)?;
    if let Some(device_id) = origin_device_id {
        let owned = state
            .sync
            .device_belongs_to_user(user_id, device_id)
            .await
            .map_err(db_error)?;
        if !owned {
            return Err(ApiError::Validation);
        }
    }
    Ok(origin_device_id)
}

pub(super) fn optional_uuid_header(
    headers: &HeaderMap,
    name: &'static str,
) -> Result<Option<Uuid>, ApiError> {
    headers
        .get(name)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or(ApiError::Validation)
        })
        .transpose()
}
