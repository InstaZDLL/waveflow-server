//! Authentication, and the rate limit on failed attempts.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;
use crate::auth::sign_in::{Refusal, Surface};

pub(super) async fn authenticate(
    state: &AppState,
    params: &Params,
    address: &str,
) -> Result<Principal, ProtocolError> {
    let guard = state.auth.sign_in_guard();
    // An API key is the secret itself, a random 256-bit token: it is looked
    // up by its hash and never throttled, so a stranger's failures behind the
    // same address never turn a working client away (RFC-017 decision 1).
    if let Some(api_key) = params.first("apiKey") {
        let credential = state
            .services
            .credential_by_api_key(api_key)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                guard.note(Surface::Subsonic, Refusal::WrongCredentials);
                auth_error()
            })?;
        return Ok(principal(credential));
    }

    let username = params.first("u").ok_or_else(missing)?;
    let Ok(reservation) =
        guard.reserve(address, &crate::auth::sign_in::canonical_account(username))
    else {
        guard.note(Surface::Subsonic, Refusal::Throttled);
        return Err(auth_error());
    };
    let credential = match state.services.credential_by_username(username).await {
        Ok(credential) => credential,
        Err(error) => {
            reservation.release();
            return Err(internal(error));
        }
    };
    let Some(credential) = credential else {
        guard.note(Surface::Subsonic, Refusal::WrongCredentials);
        return Err(auth_error());
    };
    let password = match state.services.decrypt_subsonic_password(&credential) {
        Ok(password) => password,
        Err(error) => {
            reservation.release();
            return Err(internal(error));
        }
    };
    let valid = if let (Some(token), Some(salt)) = (params.first("t"), params.first("s")) {
        let mut digest = Md5::new();
        digest.update(&password);
        digest.update(salt.as_bytes());
        let expected = hex::encode(digest.finalize());
        security::constant_time_bytes_eq(token.as_bytes(), expected.as_bytes())
    } else if let Some(provided) = params.first("p") {
        match provided.strip_prefix("enc:").map(hex::decode).transpose() {
            Ok(value) => {
                let decoded = value.unwrap_or_else(|| provided.as_bytes().to_vec());
                security::constant_time_bytes_eq(&decoded, &password)
            }
            Err(_) => false,
        }
    } else {
        false
    };
    if !valid {
        guard.note(Surface::Subsonic, Refusal::WrongCredentials);
        return Err(auth_error());
    }
    reservation.release();
    Ok(principal(credential))
}

fn principal(credential: crate::services::SubsonicCredentialRecord) -> Principal {
    Principal {
        id: credential.account.id,
        username: credential.account.username,
        role: credential.account.role,
    }
}

pub(super) fn decode_credential_password(value: &str) -> Result<String, ProtocolError> {
    let bytes = match value.strip_prefix("enc:") {
        Some(encoded) => hex::decode(encoded).map_err(|_| invalid("Invalid password encoding"))?,
        None => value.as_bytes().to_vec(),
    };
    String::from_utf8(bytes).map_err(|_| invalid("Invalid password encoding"))
}
