//! Credential lookup and native client authorisation.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    pub async fn bootstrap_admin(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Uuid, ServiceError> {
        validate_username(username)?;
        if password.len() < 12 {
            return Err(ServiceError::Invalid);
        }
        let password = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || security::hash_password(&password))
            .await
            .map_err(|_| ServiceError::Unavailable)??;
        self.db
            .bootstrap_admin(username, &password_hash, now_ms())
            .await?
            .ok_or(ServiceError::Conflict)
    }

    pub async fn credential_by_username(
        &self,
        username: &str,
    ) -> Result<Option<SubsonicCredentialRecord>, ServiceError> {
        let row = sqlx::query(
            "SELECT a.id, a.username, a.password_hash, a.role, a.disabled, \
                    c.password_nonce, c.password_ciphertext \
             FROM account a JOIN subsonic_credential c ON c.user_id=a.id \
             WHERE a.username=? COLLATE NOCASE AND a.disabled=0",
        )
        .bind(username)
        .fetch_optional(self.db.pool())
        .await?;
        row.map(credential_from_row).transpose().map_err(Into::into)
    }

    pub async fn credential_by_api_key(
        &self,
        api_key: &str,
    ) -> Result<Option<SubsonicCredentialRecord>, ServiceError> {
        let hash = security::token_hash(api_key);
        let row = sqlx::query(
            "SELECT a.id, a.username, a.password_hash, a.role, a.disabled, \
                    c.password_nonce, c.password_ciphertext \
             FROM account a JOIN subsonic_credential c ON c.user_id=a.id \
             WHERE c.api_key_hash=? AND a.disabled=0",
        )
        .bind(hash.as_slice())
        .fetch_optional(self.db.pool())
        .await?;
        row.map(credential_from_row).transpose().map_err(Into::into)
    }

    pub fn decrypt_subsonic_password(
        &self,
        credential: &SubsonicCredentialRecord,
    ) -> Result<Vec<u8>, ServiceError> {
        self.secret_box
            .decrypt(
                &credential.encrypted_password.nonce,
                &credential.encrypted_password.ciphertext,
            )
            .map_err(Into::into)
    }

    /// Issues an authorization code for a native client.
    ///
    /// Validation, credential generation and persistence live here rather than
    /// in the handler so the grant rules hold for every surface that ever
    /// issues one, and so they can be exercised without an HTTP request.
    /// Returns the URL the consent screen must send the user agent to.
    pub async fn authorize_native_client(
        &self,
        user_id: Uuid,
        request: AuthorizationRequest<'_>,
    ) -> Result<String, ServiceError> {
        crate::oauth::validate_redirect_uri(request.redirect_uri)
            .map_err(|_| ServiceError::Invalid)?;
        crate::oauth::validate_challenge(request.code_challenge_method, request.code_challenge)
            .map_err(|_| ServiceError::Invalid)?;
        let client_id = request.client_id.trim();
        let device_name = request.device_name.trim();
        // Checked before the code exists: a name the session issuer would
        // reject must not burn a grant the client can never redeem.
        if client_id.is_empty() || device_name.is_empty() || device_name.len() > 120 {
            return Err(ServiceError::Invalid);
        }

        let code = security::generate_token("wfc_");
        let now = now_ms();
        self.db
            .create_authorization(crate::database::NewAuthorization {
                code_hash: security::token_hash(&code),
                user_id,
                client_id,
                redirect_uri: request.redirect_uri,
                code_challenge: request.code_challenge,
                device_name,
                now_ms: now,
                expires_at: now + crate::oauth::AUTHORIZATION_CODE_TTL_MS,
                scopes: request.scopes,
            })
            .await?;
        Ok(crate::oauth::redirect_with_code(
            request.redirect_uri,
            &code,
            request.state,
        ))
    }
}
