//! The credentials an account signs in with: OAuth authorizations, devices,
//! sessions and API tokens.

use sqlx::Row;
use uuid::Uuid;

use super::{insert_audit, parse_role, parse_uuid, AccountRole, Database};

/// One issued API token, without its secret.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ApiTokenRecord {
    pub id: Uuid,
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub username: String,
    pub role: AccountRole,
    pub refresh_expires_at: i64,
    /// The scopes this session was issued under. Empty for a password login
    /// and for a grant made by an unscoped credential; a narrowed token that
    /// authorized a device leaves its narrowing here.
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone)]
/// One API token to write, gathered rather than passed as seven arguments.
///
/// `actor_id` is who registered it, which the audit entry records and which
/// is not always the owner: an administrator provisioning a service account is
/// the actor, and the account is the subject.
pub struct NewApiToken<'a> {
    pub actor_id: Uuid,
    pub user_id: Uuid,
    pub name: &'a str,
    pub token_hash: &'a [u8],
    pub scopes: &'a [String],
    pub expires_at: Option<i64>,
}

pub struct AccessRecord {
    pub user_id: Uuid,
    pub username: String,
    pub role: AccountRole,
    /// The scopes of the API token this request arrived on. Empty for a
    /// session or an OAuth grant, which carry the account's full authority,
    /// and for a token issued without any.
    pub scopes: Vec<String>,
    /// When the API token this request arrived on stops working, if it is one
    /// and if it has a deadline.
    ///
    /// `None` for a session, which is a different kind of credential: it is
    /// short-lived and refreshes, and authorising something longer-lived than
    /// itself is part of what it is for. `None` for a token issued without an
    /// expiry too — there is simply nothing to bound by.
    pub credential_expires_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewSession<'a> {
    pub id: Uuid,
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub access_token_hash: &'a [u8],
    pub refresh_token_hash: &'a [u8],
    pub access_expires_at: i64,
    pub refresh_expires_at: i64,
    pub now_ms: i64,
    /// Carried from the credential that issued this session, so it can never
    /// be broader than what asked for it.
    pub scopes: &'a [String],
}

#[derive(Debug, Clone)]
pub struct NewAuthorization<'a> {
    pub code_hash: [u8; 32],
    pub user_id: Uuid,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub code_challenge: &'a str,
    pub device_name: &'a str,
    pub now_ms: i64,
    pub expires_at: i64,
    /// The scopes of the credential that authorized this grant.
    pub scopes: &'a [String],
}

#[derive(Debug, Clone)]
pub struct AuthorizationRecord {
    pub user_id: Uuid,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub device_name: String,
    /// Handed to the session this grant is redeemed for.
    pub scopes: Vec<String>,
}

