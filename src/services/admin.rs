//! User accounts, API tokens and Subsonic credentials.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    pub async fn users(&self, actor_id: Uuid) -> Result<Vec<UserItem>, ServiceError> {
        self.require_admin(actor_id).await?;
        let mut users = sqlx::query("SELECT a.id, a.username, a.role, a.disabled, c.user_id IS NOT NULL AS has_credential FROM account a LEFT JOIN subsonic_credential c ON c.user_id=a.id ORDER BY a.username COLLATE NOCASE")
            .fetch_all(self.db.pool()).await?.into_iter().map(|row| Ok(UserItem { id: parse_uuid(row.try_get("id")?)?, username: row.try_get("username")?, role: AccountRole::from_str(row.try_get::<&str, _>("role")?).map_err(|error| sqlx::Error::Decode(error.into()))?, disabled: row.try_get::<i64, _>("disabled")? != 0, has_subsonic_credential: row.try_get::<i64, _>("has_credential")? != 0, folder_ids: Vec::new() })).collect::<Result<Vec<_>, sqlx::Error>>()?;
        let memberships = sqlx::query(
            "SELECT user_id, library_id FROM library_member ORDER BY user_id, library_id",
        )
        .fetch_all(self.db.pool())
        .await?;
        for row in memberships {
            let user_id = parse_uuid(row.try_get("user_id")?)?;
            let library_id = parse_uuid(row.try_get("library_id")?)?;
            if let Some(user) = users.iter_mut().find(|user| user.id == user_id) {
                user.folder_ids.push(library_id);
            }
        }
        Ok(users)
    }

    pub async fn create_web_user(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
        role: AccountRole,
    ) -> Result<UserItem, ServiceError> {
        self.require_admin(actor_id).await?;
        validate_username(username)?;
        if password.len() < 12 {
            return Err(ServiceError::Invalid);
        }
        let password = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || security::hash_password(&password))
            .await
            .map_err(|_| ServiceError::Unavailable)??;
        let id = self
            .db
            .create_account(username.trim(), &password_hash, role, now_ms())
            .await
            .map_err(|error| {
                if matches!(error, sqlx::Error::Database(ref db) if db.is_unique_violation()) {
                    ServiceError::Conflict
                } else {
                    ServiceError::Database(error)
                }
            })?;
        self.users(actor_id)
            .await?
            .into_iter()
            .find(|user| user.id == id)
            .ok_or(ServiceError::NotFound)
    }

    /// Sets a dedicated Subsonic password and rotates the API key. The clear
    /// API key is returned once; only its hash is persisted.
    pub async fn set_subsonic_credential(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
    ) -> Result<String, ServiceError> {
        self.require_admin(actor_id).await?;
        if password.len() < 12 {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let encrypted = self.secret_box.encrypt(password.as_bytes())?;
        let api_key = security::generate_token("wfsk_");
        let api_key_hash = security::token_hash(&api_key);
        self.db
            .set_subsonic_credential(actor_id, account.id, &encrypted, &api_key_hash, now_ms())
            .await?;
        Ok(api_key)
    }

    pub async fn revoke_subsonic_credential(
        &self,
        actor_id: Uuid,
        username: &str,
    ) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if self
            .db
            .revoke_subsonic_credential(actor_id, account.id, now_ms())
            .await?
        {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }

    /// The API tokens issued to one account.
    ///
    /// Administrative like the Subsonic credential routes beside it: a token
    /// carries the authority of the account it belongs to, so who may mint one
    /// is a question about the instance, not about the account itself.
    pub async fn api_tokens(
        &self,
        actor_id: Uuid,
        username: &str,
    ) -> Result<Vec<ApiTokenRecord>, ServiceError> {
        self.require_admin(actor_id).await?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        Ok(self.db.api_tokens_for_user(account.id).await?)
    }

    /// Issues a token and returns it beside its record.
    ///
    /// The secret is returned once and stored only as a SHA-256 hash, exactly
    /// as `set_subsonic_credential` returns its API key: a caller that loses it
    /// issues another one rather than reading it back.
    pub async fn create_api_token(
        &self,
        actor_id: Uuid,
        username: &str,
        name: &str,
        scopes: &[String],
    ) -> Result<(ApiTokenRecord, String), ServiceError> {
        self.require_admin(actor_id).await?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err(ServiceError::Invalid);
        }
        // Normalised on the way in, so the value a listing shows is the value
        // authorization compares. Trimming at the check instead would let a
        // stored `" admin "` grant what a reader of the listing would not
        // expect it to.
        let scopes = scopes
            .iter()
            .map(|scope| scope.trim().to_owned())
            .collect::<Vec<_>>();
        if scopes.iter().any(String::is_empty) {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let token = security::generate_token("wfapi_");
        let record = self
            .db
            .create_api_token(
                account.id,
                name,
                &security::token_hash(&token),
                &scopes,
                now_ms(),
            )
            .await?;
        Ok((record, token))
    }

    /// Revokes one token of one account.
    ///
    /// A token that is not this account's, or is already revoked, answers as a
    /// missing one: the caller asked for it to stop working, and naming the
    /// wrong owner must not confirm that it exists elsewhere.
    pub async fn revoke_api_token(
        &self,
        actor_id: Uuid,
        username: &str,
        token_id: Uuid,
    ) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if self
            .db
            .revoke_api_token(actor_id, account.id, token_id, now_ms())
            .await?
        {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }

    pub async fn create_subsonic_user(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
        admin: bool,
        folder_ids: Option<&[Uuid]>,
    ) -> Result<UserItem, ServiceError> {
        self.require_admin(actor_id).await?;
        validate_name(username)?;
        if password.is_empty() {
            return Err(ServiceError::Invalid);
        }
        let placeholder = security::generate_token("web-disabled-");
        let password_hash =
            tokio::task::spawn_blocking(move || security::hash_password(&placeholder))
                .await
                .map_err(|_| ServiceError::Unavailable)??;
        let encrypted = self.secret_box.encrypt(password.as_bytes())?;
        let api_key = security::generate_token("wfsk_");
        let api_key_hash = security::token_hash(&api_key);
        let requested_folders = self.resolve_library_ids(folder_ids).await?;
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        let user_id = Uuid::new_v4();
        let now = now_ms();
        let insert = sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(username.trim())
        .bind(password_hash)
        .bind(if admin { "admin" } else { "user" })
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await;
        if let Err(error) = insert {
            return Err(
                if matches!(error, sqlx::Error::Database(ref db) if db.is_unique_violation()) {
                    ServiceError::Conflict
                } else {
                    ServiceError::Database(error)
                },
            );
        }
        sqlx::query(
            "INSERT INTO subsonic_credential \
             (user_id, password_nonce, password_ciphertext, api_key_hash, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(encrypted.nonce.as_slice())
        .bind(encrypted.ciphertext)
        .bind(api_key_hash.as_slice())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        for library_id in requested_folders {
            sqlx::query(
                "INSERT INTO library_member (library_id, user_id, role, created_at) \
                 VALUES (?, ?, 'listener', ?)",
            )
            .bind(library_id.to_string())
            .bind(user_id.to_string())
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO audit_event (actor_user_id, kind, subject_id, occurred_at) \
             VALUES (?, 'subsonic.user_created', ?, ?)",
        )
        .bind(actor_id.to_string())
        .bind(user_id.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        drop(_writer);
        self.users(actor_id)
            .await?
            .into_iter()
            .find(|user| user.id == user_id)
            .ok_or(ServiceError::NotFound)
    }

    pub async fn update_user(
        &self,
        actor_id: Uuid,
        username: &str,
        update: UserUpdate<'_>,
    ) -> Result<UserItem, ServiceError> {
        self.require_admin(actor_id).await?;
        if update.subsonic_password.is_some_and(str::is_empty)
            || update
                .web_password
                .is_some_and(|password| password.len() < 12)
        {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if account.id == actor_id && (update.admin == Some(false) || update.disabled == Some(true))
        {
            return Err(ServiceError::Forbidden);
        }
        let requested_folders = match update.folder_ids {
            Some(ids) => Some(self.resolve_library_ids(Some(ids)).await?),
            None => None,
        };
        let encrypted = update
            .subsonic_password
            .map(|password| self.secret_box.encrypt(password.as_bytes()))
            .transpose()?;
        let web_password_hash = if let Some(password) = update.web_password {
            let password = password.to_owned();
            Some(
                tokio::task::spawn_blocking(move || security::hash_password(&password))
                    .await
                    .map_err(|_| ServiceError::Unavailable)??,
            )
        } else {
            None
        };
        let revoke_sessions = web_password_hash.is_some();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        sqlx::query("UPDATE account SET role=COALESCE(?, role), disabled=COALESCE(?, disabled), password_hash=COALESCE(?, password_hash), updated_at=? WHERE id=?")
            .bind(update.admin.map(|value| if value { "admin" } else { "user" })).bind(update.disabled.map(i64::from)).bind(web_password_hash.as_deref()).bind(now_ms()).bind(account.id.to_string()).execute(&mut *tx).await?;
        if revoke_sessions {
            sqlx::query("UPDATE session SET revoked_at=? WHERE user_id=? AND revoked_at IS NULL")
                .bind(now_ms())
                .bind(account.id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        if let Some(encrypted) = encrypted {
            let changed = sqlx::query(
                "UPDATE subsonic_credential SET password_nonce=?, password_ciphertext=?, updated_at=? WHERE user_id=?",
            )
            .bind(encrypted.nonce.as_slice())
            .bind(encrypted.ciphertext)
            .bind(now_ms())
            .bind(account.id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if changed == 0 {
                return Err(ServiceError::NotFound);
            }
        }
        if let Some(folder_ids) = requested_folders {
            sqlx::query("DELETE FROM library_member WHERE user_id=? AND role='listener'")
                .bind(account.id.to_string())
                .execute(&mut *tx)
                .await?;
            for library_id in folder_ids {
                sqlx::query(
                    "INSERT INTO library_member (library_id, user_id, role, created_at) \
                     VALUES (?, ?, 'listener', ?) \
                     ON CONFLICT (library_id, user_id) DO NOTHING",
                )
                .bind(library_id.to_string())
                .bind(account.id.to_string())
                .bind(now_ms())
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        drop(_writer);
        self.users(actor_id)
            .await?
            .into_iter()
            .find(|user| user.id == account.id)
            .ok_or(ServiceError::NotFound)
    }

    pub async fn delete_user(&self, actor_id: Uuid, username: &str) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if account.id == actor_id {
            return Err(ServiceError::Forbidden);
        }
        let _writer = self.db.writer_guard().await;
        sqlx::query("DELETE FROM account WHERE id=?")
            .bind(account.id.to_string())
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    pub async fn change_subsonic_password(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
    ) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        if password.is_empty() {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let encrypted = self.secret_box.encrypt(password.as_bytes())?;
        let _writer = self.db.writer_guard().await;
        let changed = sqlx::query("UPDATE subsonic_credential SET password_nonce=?, password_ciphertext=?, updated_at=? WHERE user_id=?")
            .bind(encrypted.nonce.as_slice()).bind(encrypted.ciphertext).bind(now_ms()).bind(account.id.to_string()).execute(self.db.pool()).await?.rows_affected();
        if changed == 0 {
            Err(ServiceError::NotFound)
        } else {
            Ok(())
        }
    }
}
