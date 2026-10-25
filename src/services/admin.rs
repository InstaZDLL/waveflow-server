//! User accounts, API tokens and Subsonic credentials.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    /// Who may see a library, and in what standing.
    ///
    /// The write side has existed since M4 — `PUT` and `DELETE` on
    /// `/libraries/{id}/members/{user}` — with nothing to read it back, so a
    /// screen could grant and revoke without ever showing who already had
    /// access. That is not a listing anyone can build client-side: an account's
    /// own membership tells it nothing about anyone else's.
    ///
    /// Restricted to members, and by membership in the join rather than by a
    /// check ahead of it: a library the caller is not in is missing, and never
    /// confirmed to exist by a different answer.
    pub async fn library_members(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<Vec<LibraryMember>, ServiceError> {
        let rows = sqlx::query(
            "SELECT m.user_id, a.username, m.role, m.created_at \
             FROM library_member m \
             JOIN account a ON a.id=m.user_id \
             WHERE m.library_id=? AND EXISTS ( \
               SELECT 1 FROM library_member self \
               WHERE self.library_id=m.library_id AND self.user_id=?) \
             ORDER BY a.username COLLATE NOCASE",
        )
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        // An empty answer would be indistinguishable from a library the caller
        // cannot see, and every library has at least its owner — so nothing
        // here means nothing to see.
        if rows.is_empty() {
            return Err(ServiceError::NotFound);
        }
        rows.into_iter()
            .map(|row| {
                Ok(LibraryMember {
                    user_id: parse_uuid(row.try_get("user_id")?)?,
                    username: row.try_get("username")?,
                    role: row.try_get("role")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(ServiceError::from)
    }

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

    /// Whether the asking account holds a Subsonic credential, and since when.
    ///
    /// Self-scoped, where the two above are administrative, and never carrying
    /// the password. It exists so the settings panel can tell the two cases
    /// apart: an account with no credential has nothing to rotate and must not
    /// be offered a button that answers 404.
    pub async fn own_subsonic_credential(
        &self,
        actor_id: Uuid,
    ) -> Result<Option<SubsonicCredentialStatus>, ServiceError> {
        self.db
            .subsonic_credential_status(actor_id)
            .await
            .map_err(Into::into)
    }

    /// Replaces the asking account's own Subsonic password and API key.
    ///
    /// **Rotation, never creation.** `rotate_subsonic_credential` is an
    /// `UPDATE`, so an account holding no credential gets `NotFound` and the
    /// facade stays shut to it. Opening that door is the operator's decision
    /// and this path does not take it; what it takes is the answer to "my key
    /// has leaked", which until now needed an operator even though the
    /// credential belongs to the account.
    ///
    /// **Refused to a credential that could not hand on what this returns.** A
    /// `wfsk_` key carries no scopes and no expiry, because the facade reads
    /// neither — `subsonic::auth` resolves a principal from
    /// `credential_by_api_key` alone, and the `subsonic_credential` table has
    /// no column for either. So unlike an API token, the credential this
    /// returns cannot be narrowed to match its caller, and the only honest
    /// bound is the caller itself:
    ///
    /// - a **scoped** caller is refused, since a token restricted to `write`
    ///   would walk out holding an unrestricted key on the facade;
    /// - an **expiring** caller is refused, since a token good until tomorrow
    ///   would walk out holding a key that never stops — the same escalation
    ///   in the other currency, and the one `create_own_api_token` answers
    ///   with `min(policy, caller)`. There is no `min` to take here.
    ///
    /// A session passes both: it carries no scopes unless its grant did, and
    /// no expiry by design, because authorising something longer-lived than
    /// itself is what a session is for. So this is a thing a person does while
    /// signed in, which is also where the button lives.
    pub async fn rotate_own_subsonic_credential(
        &self,
        actor_id: Uuid,
        actor_scopes: &[String],
        actor_expires_at: Option<i64>,
        password: &str,
    ) -> Result<String, ServiceError> {
        if !actor_scopes.is_empty() || actor_expires_at.is_some() {
            return Err(ServiceError::Forbidden);
        }
        if password.len() < 12 {
            return Err(ServiceError::Invalid);
        }
        let encrypted = self.secret_box.encrypt(password.as_bytes())?;
        let api_key = security::generate_token("wfsk_");
        let api_key_hash = security::token_hash(&api_key);
        if self
            .db
            .rotate_subsonic_credential(actor_id, actor_id, &encrypted, &api_key_hash, now_ms())
            .await?
        {
            Ok(api_key)
        } else {
            Err(ServiceError::NotFound)
        }
    }

    /// Switches off the asking account's own Subsonic credential.
    ///
    /// **Bounded by nothing, where the rotation is bounded twice**, and the
    /// asymmetry is the point: this hands on no authority, it takes some away.
    /// A narrowed or expiring credential closing a door its account holds is
    /// doing the one thing that is always safe to allow, and refusing it would
    /// leave a compromised account unable to act on the news with the
    /// credential it has in hand.
    pub async fn revoke_own_subsonic_credential(&self, actor_id: Uuid) -> Result<(), ServiceError> {
        if self
            .db
            .revoke_subsonic_credential(actor_id, actor_id, now_ms())
            .await?
        {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }

    /// The API tokens issued to one account, as an administrator asks.
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
        self.tokens_of(account.id).await
    }

    /// The API tokens of the account asking, which needs no administrator.
    ///
    /// **The authority a token carries is the account's own**, and nothing
    /// here widens it: `authenticated` checks the role separately from the
    /// scopes, so a token minted by an ordinary account with `admin` written
    /// in its scopes grants nothing — a scope narrows a credential, it never
    /// promotes one. So who may see and revoke an account's own clients is a
    /// question about that account, and it used to need the whole instance's
    /// administrator: a listener could not find out which devices were reading
    /// their library, let alone stop one.
    ///
    /// The three methods below share one implementation each with their
    /// administrative twin. Two authorizations, one operation — validation,
    /// normalisation and the shape of a refusal belong to what is being done
    /// and not to who asked.
    pub async fn own_api_tokens(
        &self,
        actor_id: Uuid,
    ) -> Result<Vec<ApiTokenRecord>, ServiceError> {
        self.tokens_of(actor_id).await
    }

    async fn tokens_of(&self, account_id: Uuid) -> Result<Vec<ApiTokenRecord>, ServiceError> {
        Ok(self.db.api_tokens_for_user(account_id).await?)
    }

    /// Issues a token and returns it beside its record.
    ///
    /// The secret is returned once and stored only as a SHA-256 hash, exactly
    /// as `set_subsonic_credential` returns its API key: a caller that loses it
    /// issues another one rather than reading it back.
    pub async fn create_api_token(
        &self,
        actor_id: Uuid,
        actor_scopes: &[String],
        username: &str,
        name: &str,
        scopes: &[String],
    ) -> Result<(ApiTokenRecord, String), ServiceError> {
        self.require_admin(actor_id).await?;
        // **The same bound as the self-service path**, which this did not
        // have. Nothing exploits that today: reaching this route needs
        // `Access::Admin`, and the `admin` scope already admits every access
        // level, so a credential that gets here can already do everything. It
        // is the asymmetry that is the defect — the day a fourth scope exists,
        // `admin` stops meaning "everything" and this path would hand on more
        // than its caller holds, silently, while its twin refused.
        bound_by_caller(actor_scopes, scopes)?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        // No expiry: the operator path keeps the contract it has had since
        // the table existed.
        self.issue_token(actor_id, account.id, name, scopes, None)
            .await
    }

    /// Issues a token to the account asking. See [`Self::own_api_tokens`].
    ///
    /// **A credential cannot mint one wider than itself.** An empty scope list
    /// means "everything this account has", so a narrowed token asking for one
    /// would walk out with the account's whole authority — an escalation
    /// inside a single account, from a credential that had been deliberately
    /// restricted. The authorization-code flow had the same hole and it is
    /// closed the same way: the caller's own scopes bound what it can hand on.
    ///
    /// An unrestricted caller may ask for anything, including nothing. A
    /// narrowed one must name scopes, and only ones it holds.
    pub async fn create_own_api_token(
        &self,
        actor_id: Uuid,
        actor_scopes: &[String],
        actor_expires_at: Option<i64>,
        name: &str,
        scopes: &[String],
    ) -> Result<(ApiTokenRecord, String), ServiceError> {
        bound_by_caller(actor_scopes, scopes)?;
        // Bounded in time as well as in scope, and **only here**. See the
        // field on `Config`: an operator provisioning a service account is a
        // different act from a person clicking a button in their settings.
        // Bounded at both ends of the conversion. `as_millis` is a `u128`
        // and `as i64` truncates it, so a TTL an operator set absurdly high
        // could have landed negative — an expiry already in the past, which
        // fails closed but reads as a bug — or wrapped somewhere plausible.
        // Saturating instead: an unrepresentable lifetime becomes the longest
        // one this column can hold, which is what asking for forever means.
        let from_policy = self
            .api_token_ttl
            .map(|ttl| now_ms().saturating_add(i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX)));
        // **And never later than the credential that asked.** A token
        // expiring tomorrow could mint one expiring in a year, and that one
        // another: the deadline was defeatable by the very credential it
        // bounded, which made it decorative after a single hop. This is the
        // scope rule applied to time — a credential hands on no more than it
        // holds, in either currency.
        //
        // A session carries no deadline here, deliberately: it is short-lived
        // and refreshes, and authorising something longer-lived than itself is
        // part of what it is for. See `AuthUser::credential_expires_at`.
        let expires_at = match (from_policy, actor_expires_at) {
            (Some(policy), Some(caller)) => Some(policy.min(caller)),
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        };
        self.issue_token(actor_id, actor_id, name, scopes, expires_at)
            .await
    }

    async fn issue_token(
        &self,
        actor_id: Uuid,
        account_id: Uuid,
        name: &str,
        scopes: &[String],
        expires_at: Option<i64>,
    ) -> Result<(ApiTokenRecord, String), ServiceError> {
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
        let token = security::generate_token("wfapi_");
        let record = self
            .db
            .create_api_token(
                crate::database::NewApiToken {
                    actor_id,
                    user_id: account_id,
                    name,
                    token_hash: &security::token_hash(&token),
                    scopes: &scopes,
                    expires_at,
                },
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
        self.retire_token(actor_id, account.id, token_id).await
    }

    /// Revokes one of the asking account's own tokens. See
    /// [`Self::own_api_tokens`].
    ///
    /// A token belonging to somebody else answers as a missing one, because
    /// the owner is part of the condition: the update matches on the account
    /// as well as the id, so naming another tenant's token cannot confirm
    /// that it exists.
    pub async fn revoke_own_api_token(
        &self,
        actor_id: Uuid,
        token_id: Uuid,
    ) -> Result<(), ServiceError> {
        self.retire_token(actor_id, actor_id, token_id).await
    }

    async fn retire_token(
        &self,
        actor_id: Uuid,
        account_id: Uuid,
        token_id: Uuid,
    ) -> Result<(), ServiceError> {
        if self
            .db
            .revoke_api_token(actor_id, account_id, token_id, now_ms())
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
        let mut tx = self.db.begin_write(&_writer).await?;
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
        let mut tx = self.db.begin_write(&_writer).await?;
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

/// A credential may hand on no more than it holds.
///
/// An empty scope list means "everything this account has", so a token already
/// restricted asking for one would walk out carrying the account's whole
/// authority — an escalation inside a single account, from a credential
/// somebody narrowed on purpose. The authorization-code flow had the same
/// hole; `src/api/access.rs` records how it was closed, and this is the same
/// rule in the same place.
///
/// Refused rather than quietly narrowed to the caller's own set: a caller that
/// asked for everything and received a subset would believe it held more than
/// it does, and find out at the first refusal from a token it minted itself.
///
/// An unrestricted caller may ask for anything, including nothing.
fn bound_by_caller(actor_scopes: &[String], requested: &[String]) -> Result<(), ServiceError> {
    if actor_scopes.is_empty() {
        return Ok(());
    }
    let wanted: Vec<String> = requested
        .iter()
        .map(|scope| scope.trim().to_owned())
        .collect();
    if wanted.is_empty() {
        return Err(ServiceError::Forbidden);
    }
    // **Compared as authority, not as strings.** The rule is that a credential
    // hands on no more than it holds, and what it holds is a set of
    // permissions rather than a set of spellings: `admin` admits `write`, so
    // an administrator's narrowed token asking for a write-only one for a
    // script is a narrowing and must be allowed. String equality refused it,
    // which fails closed and is still wrong.
    //
    // Read from `Access::granted_by` rather than restated here. That is the
    // one table of scope implications in this server, and a second copy of it
    // would be a second copy to keep in step — the shape of defect this
    // session has spent its day on. The dependency direction is backwards for
    // it, a domain module reaching into the surface: the smaller wart of the
    // two.
    let grants_more = wanted.iter().any(|scope| {
        let alone = [scope.clone()];
        crate::api::access::Access::ALL
            .iter()
            .any(|level| level.granted_by(&alone) && !level.granted_by(actor_scopes))
    });
    if grants_more {
        return Err(ServiceError::Forbidden);
    }
    Ok(())
}
