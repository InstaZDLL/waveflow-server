//! Accounts as the services hand them out: users, the stored Subsonic
//! credential, and a native client's authorization request.

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::security::EncryptedSecret,
    storage::database::{AccountRecord, AccountRole},
};

pub struct SubsonicCredentialRecord {
    pub account: AccountRecord,
    pub encrypted_password: EncryptedSecret,
}

/// Inputs for a native client's authorization request.
#[derive(Debug, Clone, Copy)]
pub struct AuthorizationRequest<'a> {
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub code_challenge: &'a str,
    pub code_challenge_method: &'a str,
    pub device_name: &'a str,
    pub state: Option<&'a str>,
    /// The scopes of the credential authorizing this grant. Recorded on the
    /// grant so the session redeemed from it inherits them, which is what
    /// keeps a session from ever being broader than what asked for it.
    pub scopes: &'a [String],
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserItem {
    pub id: Uuid,
    pub username: String,
    pub role: AccountRole,
    pub disabled: bool,
    pub has_subsonic_credential: bool,
    pub folder_ids: Vec<Uuid>,
}

pub struct UserUpdate<'a> {
    pub admin: Option<bool>,
    pub disabled: Option<bool>,
    pub folder_ids: Option<&'a [Uuid]>,
    pub subsonic_password: Option<&'a str>,
    pub web_password: Option<&'a str>,
}
