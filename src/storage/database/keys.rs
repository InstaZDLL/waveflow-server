//! The instance key: binding a database to it, reading what it protects, and
//! moving everything it governs onto a new one.

use std::path::Path;

use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Connection, Row, SqliteConnection,
};
use uuid::Uuid;

use super::Database;

/// What an instance key protects, counted from the database.
///
/// Four numbers rather than a sentence, because the sentence is what the
/// documentation already carries and it is not what an operator needs at the
/// moment of deciding. The third is the one nobody expects: discarding the
/// scrobbling links discards the listens queued behind them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyMaterial {
    pub subsonic_credentials: i64,
    pub scrobble_links: i64,
    pub queued_listens: i64,
    pub shares: i64,
}

/// What to do with the values the previous key protected.
///
/// Two variants, and two commands above them rather than one command and a
/// flag: one of these preserves exactly what the other destroys, and that is
/// not a detail of a single operation.
pub enum Rekey<'a> {
    /// The old key is in hand. Every ciphertext is read and written back.
    Reencrypt(&'a crate::auth::security::SecretBox),
    /// The old key is gone. Nothing can be read, so the rows go.
    Discard,
}

impl Database {
    pub async fn bind_instance_key(
        &self,
        fingerprint: &[u8],
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        sqlx::query(
            "INSERT INTO instance_metadata (singleton, key_fingerprint, created_at) \
             VALUES (1, ?, ?) ON CONFLICT (singleton) DO NOTHING",
        )
        .bind(fingerprint)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        let stored: Vec<u8> =
            sqlx::query_scalar("SELECT key_fingerprint FROM instance_metadata WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(stored == fingerprint)
    }

    pub async fn check_file_instance_key(path: &Path, fingerprint: &[u8]) -> anyhow::Result<bool> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let stored = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT key_fingerprint FROM instance_metadata WHERE singleton=1",
        )
        .fetch_optional(&pool)
        .await?;
        pool.close().await;
        Ok(stored.is_some_and(|stored| stored == fingerprint))
    }

    /// The fingerprint this database was bound to, if it has been bound.
    ///
    /// Absent on a database no server has ever opened. Present and different
    /// is what aborts a boot, and what tells the two key commands apart.
    pub async fn stored_key_fingerprint(
        connection: &mut SqliteConnection,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT key_fingerprint FROM instance_metadata WHERE singleton=1",
        )
        .fetch_optional(&mut *connection)
        .await?)
    }

    /// What the instance key protects, counted rather than described.
    ///
    /// The operator is about to be told what a key change costs, and a count
    /// read from their own database is the only version of that worth
    /// printing.
    pub async fn key_material(connection: &mut SqliteConnection) -> anyhow::Result<KeyMaterial> {
        let count = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql);
        Ok(KeyMaterial {
            subsonic_credentials: count("SELECT COUNT(*) FROM subsonic_credential")
                .fetch_one(&mut *connection)
                .await?,
            scrobble_links: count("SELECT COUNT(*) FROM scrobble_link")
                .fetch_one(&mut *connection)
                .await?,
            // Every row of the outbox hangs off a link by `ON DELETE CASCADE`,
            // so discarding the links discards these with them. Counted
            // because an operator told "two scrobbling links" would not guess
            // that it also means a few hundred listens nobody has sent yet.
            queued_listens: count("SELECT COUNT(*) FROM scrobble_outbox")
                .fetch_one(&mut *connection)
                .await?,
            shares: count("SELECT COUNT(*) FROM share")
                .fetch_one(&mut *connection)
                .await?,
        })
    }

    /// Moves every value the instance key governs onto a new key.
    ///
    /// One transaction, so a database half under one key and half under
    /// another is not a state this can leave behind. The caller writes the new
    /// key file only after this returns, for the same reason: a key that
    /// arrived before the commit would name a database that never changed.
    ///
    /// Shares are rewritten whichever plan runs, and they are the reason this
    /// is one function rather than two. A share token is *derived* from the
    /// key rather than stored ([`crate::auth::security::SecretBox::derive_share_token`]),
    /// so after a key change the stored hash is one the new key can never
    /// produce: the share is not merely re-addressed, it is unreachable, and
    /// its owner is shown a URL that answers 404. Recomputing the hash is what
    /// makes "shares survive, with a new URL" true rather than hoped for.
    pub async fn rekey(
        connection: &mut SqliteConnection,
        plan: Rekey<'_>,
        new_key: &crate::auth::security::SecretBox,
        new_fingerprint: &[u8],
        now_ms: i64,
    ) -> anyhow::Result<KeyMaterial> {
        // Asked for here rather than taken on trust from the caller. It is a
        // connection flag, `migrate_on` turns it off, and the delete below is
        // the one statement whose correctness depends on it — a cascade that
        // did not fire leaves no error, only rows. A no-op inside a
        // transaction, so it has to come before the `begin`.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *connection)
            .await
            .map_err(|error| anyhow::anyhow!("could not enforce foreign keys: {error}"))?;
        let enforced = sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
            .fetch_one(&mut *connection)
            .await?;
        if enforced != 1 {
            anyhow::bail!("foreign keys are not enforced; nothing has been changed");
        }
        let mut tx = connection.begin().await?;
        let mut moved = KeyMaterial::default();
        match plan {
            Rekey::Reencrypt(old_key) => {
                let credentials = sqlx::query(
                    "SELECT user_id, password_nonce, password_ciphertext FROM subsonic_credential",
                )
                .fetch_all(&mut *tx)
                .await?;
                for row in credentials {
                    let user_id: String = row.try_get("user_id")?;
                    let plaintext = old_key
                        .decrypt(
                            row.try_get::<Vec<u8>, _>("password_nonce")?.as_slice(),
                            row.try_get::<Vec<u8>, _>("password_ciphertext")?.as_slice(),
                        )
                        .map_err(|_| {
                            anyhow::anyhow!(
                                "a stored Subsonic password does not open with this key; \
                                 nothing has been changed"
                            )
                        })?;
                    let sealed = new_key.encrypt(&plaintext)?;
                    sqlx::query(
                        "UPDATE subsonic_credential \
                         SET password_nonce=?, password_ciphertext=?, updated_at=? \
                         WHERE user_id=?",
                    )
                    .bind(sealed.nonce.as_slice())
                    .bind(&sealed.ciphertext)
                    .bind(now_ms)
                    .bind(&user_id)
                    .execute(&mut *tx)
                    .await?;
                    moved.subsonic_credentials += 1;
                }

                // The `unlinked` generations included: they keep their
                // ciphertext, and a past authorisation left unreadable would
                // be a row this server can no longer account for.
                let links = sqlx::query(
                    "SELECT id, credential_nonce, credential_ciphertext FROM scrobble_link",
                )
                .fetch_all(&mut *tx)
                .await?;
                for row in links {
                    let id: String = row.try_get("id")?;
                    let plaintext = old_key
                        .decrypt(
                            row.try_get::<Vec<u8>, _>("credential_nonce")?.as_slice(),
                            row.try_get::<Vec<u8>, _>("credential_ciphertext")?
                                .as_slice(),
                        )
                        .map_err(|_| {
                            anyhow::anyhow!(
                                "a stored scrobbling secret does not open with this key; \
                                 nothing has been changed"
                            )
                        })?;
                    let sealed = new_key.encrypt(&plaintext)?;
                    sqlx::query(
                        "UPDATE scrobble_link \
                         SET credential_nonce=?, credential_ciphertext=?, updated_at=? \
                         WHERE id=?",
                    )
                    .bind(sealed.nonce.as_slice())
                    .bind(&sealed.ciphertext)
                    .bind(now_ms)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                    moved.scrobble_links += 1;
                }
            }
            Rekey::Discard => {
                // The outbox goes with the links, by `ON DELETE CASCADE`
                // rather than by a statement here — which only holds with
                // enforcement on, and `migrate_on` leaves it off. Counted
                // before the delete and asserted against afterwards, because
                // a cascade that silently did not fire would leave the number
                // printed to the operator as the only trace of it.
                moved.queued_listens = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM scrobble_outbox \
                     WHERE link_id IN (SELECT id FROM scrobble_link)",
                )
                .fetch_one(&mut *tx)
                .await?;
                moved.subsonic_credentials = sqlx::query("DELETE FROM subsonic_credential")
                    .execute(&mut *tx)
                    .await?
                    .rows_affected() as i64;
                moved.scrobble_links = sqlx::query("DELETE FROM scrobble_link")
                    .execute(&mut *tx)
                    .await?
                    .rows_affected() as i64;
                let orphans = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM scrobble_outbox")
                    .fetch_one(&mut *tx)
                    .await?;
                if orphans > 0 {
                    anyhow::bail!(
                        "{orphans} queued listen(s) outlived the links they belong to; \
                         foreign keys were not enforced and nothing has been changed"
                    );
                }
            }
        }

        for id in sqlx::query_scalar::<_, String>("SELECT id FROM share")
            .fetch_all(&mut *tx)
            .await?
        {
            let share = Uuid::parse_str(&id)
                .map_err(|_| anyhow::anyhow!("a share carries an identifier that is not a UUID"))?;
            let token = new_key.derive_share_token(share);
            // `updated_at` is left alone on purpose. The share did not change;
            // the key that names it did, and bumping the column would hand
            // every synchronising client a modification with no journal entry
            // behind it.
            sqlx::query("UPDATE share SET token_hash=? WHERE id=?")
                .bind(crate::auth::security::token_hash(&token).as_slice())
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            moved.shares += 1;
        }

        sqlx::query(
            "INSERT INTO instance_metadata (singleton, key_fingerprint, created_at) \
             VALUES (1, ?, ?) \
             ON CONFLICT (singleton) DO UPDATE SET key_fingerprint = excluded.key_fingerprint",
        )
        .bind(new_fingerprint)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(moved)
    }
}