impl Database {
    pub async fn create_authorization(
        &self,
        grant: NewAuthorization<'_>,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        sqlx::query(
            "INSERT INTO oauth_authorization \
               (code_hash, user_id, client_id, redirect_uri, code_challenge, device_name, \
                created_at, expires_at, scopes_json) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(grant.code_hash.as_slice())
        .bind(grant.user_id.to_string())
        .bind(grant.client_id)
        .bind(grant.redirect_uri)
        .bind(grant.code_challenge)
        .bind(grant.device_name)
        .bind(grant.now_ms)
        .bind(grant.expires_at)
        .bind(encode_scopes(grant.scopes)?)
        .execute(&mut *tx)
        .await?;
        // Delegating access to another application is exactly the kind of event
        // the audit trail exists for, alongside account and credential changes.
        insert_audit(
            &mut tx,
            Some(grant.user_id),
            "oauth.authorization.granted",
            Some(grant.user_id),
            grant.now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Claims a grant for redemption, atomically marking it used.
    ///
    /// The `redeemed_at IS NULL` predicate inside the UPDATE is what makes a
    /// code single-use: two concurrent redemptions cannot both match, so a
    /// stolen code cannot be replayed alongside the legitimate exchange.
    ///
    /// A code is spent by the first presentation whatever its outcome, so a
    /// failed PKCE check burns it rather than leaving it open to further
    /// attempts. That forecloses guessing a verifier, and matches OAuth 2.1's
    /// rule that a code presented more than once must be revoked. The cost is
    /// that a client which botches its own exchange must restart the flow.
    pub async fn redeem_authorization(
        &self,
        code_hash: &[u8],
        now_ms: i64,
    ) -> Result<Option<AuthorizationRecord>, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let row = sqlx::query(
            "UPDATE oauth_authorization SET redeemed_at = ? \
             WHERE code_hash = ? AND redeemed_at IS NULL AND expires_at > ? \
             RETURNING user_id, client_id, redirect_uri, code_challenge, device_name, \
                       scopes_json",
        )
        .bind(now_ms)
        .bind(code_hash)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(AuthorizationRecord {
                user_id: parse_uuid(row.try_get("user_id")?)?,
                client_id: row.try_get("client_id")?,
                redirect_uri: row.try_get("redirect_uri")?,
                code_challenge: row.try_get("code_challenge")?,
                device_name: row.try_get("device_name")?,
                scopes: decode_scopes(row.try_get("scopes_json")?)?,
            })
        })
        .transpose()
    }

    /// Sweeps expired grants hourly for the life of the process.
    ///
    /// Rows are kept after redemption so a replay is recognised rather than
    /// looking unknown, so nothing else ever deletes them; without this the
    /// table only grows.
    pub fn spawn_authorization_pruning(&self) {
        let db = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
            loop {
                ticker.tick().await;
                match db
                    .prune_authorizations(crate::authentication::now_ms())
                    .await
                {
                    Ok(0) => {}
                    Ok(removed) => tracing::debug!(removed, "pruned expired authorization codes"),
                    Err(error) => {
                        tracing::warn!(error = %error, "could not prune authorization codes")
                    }
                }
            }
        });
    }

    /// Drops grants that can no longer be redeemed.
    pub async fn prune_authorizations(&self, now_ms: i64) -> Result<u64, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query("DELETE FROM oauth_authorization WHERE expires_at <= ?")
            .bind(now_ms)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn create_device(
        &self,
        user_id: Uuid,
        name: &str,
        now_ms: i64,
    ) -> Result<Uuid, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO device (id, user_id, name, created_at, last_seen_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(name.trim())
        .bind(now_ms)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn create_session(&self, session: NewSession<'_>) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        sqlx::query(
            "INSERT INTO session \
             (id, user_id, device_id, access_token_hash, refresh_token_hash, access_expires_at, \
              refresh_expires_at, created_at, last_used_at, scopes_json) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session.id.to_string())
        .bind(session.user_id.to_string())
        .bind(session.device_id.to_string())
        .bind(session.access_token_hash)
        .bind(session.refresh_token_hash)
        .bind(session.access_expires_at)
        .bind(session.refresh_expires_at)
        .bind(session.now_ms)
        .bind(session.now_ms)
        .bind(encode_scopes(session.scopes)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn session_by_refresh_hash(
        &self,
        refresh_hash: &[u8],
        now_ms: i64,
    ) -> Result<Option<SessionRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT s.id, s.user_id, s.device_id, a.username, a.role, s.refresh_expires_at, \
                    s.scopes_json \
             FROM session s JOIN account a ON a.id = s.user_id \
             WHERE s.refresh_token_hash = ? AND s.revoked_at IS NULL \
               AND s.refresh_expires_at > ? AND a.disabled = 0",
        )
        .bind(refresh_hash)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await?;
        row.map(session_from_row).transpose()
    }

    pub async fn account_by_access_hash(
        &self,
        access_hash: &[u8],
        now_ms: i64,
    ) -> Result<Option<AccessRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT a.id, a.username, a.role, s.scopes_json FROM session s \
             JOIN account a ON a.id = s.user_id \
             JOIN device d ON d.id = s.device_id \
             WHERE s.access_token_hash = ? AND s.revoked_at IS NULL \
               AND s.access_expires_at > ? AND a.disabled = 0 AND d.revoked_at IS NULL",
        )
        .bind(access_hash)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(AccessRecord {
                user_id: parse_uuid(row.try_get("id")?)?,
                username: row.try_get("username")?,
                role: parse_role(row.try_get("role")?)?,
                scopes: decode_scopes(row.try_get("scopes_json")?)?,
                // A session, not an API token: see the field.
                credential_expires_at: None,
            })
        })
        .transpose()
    }

    pub async fn account_by_api_token_hash(
        &self,
        token_hash: &[u8],
        now_ms: i64,
    ) -> Result<Option<AccessRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT a.id, a.username, a.role, t.scopes_json, t.last_used_at, \
                    t.expires_at FROM api_token t \
             JOIN account a ON a.id = t.user_id \
             WHERE t.token_hash = ? AND t.revoked_at IS NULL \
               AND (t.expires_at IS NULL OR t.expires_at > ?) AND a.disabled = 0",
        )
        .bind(token_hash)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let last_used_at: Option<i64> = row.try_get("last_used_at")?;
        let account = AccessRecord {
            user_id: parse_uuid(row.try_get("id")?)?,
            username: row.try_get("username")?,
            role: parse_role(row.try_get("role")?)?,
            scopes: decode_scopes(row.try_get("scopes_json")?)?,
            credential_expires_at: row.try_get("expires_at")?,
        };
        if last_used_at.is_none_or(|last_used| last_used <= now_ms.saturating_sub(60_000)) {
            let _writer = self.writer_guard().await;
            sqlx::query(
                "UPDATE api_token SET last_used_at = ? WHERE token_hash = ? \
                 AND revoked_at IS NULL AND (last_used_at IS NULL OR last_used_at <= ?)",
            )
            .bind(now_ms)
            .bind(token_hash)
            .bind(now_ms.saturating_sub(60_000))
            .execute(&self.pool)
            .await?;
        }
        Ok(Some(account))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn rotate_session(
        &self,
        session_id: Uuid,
        expected_refresh_hash: &[u8],
        access_hash: &[u8],
        refresh_hash: &[u8],
        access_expires_at: i64,
        refresh_expires_at: i64,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query(
            "UPDATE session SET access_token_hash = ?, refresh_token_hash = ?, \
                access_expires_at = ?, refresh_expires_at = ?, last_used_at = ? \
             WHERE id = ? AND refresh_token_hash = ? AND revoked_at IS NULL \
               AND refresh_expires_at > ?",
        )
        .bind(access_hash)
        .bind(refresh_hash)
        .bind(access_expires_at)
        .bind(refresh_expires_at)
        .bind(now_ms)
        .bind(session_id.to_string())
        .bind(expected_refresh_hash)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn revoke_session_by_access_hash(
        &self,
        access_hash: &[u8],
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query(
            "UPDATE session SET revoked_at = ? \
             WHERE access_token_hash = ? AND revoked_at IS NULL",
        )
        .bind(now_ms)
        .bind(access_hash)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn revoke_session_by_refresh_hash(
        &self,
        refresh_hash: &[u8],
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query(
            "UPDATE session SET revoked_at = ? \
             WHERE refresh_token_hash = ? AND revoked_at IS NULL",
        )
        .bind(now_ms)
        .bind(refresh_hash)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Issues a token and returns its record.
    ///
    /// `RETURNING` rather than an insert followed by a read: the caller wants
    /// the row it just wrote, and looking it up afterwards meant listing every
    /// token the account holds to find one whose id was already known.
    pub async fn create_api_token(
        &self,
        token: NewApiToken<'_>,
        now_ms: i64,
    ) -> Result<ApiTokenRecord, sqlx::Error> {
        let NewApiToken {
            actor_id,
            user_id,
            name,
            token_hash,
            scopes,
            expires_at,
        } = token;
        let _writer = self.writer_guard().await;
        let id = Uuid::new_v4();
        // `expires_at` has been a column, and honoured by the lookup, since
        // the table existed — and never written. Nothing minted a bounded
        // token, so every one of them outlived whatever asked for it.
        let mut tx = self.begin_write(&_writer).await?;
        let row = sqlx::query(
            "INSERT INTO api_token \
               (id, user_id, name, token_hash, scopes_json, expires_at, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             RETURNING id, name, scopes_json, expires_at, created_at, last_used_at, revoked_at",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(name.trim())
        .bind(token_hash)
        .bind(encode_scopes(scopes)?)
        .bind(expires_at)
        .bind(now_ms)
        .fetch_one(&mut *tx)
        .await?;
        // In the same transaction as the row it describes, the way the
        // revocation beside it already is. Revoking a credential left an audit
        // entry and registering one left none, which is the wrong half to
        // record: it became an act an account can perform for itself without
        // an administrator anywhere, so it is exactly the event an operator
        // would want to find afterwards.
        insert_audit(
            &mut tx,
            Some(actor_id),
            "api_token.created",
            Some(id),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        api_token_from_row(row)
    }

    pub async fn revoke_api_token_by_hash(
        &self,
        token_hash: &[u8],
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query(
            "UPDATE api_token SET revoked_at = ? \
             WHERE token_hash = ? AND revoked_at IS NULL",
        )
        .bind(now_ms)
        .bind(token_hash)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// The tokens issued to one account, newest first, secrets excluded.
    ///
    /// `token_hash` is never projected. The secret exists once, in the creation
    /// response; a listing that could return it would make every read of this
    /// route as sensitive as issuing a new one.
    pub async fn api_tokens_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<ApiTokenRecord>, sqlx::Error> {
        sqlx::query(
            "SELECT id, name, scopes_json, expires_at, created_at, last_used_at, revoked_at \
             FROM api_token WHERE user_id = ? ORDER BY created_at DESC, id",
        )
        .bind(user_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(api_token_from_row)
        .collect()
    }

    /// Revokes one token of one account.
    ///
    /// The account is part of the condition rather than checked beforehand, so
    /// an administrator cannot revoke a token by naming the wrong owner, and a
    /// token already revoked answers the same as one that never existed: the
    /// caller asked for it to stop working, and it does not.
    pub async fn revoke_api_token(
        &self,
        actor_id: Uuid,
        user_id: Uuid,
        token_id: Uuid,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let result = sqlx::query(
            "UPDATE api_token SET revoked_at = ? \
             WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
        )
        .bind(now_ms)
        .bind(token_id.to_string())
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
        let revoked = result.rows_affected() == 1;
        if revoked {
            insert_audit(
                &mut tx,
                Some(actor_id),
                "api_token.revoked",
                Some(token_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(revoked)
    }
}

fn session_from_row(row: sqlx::sqlite::SqliteRow) -> Result<SessionRecord, sqlx::Error> {
    Ok(SessionRecord {
        id: parse_uuid(row.try_get("id")?)?,
        user_id: parse_uuid(row.try_get("user_id")?)?,
        device_id: parse_uuid(row.try_get("device_id")?)?,
        username: row.try_get("username")?,
        role: parse_role(row.try_get("role")?)?,
        refresh_expires_at: row.try_get("refresh_expires_at")?,
        scopes: decode_scopes(row.try_get("scopes_json")?)?,
    })
}

/// Scopes are stored as a JSON array so a credential's limit travels as one
/// column, on the three tables that can carry one: `api_token`, and now
/// `oauth_authorization` and `session`.
fn encode_scopes(scopes: &[String]) -> Result<String, sqlx::Error> {
    serde_json::to_string(scopes).map_err(|error| sqlx::Error::Encode(error.into()))
}

/// Counterpart of [`encode_scopes`]. An unreadable list is a decode error
/// rather than an empty one: silently reading a corrupt limit as "no limit"
/// would widen a credential exactly where it must not.
fn decode_scopes(value: &str) -> Result<Vec<String>, sqlx::Error> {
    serde_json::from_str(value).map_err(|error| sqlx::Error::Decode(error.into()))
}

fn api_token_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ApiTokenRecord, sqlx::Error> {
    Ok(ApiTokenRecord {
        id: parse_uuid(row.try_get("id")?)?,
        name: row.try_get("name")?,
        scopes: decode_scopes(row.try_get("scopes_json")?)?,
        expires_at: row.try_get("expires_at")?,
        created_at: row.try_get("created_at")?,
        last_used_at: row.try_get("last_used_at")?,
        revoked_at: row.try_get("revoked_at")?,
    })
}
