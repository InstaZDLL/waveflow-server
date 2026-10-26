//! The dedicated Subsonic credential: setting, rotating, revoking it, and
//! saying whether an account holds one.

use uuid::Uuid;

use crate::auth::security::EncryptedSecret;

use super::{insert_audit, Database};

/// Whether an account holds a dedicated Subsonic credential, and since when.
///
/// **The password is not in this shape and cannot be put into it**, because
/// the query behind it never selects the columns that hold one. A shape that
/// does not carry a secret cannot leak one by a later careless edit of the
/// handler.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct SubsonicCredentialStatus {
    /// When this account's credential was first set.
    pub created_at: i64,
    /// When it last changed — the same instant a rotation reports back.
    pub updated_at: i64,
}

impl Database {
    pub async fn set_subsonic_credential(
        &self,
        actor_id: Uuid,
        user_id: Uuid,
        secret: &EncryptedSecret,
        api_key_hash: &[u8],
        now_ms: i64,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        sqlx::query(
            "INSERT INTO subsonic_credential \
             (user_id, password_nonce, password_ciphertext, api_key_hash, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (user_id) DO UPDATE SET \
               password_nonce = excluded.password_nonce, \
               password_ciphertext = excluded.password_ciphertext, \
               api_key_hash = excluded.api_key_hash, updated_at = excluded.updated_at",
        )
        .bind(user_id.to_string())
        .bind(secret.nonce.as_slice())
        .bind(&secret.ciphertext)
        .bind(api_key_hash)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            Some(actor_id),
            "subsonic.credential_set",
            Some(user_id),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn revoke_subsonic_credential(
        &self,
        actor_id: Uuid,
        user_id: Uuid,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let result = sqlx::query("DELETE FROM subsonic_credential WHERE user_id = ?")
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?;
        let revoked = result.rows_affected() == 1;
        // Only when something was removed. The entry was unconditional, and
        // the service answers 404 after writing one, so the trail recorded
        // revocations that had not happened. That was cosmetic while an
        // administrator was the only caller; the self-scoped route makes it
        // reachable by any account, in a loop, which turns a wrong line in a
        // history into a way to fill one. `rotate_subsonic_credential` guards
        // the same way, for the same reason.
        if revoked {
            insert_audit(
                &mut tx,
                Some(actor_id),
                "subsonic.credential_revoked",
                Some(user_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(revoked)
    }

    /// Whether this account holds a Subsonic credential, and since when.
    ///
    /// The projection is the two timestamps and nothing else. The password
    /// columns are not read, so no edit of a caller can forward what this
    /// never fetched.
    pub async fn subsonic_credential_status(
        &self,
        user_id: Uuid,
    ) -> Result<Option<SubsonicCredentialStatus>, sqlx::Error> {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT created_at, updated_at FROM subsonic_credential WHERE user_id = ?",
        )
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map(|row| {
            row.map(|(created_at, updated_at)| SubsonicCredentialStatus {
                created_at,
                updated_at,
            })
        })
    }

    /// Replaces the password and the API key of a credential that already
    /// exists, and answers whether one did.
    ///
    /// **An `UPDATE`, where `set_subsonic_credential` upserts, and that is the
    /// whole difference between the two.** This one cannot bring a credential
    /// into being: an account holding none cannot use the Subsonic facade at
    /// all, so creating one does not narrow a door already open — it opens
    /// one, which is the operator's decision and not this path's. Checking
    /// first and then upserting would be the same decision taken racily;
    /// `WHERE user_id = ?` takes it in the statement, where two callers cannot
    /// interleave.
    ///
    /// The API key moves with the password. Leaving it would rotate half a
    /// credential: a leaked `wfsk_` key authenticates on its own, so a
    /// rotation that kept it would answer "my key has leaked" with a new
    /// password and the same working key.
    pub async fn rotate_subsonic_credential(
        &self,
        actor_id: Uuid,
        user_id: Uuid,
        secret: &EncryptedSecret,
        api_key_hash: &[u8],
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let result = sqlx::query(
            "UPDATE subsonic_credential SET \
               password_nonce = ?, password_ciphertext = ?, api_key_hash = ?, updated_at = ? \
             WHERE user_id = ?",
        )
        .bind(secret.nonce.as_slice())
        .bind(&secret.ciphertext)
        .bind(api_key_hash)
        .bind(now_ms)
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
        let rotated = result.rows_affected() == 1;
        // Only when something changed. An audit trail that records attempts
        // the database refused would read as a history of rotations that never
        // happened.
        if rotated {
            insert_audit(
                &mut tx,
                Some(actor_id),
                "subsonic.credential_rotated",
                Some(user_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(rotated)
    }
}
