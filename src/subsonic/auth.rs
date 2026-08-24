//! Authentication, and the rate limit on failed attempts.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

pub(super) async fn authenticate(
    state: &AppState,
    params: &Params,
) -> Result<Principal, ProtocolError> {
    let rate_key = params
        .first("apiKey")
        .or_else(|| params.first("u"))
        .unwrap_or("missing");
    let rate_hash = hex::encode(security::token_hash(rate_key));
    if auth_rate_limited(&rate_hash) {
        return Err(ProtocolError {
            code: 40,
            message: "Wrong username or password",
        });
    }

    let credential = if let Some(api_key) = params.first("apiKey") {
        state
            .services
            .credential_by_api_key(api_key)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                record_auth_failure(&rate_hash);
                auth_error()
            })?
    } else {
        let username = params.first("u").ok_or_else(missing)?;
        state
            .services
            .credential_by_username(username)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                record_auth_failure(&rate_hash);
                auth_error()
            })?
    };
    let password = state
        .services
        .decrypt_subsonic_password(&credential)
        .map_err(internal)?;
    if params.first("apiKey").is_none() {
        let valid = if let (Some(token), Some(salt)) = (params.first("t"), params.first("s")) {
            let mut digest = Md5::new();
            digest.update(&password);
            digest.update(salt.as_bytes());
            let expected = hex::encode(digest.finalize());
            security::constant_time_bytes_eq(token.as_bytes(), expected.as_bytes())
        } else if let Some(provided) = params.first("p") {
            let decoded = match provided.strip_prefix("enc:").map(hex::decode).transpose() {
                Ok(value) => value.unwrap_or_else(|| provided.as_bytes().to_vec()),
                Err(_) => {
                    record_auth_failure(&rate_hash);
                    return Err(auth_error());
                }
            };
            security::constant_time_bytes_eq(&decoded, &password)
        } else {
            false
        };
        if !valid {
            record_auth_failure(&rate_hash);
            return Err(auth_error());
        }
    }
    clear_auth_failures(&rate_hash);
    Ok(Principal {
        id: credential.account.id,
        username: credential.account.username,
        role: credential.account.role,
    })
}

pub(super) fn auth_rate_limited(key: &str) -> bool {
    let now = Instant::now();
    let windows = AUTH_WINDOWS.get_or_init(|| StdMutex::new(HashMap::new()));
    let Ok(mut windows) = windows.lock() else {
        return false;
    };
    prune_auth_windows(&mut windows, now);
    let attempts = windows.entry(key.to_owned()).or_default();
    attempts.len() >= AUTH_ATTEMPTS_PER_MINUTE
}

pub(super) fn record_auth_failure(key: &str) {
    let windows = AUTH_WINDOWS.get_or_init(|| StdMutex::new(HashMap::new()));
    if let Ok(mut windows) = windows.lock() {
        let now = Instant::now();
        prune_auth_windows(&mut windows, now);
        if !windows.contains_key(key) && windows.len() >= MAX_AUTH_RATE_KEYS {
            if let Some(oldest) = windows
                .iter()
                .min_by_key(|(_, attempts)| attempts.back().copied())
                .map(|(key, _)| key.clone())
            {
                windows.remove(&oldest);
            }
        }
        windows.entry(key.to_owned()).or_default().push_back(now);
    }
}

pub(super) fn prune_auth_windows(windows: &mut HashMap<String, VecDeque<Instant>>, now: Instant) {
    windows.retain(|_, attempts| {
        while attempts
            .front()
            .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
        {
            attempts.pop_front();
        }
        !attempts.is_empty()
    });
}

pub(super) fn clear_auth_failures(key: &str) {
    let windows = AUTH_WINDOWS.get_or_init(|| StdMutex::new(HashMap::new()));
    if let Ok(mut windows) = windows.lock() {
        windows.remove(key);
    }
}

pub(super) fn decode_credential_password(value: &str) -> Result<String, ProtocolError> {
    let bytes = match value.strip_prefix("enc:") {
        Some(encoded) => hex::decode(encoded).map_err(|_| invalid("Invalid password encoding"))?,
        None => value.as_bytes().to_vec(),
    };
    String::from_utf8(bytes).map_err(|_| invalid("Invalid password encoding"))
}
