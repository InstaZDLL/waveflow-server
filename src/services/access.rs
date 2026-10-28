//! The authorization checks every service shares: the administrator gate,
//! the library list a request may name, and entity membership.

use super::*;

impl DomainServices {
    pub(super) async fn require_admin(&self, actor_id: Uuid) -> Result<(), ServiceError> {
        let account = self
            .db
            .account_by_id(actor_id)
            .await?
            .ok_or(ServiceError::Forbidden)?;
        if account.role == AccountRole::Admin && !account.disabled {
            Ok(())
        } else {
            Err(ServiceError::Forbidden)
        }
    }

    pub(super) async fn resolve_library_ids(
        &self,
        requested: Option<&[Uuid]>,
    ) -> Result<Vec<Uuid>, ServiceError> {
        let available = sqlx::query_scalar::<_, String>("SELECT id FROM library ORDER BY id")
            .fetch_all(self.db.pool())
            .await?
            .into_iter()
            .map(parse_uuid)
            .collect::<Result<Vec<_>, _>>()?;
        let Some(requested) = requested else {
            return Ok(available);
        };
        let mut unique = Vec::new();
        for id in requested {
            if !available.contains(id) {
                return Err(ServiceError::NotFound);
            }
            if !unique.contains(id) {
                unique.push(*id);
            }
        }
        Ok(unique)
    }

    /// Whether `user_id` may write about every one of `entities`, checked
    /// before any of them is written.
    ///
    /// A Subsonic `star` or `scrobble` names several identifiers, and each is
    /// its own mutation with its own transaction. Applied one after another, a
    /// refusal on the third left the first two written and reported the whole
    /// request as failed — and a client retrying it wrote them twice. Checking
    /// them all first makes the refusal leave nothing behind.
    pub async fn authorize_entities(
        &self,
        user_id: Uuid,
        entities: &[(&str, Uuid)],
    ) -> Result<(), ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        for (kind, id) in entities {
            self.authorize_entity_on(&mut connection, user_id, kind, *id)
                .await?;
        }
        Ok(())
    }

    /// Whether `user_id` may write about the entity: a member of its library,
    /// and, for a track, the track available.
    ///
    /// A track whose file went is not found to every read, so it is not found
    /// to a write either: a bookmark, a scrobble, a star or a rating on it was
    /// accepted while the read beside it answered 70 (the 2026-10-28 client
    /// campaign). Refusing changes nothing already written — the rows stay, and
    /// come back with the file.
    pub(super) async fn authorize_entity_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        kind: &str,
        id: Uuid,
    ) -> Result<(), ServiceError> {
        let query = match kind {
            "track" => "SELECT 1 FROM track e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=? AND m.user_id=? AND e.is_available=1",
            "album" => "SELECT 1 FROM album e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=? AND m.user_id=?",
            "artist" => "SELECT 1 FROM artist e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=? AND m.user_id=?",
            _ => return Err(ServiceError::Invalid),
        };
        let exists = sqlx::query_scalar::<_, i64>(query)
            .bind(id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(&mut *connection)
            .await?;
        if exists.is_some() {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }
}
