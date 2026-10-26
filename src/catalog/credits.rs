//! Track credits and the library's credit decisions (RFC-015): storing a
//! track's first-step credits, and recomputing its participants from them.

use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use super::{apply::upsert_artist, parse_uuid, track_override_lists};

/// The credits of one role, in tag order.
///
/// The two lists derived from it are read in lockstep — a name and the sort
/// form paired with it — so they have to come from one ordering.
fn credits_in_role(
    credits: &[crate::tags::Credit],
    role: crate::tags::Role,
) -> Vec<&crate::tags::Credit> {
    let mut named: Vec<&crate::tags::Credit> = credits
        .iter()
        .filter(|credit| credit.role == role)
        .collect();
    named.sort_by_key(|credit| credit.position);
    named
}

/// The sort form of each artist credit *on this track*. A credit the track
/// tag says nothing about falls back to the album artist tag when the two name
/// the same artist — ALBUMARTISTSORT is the more commonly written of the two,
/// and matching on the name rather than on position means this cannot file one
/// artist under another's sort form.
///
/// Applied to the first step's credits, before they are stored, so
/// `track_credit` holds the sort form a participant gets and a decision splits
/// the same one a scan would.
pub(super) fn with_album_artist_sorts(
    mut credits: Vec<crate::tags::Credit>,
) -> Vec<crate::tags::Credit> {
    let album: Vec<(String, Option<String>)> =
        credits_in_role(&credits, crate::tags::Role::AlbumArtist)
            .iter()
            .map(|credit| (credit.name.clone(), credit.sort_name.clone()))
            .collect();
    for credit in credits
        .iter_mut()
        .filter(|credit| credit.role == crate::tags::Role::Artist && credit.sort_name.is_none())
    {
        credit.sort_name = album
            .iter()
            .find(|(name, _)| *name == credit.name)
            .and_then(|(_, sort)| sort.clone());
    }
    credits
}

/// A library's credit decisions (RFC-015), by key.
///
/// A row whose parts no longer decode is read as no decision rather than
/// failing the scan that reads it: the one writer stores a `Vec<String>`, and
/// a credit left whole is what the library had before anyone decided.
pub(crate) async fn credit_decisions(
    tx: &mut Transaction<'_, Sqlite>,
    library_id: Uuid,
) -> Result<crate::credit_split::Decisions, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT credit_key, decision, parts FROM artist_credit_decision WHERE library_id = ?",
    )
    .bind(library_id.to_string())
    .fetch_all(&mut **tx)
    .await?;
    let mut decisions = crate::credit_split::Decisions::new();
    for row in rows {
        let key: String = row.try_get("credit_key")?;
        let decision = match row.try_get::<&str, _>("decision")? {
            "keep" => crate::credit_split::CreditDecision::Keep,
            _ => {
                let parts: Option<String> = row.try_get("parts")?;
                match parts.and_then(|parts| serde_json::from_str::<Vec<String>>(&parts).ok()) {
                    Some(parts) => crate::credit_split::CreditDecision::Split(parts),
                    None => continue,
                }
            }
        };
        decisions.insert(key, decision);
    }
    Ok(decisions)
}

