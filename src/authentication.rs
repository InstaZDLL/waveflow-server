//! Local WaveFlow authentication service.

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    config::Config,
    database::{AccountRole, Database, NewSession},
    security,
    sign_in::{self, Refusal, SignInGuard, Surface},
};

#[derive(Clone)]
pub struct AuthService {
    db: Database,
    access_ttl_ms: i64,
    refresh_ttl_ms: i64,
    /// The limiter, gate and tally every password check goes through
    /// (RFC-017), Subsonic's included.
    guard: std::sync::Arc<SignInGuard>,
    /// What an unknown or disabled account is verified against: a hash of a
    /// password nobody knows, with the same parameters as every stored one.
    /// Made here, at startup, so the first sign-in after a restart does not pay
    /// for it — which would itself tell an unknown account from a real one.
    dummy_hash: std::sync::Arc<str>,
}

/// Who is signing in, as far as the limiter is concerned: the surface, and the
/// address bucket the request is counted under.
#[derive(Debug, Clone)]
pub struct SignInClient {
    pub surface: Surface,
    pub address: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub user: AuthUser,
    pub device_id: Uuid,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub role: AccountRole,
    /// The scopes limiting this request, from the credential it arrived on —
    /// an API token, or a session issued under a grant that carried some.
    ///
    /// Empty means unrestricted: a password login, or a token issued without
    /// any, both of which carry the account's full authority. A non-empty list is a restriction, and a route that needs
    /// more than the list grants must refuse rather than trust the account
    /// behind it. A token is handed to a script, and the point of writing
    /// scopes on it is that they hold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// When the credential this request arrived on stops working, if it is an
    /// API token with a deadline.
    ///
    /// It exists so that **a credential cannot hand on a longer life than it
    /// has**. A token expiring tomorrow could mint one expiring in a year, and
    /// that one another, so the expiry a self-minted token carries was
    /// defeatable by the very credential it bounded — the deadline was
    /// decorative after one hop.
    ///
    /// `None` for a session, deliberately. A session is short-lived and
    /// refreshes, and authorising something longer-lived than itself is part
    /// of what it is for; capping by it would make every token a person
    /// registers die in fifteen minutes.
    ///
    /// Never serialised: it describes the credential, not the account, and the
    /// shape this type has on the wire is the account.
    #[serde(skip)]
    pub credential_expires_at: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("invalid or expired refresh token")]
    InvalidRefreshToken,
    #[error("invalid device name")]
    InvalidDeviceName,
    /// Too many failed attempts for this account from this address
    /// (RFC-017). Nothing was checked.
    #[error("too many failed sign-in attempts")]
    Throttled,
    #[error("authentication service unavailable")]
    Unavailable,
}

impl AuthService {
    pub fn new(db: Database, config: &Config) -> Self {
        Self {
            db,
            access_ttl_ms: duration_millis(config.access_token_ttl),
            refresh_ttl_ms: duration_millis(config.refresh_token_ttl),
            guard: SignInGuard::with_default_permits(),
            dummy_hash: security::hash_password(&Uuid::new_v4().to_string())
                .expect("hashing a random password cannot fail")
                .into(),
        }
    }

    /// The same service with a guard admitting `permits` verifications at once,
    /// so a test's outcome does not depend on the machine's CPU count.
    #[doc(hidden)]
    pub fn with_sign_in_permits(mut self, permits: usize) -> Self {
        self.guard = SignInGuard::new(permits);
        self
    }

    /// The guard, for the surfaces that check a password elsewhere.
    pub fn sign_in_guard(&self) -> &std::sync::Arc<SignInGuard> {
        &self.guard
    }

