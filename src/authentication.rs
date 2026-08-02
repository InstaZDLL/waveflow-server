//! Local WaveFlow authentication service.

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    config::Config,
    database::{AccountRole, Database, NewSession},
    security,
};

#[derive(Clone)]
pub struct AuthService {
    db: Database,
    access_ttl_ms: i64,
    refresh_ttl_ms: i64,
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
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("invalid or expired refresh token")]
    InvalidRefreshToken,
    #[error("invalid device name")]
    InvalidDeviceName,
    #[error("authentication service unavailable")]
    Unavailable,
}

impl AuthService {
    pub fn new(db: Database, config: &Config) -> Self {
        Self {
            db,
            access_ttl_ms: duration_millis(config.access_token_ttl),
            refresh_ttl_ms: duration_millis(config.refresh_token_ttl),
        }
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
        device_name: &str,
    ) -> Result<AuthTokens, AuthError> {
        let device_name = device_name.trim();
        if device_name.is_empty() || device_name.len() > 120 {
            return Err(AuthError::InvalidDeviceName);
        }

        let account = self
            .db
            .account_by_username(username)
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "account lookup failed");
                AuthError::Unavailable
            })?
            .ok_or(AuthError::InvalidCredentials)?;
        if account.disabled {
            return Err(AuthError::InvalidCredentials);
        }

        let encoded_hash = account.password_hash.clone();
        let password = password.to_owned();
        let valid = tokio::task::spawn_blocking(move || {
            security::verify_password(&password, &encoded_hash)
        })
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "password worker failed");
            AuthError::Unavailable
        })?
        .map_err(|error| {
            tracing::warn!(error = %error, "stored password hash is invalid");
            AuthError::Unavailable
        })?;
        if !valid {
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

    pub async fn authenticate(&self, access_token: &str) -> Result<AuthUser, AuthError> {
        let hash = security::token_hash(access_token);
        let account = self
            .db
            .account_by_access_hash(&hash, now_ms())
            .await
            .map_err(db_unavailable)?
            .ok_or(AuthError::InvalidCredentials)?;
        Ok(AuthUser {
            id: account.user_id,
            username: account.username,
            role: account.role,
        })
    }

    async fn issue_new_session(
        &self,
        user_id: Uuid,
        username: String,
        role: AccountRole,
        device_id: Uuid,
        now_ms: i64,
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