/// Stores a track's first-step credits, replacing what it held.
pub(crate) async fn write_track_credits(
    tx: &mut Transaction<'_, Sqlite>,
    library_id: Uuid,
    track_id: Uuid,
    credits: &[crate::tags::Credit],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM track_credit WHERE track_id = ?")
        .bind(track_id.to_string())
        .execute(&mut **tx)
        .await?;
    for credit in credits {
        // `OR IGNORE`: a position is unique per role by construction, and the
        // one thing that could repeat it — two groups of one role — the tag
        // mapper already merges. Failing a whole scan batch on it would cost
        // more than the duplicate credit it would protect.
        sqlx::query(
            "INSERT OR IGNORE INTO track_credit                (track_id, library_id, role, sub_role, position, name, sort_name)              VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(track_id.to_string())
        .bind(library_id.to_string())
        .bind(credit.role.as_str())
        .bind(&credit.sub_role)
        .bind(credit.position as i64)
        .bind(credit.name.trim())
        .bind(credit.sort_name.as_deref())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// A track's stored first-step credits, in role and position order.
pub(crate) async fn read_track_credits(
    tx: &mut Transaction<'_, Sqlite>,
    track_id: Uuid,
) -> Result<Vec<crate::tags::Credit>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT role, sub_role, position, name, sort_name FROM track_credit          WHERE track_id = ? ORDER BY role, position",
    )
    .bind(track_id.to_string())
    .fetch_all(&mut **tx)
    .await?;
    let mut credits = Vec::with_capacity(rows.len());
    for row in rows {
        // A role this binary does not know was written by a newer one; it is
        // left out rather than guessed, as `track_participant` leaves it.
        let Some(role) = crate::tags::Role::parse(row.try_get::<&str, _>("role")?) else {
            continue;
        };
        credits.push(crate::tags::Credit {
            role,
            sub_role: row.try_get("sub_role")?,
            name: row.try_get("name")?,
            sort_name: row.try_get("sort_name")?,
            position: row.try_get::<i64, _>("position")?.max(0) as usize,
        });
    }
    Ok(credits)
}

/// What recomputing one track's participants did.
#[derive(Debug, Clone)]
pub(crate) struct Recomputed {
    pub changed: bool,
    pub album_id: Option<Uuid>,
    pub full_hash: String,
}

/// Recomputes one track's participants from its stored credits and the
/// library's decisions (RFC-015 decision 5), with no file read.
///
/// The same function a scan calls, over the same inputs, so a decision taken
/// now and a rescan later end in identical rows. Nothing is written when the
/// result is what the track already holds: a pass over a track the decision
/// does not change touches no row, records no event and leaves no artist
/// behind.
pub(crate) async fn recompute_track_participants(
    tx: &mut Transaction<'_, Sqlite>,
    pid: &crate::pid::PidSpecs,
    library_id: Uuid,
    track_id: Uuid,
    decisions: &crate::credit_split::Decisions,
    now: i64,
) -> Result<Recomputed, sqlx::Error> {
    let row = sqlx::query("SELECT album_id, full_hash, artist_display FROM track WHERE id = ?")
        .bind(track_id.to_string())
        .fetch_one(&mut **tx)
        .await?;
    let album_id = row
        .try_get::<Option<String>, _>("album_id")?
        .map(parse_uuid)
        .transpose()?;
    let full_hash: String = row.try_get("full_hash")?;
    let artist_display: Option<String> = row.try_get("artist_display")?;

    let file_credits = read_track_credits(tx, track_id).await?;
    let overrides = track_override_lists(tx, track_id).await?;
    let credits = crate::credit_split::resolve(
        &file_credits,
        decisions,
        overrides.artists.as_deref(),
        |name| pid.artist_id(library_id, name.trim()),
    );
    let mut desired: Vec<(String, String, String, i64, Option<String>)> = credits
        .iter()
        .map(|credit| {
            (
                credit.role.as_str().to_owned(),
                pid.artist_id(library_id, credit.name.trim()).to_string(),
                credit.sub_role.clone(),
                credit.position as i64,
                credit.sort_name.clone(),
            )
        })
        .collect();
    desired.sort_by(|left, right| (&left.0, left.3).cmp(&(&right.0, right.3)));
    let current: Vec<(String, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT role, artist_id, sub_role, position, sort_name FROM track_participant          WHERE track_id = ? ORDER BY role, position",
    )
    .bind(track_id.to_string())
    .fetch_all(&mut **tx)
    .await?;
    if current == desired {
        return Ok(Recomputed {
            changed: false,
            album_id,
            full_hash,
        });
    }

    sqlx::query("DELETE FROM track_participant WHERE track_id = ?")
        .bind(track_id.to_string())
        .execute(&mut **tx)
        .await?;
    let mut first_album_artist = None;
    for credit in &credits {
        let artist_id = upsert_artist(tx, pid, library_id, &credit.name, now).await?;
        if credit.role == crate::tags::Role::AlbumArtist && first_album_artist.is_none() {
            first_album_artist = Some(artist_id);
        }
        sqlx::query(
            "INSERT INTO track_participant                (track_id, artist_id, library_id, role, sub_role, position, sort_name)              VALUES (?, ?, ?, ?, ?, ?, ?)              ON CONFLICT (track_id, artist_id, role, sub_role) DO NOTHING",
        )
        .bind(track_id.to_string())
        .bind(artist_id.to_string())
        .bind(library_id.to_string())
        .bind(credit.role.as_str())
        .bind(&credit.sub_role)
        .bind(credit.position as i64)
        .bind(credit.sort_name.as_deref())
        .execute(&mut **tx)
        .await?;
    }
    // The album hangs off its first credited artist, as the scan's upsert
    // writes it; the display string, and with it the album's id, stay what
    // the file said.
    if let (Some(album), Some(artist)) = (album_id, first_album_artist) {
        sqlx::query("UPDATE album SET album_artist_id = ?, updated_at = ? WHERE id = ?")
            .bind(artist.to_string())
            .bind(now)
            .bind(album.to_string())
            .execute(&mut **tx)
            .await?;
    }
    // The search index holds every credited name, as the scan writes it, so
    // a split track is found under each part and no longer under the whole.
    let mut names: Vec<&str> = credits.iter().map(|credit| credit.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    if let Some(display) = artist_display.as_deref() {
        names.push(display);
    }
    sqlx::query("UPDATE track_fts SET artists = ? WHERE track_id = ?")
        .bind(names.join(" "))
        .bind(track_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(Recomputed {
        changed: true,
        album_id,
        full_hash,
    })
}

pub(super) fn credit_names(
    credits: &[crate::tags::Credit],
    role: crate::tags::Role,
) -> Vec<String> {
    credits_in_role(credits, role)
        .iter()
        .map(|credit| credit.name.clone())
        .collect()
}