    /// Signs in with a password (RFC-017).
    ///
    /// A request that cannot be a guess — a device name that fails validation
    /// — is refused before anything is reserved. Then the attempt takes its
    /// slot in the limiter, and keeps it unless it succeeds or fails for an
    /// outage before the comparison. An unknown or disabled account costs the
    /// same Argon2 verification as a real one, against a dummy hash, so the
    /// answer's timing does not carry the large signal of a verification
    /// present or absent.
    pub async fn login(
        &self,
        username: &str,
        password: &str,
        device_name: &str,
        client: &SignInClient,
    ) -> Result<AuthTokens, AuthError> {
        let device_name = device_name.trim();
        if device_name.is_empty() || device_name.len() > 120 {
            return Err(AuthError::InvalidDeviceName);
        }

        let Ok(reservation) = self
            .guard
            .reserve(&client.address, &sign_in::canonical_account(username))
        else {
            self.guard.note(client.surface, Refusal::Throttled);
            return Err(AuthError::Throttled);
        };
        let account = match self.db.account_by_username(username).await {
            Ok(account) => account.filter(|account| !account.disabled),
            Err(error) => {
                tracing::error!(error = %error, "account lookup failed");
                reservation.release();
                return Err(AuthError::Unavailable);
            }
        };
        let Some(permit) = self.guard.admit().await else {
            reservation.release();
            self.guard.note(client.surface, Refusal::Busy);
            return Err(AuthError::Unavailable);
        };

        // The permit moves into the blocking task, so it is held for exactly
        // as long as the hashing runs, even if this request is dropped.
        let encoded_hash = account
            .as_ref()
            .map(|account| account.password_hash.clone());
        let dummy_hash = std::sync::Arc::clone(&self.dummy_hash);
        let password = password.to_owned();
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match encoded_hash {
                Some(hash) => security::verify_password(&password, &hash),
                None => security::verify_password(&password, &dummy_hash).map(|_| false),
            }
        })
        .await;
        // A worker that failed and a stored hash that cannot be read are the
        // server's faults, not an answer to the guess: the slot goes back.
        let valid = match valid {
            Ok(Ok(valid)) => valid,
            Ok(Err(error)) => {
                tracing::warn!(error = %error, "stored password hash is invalid");
                reservation.release();
                return Err(AuthError::Unavailable);
            }
            Err(error) => {
                tracing::error!(error = %error, "password worker failed");
                reservation.release();
                return Err(AuthError::Unavailable);
            }
        };
        let Some(account) = account.filter(|_| valid) else {
            self.guard.note(client.surface, Refusal::WrongCredentials);
            return Err(AuthError::InvalidCredentials);
        };
        reservation.release();

        let now_ms = now_ms();
        let device_id = self
            .db
            .create_device(account.id, device_name, now_ms)
            .await
            .map_err(db_unavailable)?;
        self.issue_new_session(
            account.id,
            account.username,
            account.role,
            device_id,
            now_ms,
            // A password login is the account itself: nothing narrows it.
            &[],
        )
        .await
    }

    /// Issues a session for an account whose identity was already established
    /// by another means — today, a redeemed OAuth authorization code. It
    /// deliberately takes no password: the caller is responsible for having
    /// proved the grant, and a disabled account is still refused here so a code
    /// minted before deactivation cannot be cashed in afterwards.
    pub async fn issue_session_for_account(
        &self,
        user_id: Uuid,
        device_name: &str,
        scopes: &[String],
    ) -> Result<AuthTokens, AuthError> {
        let device_name = device_name.trim();
        if device_name.is_empty() || device_name.len() > 120 {
            return Err(AuthError::InvalidDeviceName);
        }
        let account = self
            .db
            .account_by_id(user_id)
            .await
            .map_err(db_unavailable)?
            .ok_or(AuthError::InvalidCredentials)?;
        if account.disabled {
            return Err(AuthError::InvalidCredentials);
        }
        let now_ms = now_ms();
        let device_id = self
            .db
            .create_device(account.id, device_name, now_ms)
            .await
            .map_err(db_unavailable)?;
        self.issue_new_session(
            account.id,
            account.username,
            account.role,
            device_id,
            now_ms,
            scopes,
        )
        .await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<AuthTokens, AuthError> {
        let expected_hash = security::token_hash(refresh_token);
        let now_ms = now_ms();
        let session = self
            .db
            .session_by_refresh_hash(&expected_hash, now_ms)
            .await
            .map_err(db_unavailable)?
            .ok_or(AuthError::InvalidRefreshToken)?;

        let access_token = security::generate_token("wfa_");
        let refresh_token = security::generate_token("wfr_");
        let access_hash = security::token_hash(&access_token);
        let refresh_hash = security::token_hash(&refresh_token);
        let access_expires_at = now_ms + self.access_ttl_ms;
        let refresh_expires_at = now_ms + self.refresh_ttl_ms;
        let rotated = self
            .db
            .rotate_session(
                session.id,
                &expected_hash,
                &access_hash,
                &refresh_hash,
                access_expires_at,
                refresh_expires_at,
                now_ms,
            )
            .await
            .map_err(db_unavailable)?;
        if !rotated {
            return Err(AuthError::InvalidRefreshToken);
        }

        Ok(AuthTokens {
            access_token,
            refresh_token,
            token_type: "Bearer",
            expires_in: (self.access_ttl_ms / 1_000) as u64,
            user: AuthUser {
                id: session.user_id,
                username: session.username,
                role: session.role,
                // Rotation must not widen: the refreshed session answers to
                // exactly the scopes the original was issued under.
                scopes: session.scopes,
                // A session, so nothing to bound a minted token by. See the
                // field.
                credential_expires_at: None,
            },
            device_id: session.device_id,
        })
    }

    pub async fn logout(&self, access_token: &str) -> Result<(), AuthError> {
        let hash = security::token_hash(access_token);
        self.db
            .revoke_session_by_access_hash(&hash, now_ms())
            .await
            .map_err(db_unavailable)?;
        Ok(())
    }

    pub async fn revoke_refresh(&self, refresh_token: &str) -> Result<(), AuthError> {
        let hash = security::token_hash(refresh_token);
        self.db
            .revoke_session_by_refresh_hash(&hash, now_ms())
            .await
            .map_err(db_unavailable)?;
        Ok(())
    }

    pub async fn authenticate(&self, access_token: &str) -> Result<AuthUser, AuthError> {
        let hash = security::token_hash(access_token);
        let now = now_ms();
        let account = if access_token.starts_with("wfapi_") {
            self.db.account_by_api_token_hash(&hash, now).await
        } else {
            self.db.account_by_access_hash(&hash, now).await
        }
        .map_err(db_unavailable)?
        .ok_or(AuthError::InvalidCredentials)?;
        Ok(AuthUser {
            id: account.user_id,
            username: account.username,
            role: account.role,
            scopes: account.scopes,
            credential_expires_at: account.credential_expires_at,
        })
    }

    async fn issue_new_session(
        &self,
        user_id: Uuid,
        username: String,
        role: AccountRole,
        device_id: Uuid,
        now_ms: i64,
        scopes: &[String],
    ) -> Result<AuthTokens, AuthError> {
        let access_token = security::generate_token("wfa_");
        let refresh_token = security::generate_token("wfr_");
        let access_hash = security::token_hash(&access_token);
        let refresh_hash = security::token_hash(&refresh_token);
        let access_expires_at = now_ms + self.access_ttl_ms;
        let refresh_expires_at = now_ms + self.refresh_ttl_ms;
        self.db
            .create_session(NewSession {
                id: Uuid::new_v4(),
                user_id,
                device_id,
                access_token_hash: &access_hash,
                refresh_token_hash: &refresh_hash,
                access_expires_at,
                refresh_expires_at,
                now_ms,
                scopes,
            })
            .await
            .map_err(db_unavailable)?;

        Ok(AuthTokens {
            access_token,
            refresh_token,
            token_type: "Bearer",
            expires_in: (self.access_ttl_ms / 1_000) as u64,
            user: AuthUser {
                id: user_id,
                username,
                role,
                // Whatever the credential that issued it carried — empty for a
                // password login, narrowed for a grant made by a scoped token.
                scopes: scopes.to_vec(),
                // A session: see the field.
                credential_expires_at: None,
            },
            device_id,
        })
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn duration_millis(duration: std::time::Duration) -> i64 {
    i64::try_from(duration.as_millis()).expect("configured duration fits in i64 milliseconds")
}

fn db_unavailable(error: sqlx::Error) -> AuthError {
    tracing::error!(error = %error, "authentication database operation failed");
    AuthError::Unavailable
}
