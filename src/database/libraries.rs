//! Libraries: their visibility and member roles, creating one, its members,
//! and the operator's switches for uploads and canvases.

use std::{path::Path, str::FromStr};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{insert_audit, Database};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LibraryVisibility {
    Private,
    Shared,
}

impl LibraryVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Shared => "shared",
        }
    }
}

impl FromStr for LibraryVisibility {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "private" => Ok(Self::Private),
            "shared" => Ok(Self::Shared),
            _ => anyhow::bail!("visibility must be private or shared"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LibraryRole {
    Owner,
    Manager,
    Listener,
}

impl LibraryRole {
    /// Whether the role may spend the owner's disk on a rescan.
    ///
    /// The same rule the `scan_job` insert enforces in SQL, named once here
    /// so the Subsonic fan-out over every reachable library agrees with it
    /// instead of discovering the refusal one insert at a time.
    pub fn may_scan(self) -> bool {
        matches!(self, Self::Owner | Self::Manager)
    }

    /// Whether the role may correct a track's tags.
    ///
    /// The same two roles as [`Self::may_scan`], and not by coincidence: a
    /// correction is written once and read by every member, so it asks the same
    /// trust as spending the owner's disk. Named apart because the two rules
    /// answer different questions and one of them may move.
    pub fn may_write_metadata(self) -> bool {
        matches!(self, Self::Owner | Self::Manager)
    }

    /// Whether the role may hand the library a new file.
    ///
    /// The same two roles again, and deliberately so: a rescan spends the
    /// owner's disk, and so does receiving a file — permanently, this time.
    /// It is never sufficient on its own. `library.accepts_uploads` decides
    /// where, and a role that may upload into an opted-in library may upload
    /// into no other.
    pub fn may_upload(self) -> bool {
        matches!(self, Self::Owner | Self::Manager)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Manager => "manager",
            Self::Listener => "listener",
        }
    }
}

impl FromStr for LibraryRole {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "owner" => Ok(Self::Owner),
            "manager" => Ok(Self::Manager),
            "listener" => Ok(Self::Listener),
            _ => anyhow::bail!("library role must be owner, manager or listener"),
        }
    }
}

impl Database {
    pub async fn create_library(
        &self,
        owner_id: Uuid,
        name: &str,
        root_path: &Path,
        visibility: LibraryVisibility,
        now_ms: i64,
    ) -> Result<Uuid, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let id = Uuid::new_v4();
        let root_path = root_path.to_string_lossy();
        sqlx::query(
            "INSERT INTO library \
             (id, owner_user_id, name, root_path, visibility, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(owner_id.to_string())
        .bind(name.trim())
        .bind(root_path.as_ref())
        .bind(visibility.as_str())
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO library_member (library_id, user_id, role, created_at) VALUES (?, ?, 'owner', ?)",
        )
        .bind(id.to_string())
        .bind(owner_id.to_string())
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        insert_audit(&mut tx, Some(owner_id), "library.created", Some(id), now_ms).await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn add_library_member(
        &self,
        actor_id: Uuid,
        library_id: Uuid,
        user_id: Uuid,
        role: LibraryRole,
        now_ms: i64,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        sqlx::query(
            "INSERT INTO library_member (library_id, user_id, role, created_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT (library_id, user_id) DO UPDATE SET role = excluded.role",
        )
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .bind(role.as_str())
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            Some(actor_id),
            "library.member_set",
            Some(library_id),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn remove_library_member(
        &self,
        actor_id: Uuid,
        library_id: Uuid,
        user_id: Uuid,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let removed = sqlx::query(
            "DELETE FROM library_member \
             WHERE library_id=? AND user_id=? AND role!='owner'",
        )
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if removed {
            insert_audit(
                &mut tx,
                Some(actor_id),
                "library.member_removed",
                Some(library_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(removed)
    }

    /// Opens or closes a library to receiving files.
    ///
    /// Audited, like every other change to what a library permits. The role a
    /// member holds says who may upload; this says whether the library takes
    /// anything at all, and it is the operator's call rather than a member's —
    /// which is why it arrives through the CLI and not through the API.
    pub async fn set_library_accepts_uploads(
        &self,
        actor_id: Uuid,
        library_id: Uuid,
        accepts: bool,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let changed = sqlx::query("UPDATE library SET accepts_uploads=? WHERE id=?")
            .bind(i64::from(accepts))
            .bind(library_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
        if changed {
            insert_audit(
                &mut tx,
                Some(actor_id),
                if accepts {
                    "library.uploads_opened"
                } else {
                    "library.uploads_closed"
                },
                Some(library_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    /// Opens or closes a library to receiving canvases.
    ///
    /// A second door rather than the same one, and the reasoning is in
    /// `20260830010000_library_accepts_canvas.sql`: a read-only server that
    /// refuses to grow in audio may still want a few hundred kilobytes of video
    /// loop, and that is the most common installation there is.
    ///
    /// Audited and CLI-only, exactly like the upload flag. The role a member
    /// holds says who may place one; this says whether the library takes any at
    /// all, and that is the operator's call rather than a member's.
    pub async fn set_library_accepts_canvas(
        &self,
        actor_id: Uuid,
        library_id: Uuid,
        accepts: bool,
        now_ms: i64,
    ) -> Result<bool, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let changed = sqlx::query("UPDATE library SET accepts_canvas=? WHERE id=?")
            .bind(i64::from(accepts))
            .bind(library_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
        if changed {
            insert_audit(
                &mut tx,
                Some(actor_id),
                if accepts {
                    "library.canvas_opened"
                } else {
                    "library.canvas_closed"
                },
                Some(library_id),
                now_ms,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }
}
