//! Accounts: the first administrator, creating an account and finding one.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{insert_audit, parse_role, parse_uuid, Database};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountRole {
    Admin,
    User,
}

impl AccountRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }
}

impl FromStr for AccountRole {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "admin" => Ok(Self::Admin),
            "user" => Ok(Self::User),
            _ => anyhow::bail!("unknown account role: {value}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AccountRecord {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub role: AccountRole,
    pub disabled: bool,
}

impl Database {
    pub async fn setup_required(&self) -> Result<bool, sqlx::Error> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM account")
            .fetch_one(&self.pool)
            .await?;
        Ok(count == 0)
    }

    pub async fn bootstrap_admin(
        &self,
        username: &str,
        password_hash: &str,
        now_ms: i64,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let id = Uuid::new_v4();
        let inserted = sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
             SELECT ?, ?, ?, 'admin', ?, ? WHERE NOT EXISTS (SELECT 1 FROM account)",
        )
        .bind(id.to_string())
        .bind(username.trim())
        .bind(password_hash)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if inserted {
            insert_audit(&mut tx, Some(id), "instance.bootstrapped", Some(id), now_ms).await?;
        }
        tx.commit().await?;
        Ok(inserted.then_some(id))
    }

    pub async fn create_account(
        &self,
        username: &str,
        password_hash: &str,
        role: AccountRole,
        now_ms: i64,
    ) -> Result<Uuid, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(username.trim())
        .bind(password_hash)
        .bind(role.as_str())
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        insert_audit(&mut tx, Some(id), "account.created", Some(id), now_ms).await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn account_by_username(
        &self,
        username: &str,
    ) -> Result<Option<AccountRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, disabled \
             FROM account WHERE username = ? COLLATE NOCASE",
        )
        .bind(username.trim())
        .fetch_optional(&self.pool)
        .await?;
        row.map(account_from_row).transpose()
    }

    pub async fn account_by_id(&self, id: Uuid) -> Result<Option<AccountRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, disabled FROM account WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(account_from_row).transpose()
    }
}

fn account_from_row(row: sqlx::sqlite::SqliteRow) -> Result<AccountRecord, sqlx::Error> {
    Ok(AccountRecord {
        id: parse_uuid(row.try_get("id")?)?,
        username: row.try_get("username")?,
        password_hash: row.try_get("password_hash")?,
        role: parse_role(row.try_get("role")?)?,
        disabled: row.try_get::<i64, _>("disabled")? != 0,
    })
}
