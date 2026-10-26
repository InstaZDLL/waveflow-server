//! Importing a playlist file into a playlist, and exporting one as a file.

use super::*;

impl DomainServices {
    /// Imports an `.m3u` or `.m3u8` file as a playlist of this account's.
    ///
    /// The file is read by [`crate::library::playlist_file`]; what is left here is the
    /// part that needs the catalogue — turning what an entry *says* into a
    /// track this account can actually reach.
    ///
    /// **The ladder.** A playlist entry is a path relative to wherever the file
    /// lived, or absolute on a filesystem that is not this server's, so what
    /// the two sides share is a trailing run of segments. The longest suffix is
    /// tried first and the rungs go down to the bare file name; several matches
    /// at any rung is a refusal rather than a guess, because a shorter suffix
    /// can only match more. An entry no path matched gets one last rung: its
    /// `#EXTINF` credit, which is the only other thing a playlist file carries.
    ///
    /// **The issue asked for the scanner's ladder — path, then content hash —
    /// and the second rung cannot exist here.** The scanner holds the file and
    /// can hash it; a playlist entry hands over a path and, at best, a display
    /// title. So the fallback is the credit, and an entry whose path moved
    /// *and* whose credit was never written is reported rather than dropped.
    ///
    /// Resolution runs tenant-scoped in SQL, so a track in a library this
    /// account cannot see is reported as unmatched, exactly like one that does
    /// not exist — the report says nothing about which of the two it was.
    pub async fn import_playlist(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
        name: Option<&str>,
        bytes: &[u8],
        dry_run: bool,
        context: MutationContext,
    ) -> Result<PlaylistImport, ServiceError> {
        // The playlist ceiling, applied while the file is read rather than
        // discovered after the work: every entry costs a resolution, and a
        // caller that sent five thousand too many is owed the refusal before
        // the reads — and before the parser has built them (RFC-023
        // decision 5).
        let file = playlist_file::parse(bytes, MAX_PLAYLIST_TRACKS)
            .map_err(|playlist_file::TooManyEntries| ServiceError::Invalid)?;
        let name = name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| file.name.clone())
            .ok_or(ServiceError::Invalid)?;
        validate_name(&name)?;

        let mut connection = self.db.pool().acquire().await?;
        let by_file_name = self
            .tracks_by_file_name_on(&mut connection, user_id, library_ids, &file.entries)
            .await?;
        // The path rung for every entry, then one batch for the leftovers: the
        // credit lookup is only worth making for the entries that reached it,
        // and making it per entry would be a query per line of a file that may
        // hold ten thousand.
        let climbed = file
            .entries
            .iter()
            .map(|entry| {
                // ASCII, like the comparison that selected the candidates
                // and like every other catalogue match. `to_lowercase` folds
                // more than SQLite's `NOCASE` does, and a key wider than the
                // filter that produced it is a disagreement: an entry reading
                // `ÉTÉ.flac` against a stored `été.flac` would be folded here
                // and never returned there. The residual limit is that such an
                // entry is reported rather than placed, which the report says
                // out loud.
                let segments = entry
                    .segments
                    .iter()
                    .map(|segment| segment.to_ascii_lowercase())
                    .collect::<Vec<_>>();
                (entry, climb(&segments, &by_file_name))
            })
            .collect::<Vec<_>>();
        let leftovers = climbed
            .iter()
            .filter(|(_, rung)| matches!(rung, Rung::Unmatched))
            .map(|(entry, _)| *entry)
            .collect::<Vec<_>>();
        let by_credit = self
            .tracks_by_credit_on(&mut connection, user_id, library_ids, &leftovers)
            .await?;
        drop(connection);

        let mut track_ids: Vec<Uuid> = Vec::new();
        let mut unresolved: Vec<UnresolvedEntry> = Vec::new();
        for (entry, rung) in climbed {
            let report = |reason| UnresolvedEntry {
                line: entry.line,
                entry: entry.raw.clone(),
                title: entry.title.clone(),
                reason,
            };
            let outcome = match rung {
                Rung::Found(id) => Ok(id),
                Rung::Ambiguous => Err(UnresolvedReason::Ambiguous),
                Rung::NotAPath => Err(UnresolvedReason::NotAPath),
                Rung::Unmatched => match entry
                    .artist_and_title()
                    .map(|(artist, title)| format!("{artist}\n{title}").to_ascii_lowercase())
                    .and_then(|key| by_credit.get(&key))
                    .map(Vec::as_slice)
                {
                    Some([only]) => Ok(*only),
                    Some([_, ..]) => Err(UnresolvedReason::Ambiguous),
                    Some([]) | None => Err(UnresolvedReason::NoMatch),
                },
            };
            match outcome {
                // Every copy is placed: a file that names a track twice means
                // it twice, and a playlist now holds what it is given (RFC-020
                // decision 1). It used to be reported as a duplicate and left
                // out, which made the export of a playlist with a repeat
                // import as a shorter one.
                Ok(id) => track_ids.push(id),
                Err(reason) => unresolved.push(report(reason)),
            }
        }

        // Created through the ordinary verb rather than beside it: an import is
        // a playlist creation whose track list came out of a file, so it claims
        // a sync operation, lands in the journal and reaches other devices like
        // any other. A second insert here would be a second implementation of
        // what a playlist is.
        let playlist = match dry_run {
            true => None,
            false => Some(
                self.create_playlist_with_context(user_id, &name, &track_ids, context)
                    .await?,
            ),
        };
        Ok(PlaylistImport {
            playlist,
            name,
            encoding: file.encoding,
            entries: file.entries.len(),
            resolved: track_ids.len(),
            unresolved,
        })
    }

    /// Every track whose file name one of the entries could be naming, with
    /// its own path, keyed on that name.
    ///
    /// **The file name is the key because the comparison is symmetric, and the
    /// first version of this was not.** An entry can be *longer* than the
    /// stored path — absolute on someone else's filesystem — and it can just
    /// as easily be *shorter*: a `.m3u` written inside an album folder names
    /// `01 Hidden Place.flac` while the catalogue stores
    /// `Björk/Vespertine/01 Hidden Place.flac`. Asking SQL for the stored path
    /// to equal one of the entry's own suffixes answers the first case and
    /// never the second, which is the common one. What the two shapes always
    /// share is the file name, so that is what is asked for; how much of the
    /// rest agrees is settled in memory by [`climb`], where both sides are
    /// whole.
    ///
    /// The name is extracted in SQL — SQLite has no `basename`, hence the
    /// `rtrim` of everything after the last separator — so no index can serve
    /// the predicate and the query is a scan of the visible tracks. One scan
    /// per import, which is a rare user-initiated act; the alternative is a
    /// stored or generated column for a path shape only this feature reads.
    ///
    /// **One scan, which is why the whole candidate set goes down at once.**
    /// Asking in slices made the expression re-evaluated over every visible
    /// track once per slice — ten scans for the largest file the route
    /// accepts — to protect against a statement size that was never at risk:
    /// the names travel as a single bind parameter, so the SQL text is fixed
    /// whatever the set, and a few hundred kilobytes of JSON is a parameter
    /// SQLite reads once. The slicing was guarding a non-problem by causing a
    /// real one. Raised in review.
    ///
    /// The value is a list on purpose: a file name two tracks carry is what
    /// [`UnresolvedReason::Ambiguous`] is about, and a map keyed to one id
    /// would have hidden the second.
    async fn tracks_by_file_name_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        library_ids: &[Uuid],
        entries: &[PlaylistFileEntry],
    ) -> Result<HashMap<String, Vec<(Uuid, Vec<String>)>>, ServiceError> {
        let mut candidates: Vec<String> = entries
            .iter()
            .filter_map(|entry| entry.segments.last().cloned())
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        let folders = folder_filter(library_ids);
        let wanted = serde_json::to_string(&candidates).map_err(|_| ServiceError::Invalid)?;
        let mut found: HashMap<String, Vec<(Uuid, Vec<String>)>> = HashMap::new();
        for row in sqlx::query(
            "SELECT t.id, t.relative_path, \
                    substr(t.relative_path, \
                           length(rtrim(t.relative_path, \
                                        replace(t.relative_path, '/', ''))) + 1) AS file_name \
             FROM track t \
             JOIN library_member m ON m.library_id=t.library_id \
             WHERE m.user_id=? AND t.is_available=1 \
               AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?))) \
               AND file_name COLLATE NOCASE IN (SELECT value FROM json_each(?))",
        )
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(&wanted)
        .fetch_all(&mut *connection)
        .await?
        {
            let path: String = row.try_get("relative_path")?;
            let file_name: String = row.try_get("file_name")?;
            found
                .entry(file_name.to_ascii_lowercase())
                .or_default()
                .push((parse_uuid(row.try_get("id")?)?, lowercased_segments(&path)));
        }
        Ok(found)
    }

    /// The last rung: the `#EXTINF` credit, matched exactly.
    ///
    /// Asked only for the entries no path matched, and only for those whose
    /// title splits into an artist and a song — the convention every player
    /// writes and none guarantees. The comparison is against the title the
    /// account sees, correction included, because a corrected title is the
    /// title.
    ///
    /// Asked in one query rather than per entry: a file whose paths are all
    /// foreign would otherwise be one query per line, and the ceiling on a
    /// file is ten thousand lines. The concatenation is computed per row like
    /// the file name above, so this is a scan too — and for the same reason it
    /// is one scan and not one per slice.
    ///
    /// The key is the two fields joined by a newline, which is a separator
    /// neither of them can contain — `track.title` and `artist_display` come
    /// from tags read line by line. The folding is ASCII on both sides: SQLite
    /// matches `NOCASE`, and the map is keyed through `to_ascii_lowercase`
    /// rather than `to_lowercase` so the key cannot be wider than the filter
    /// that produced it.
    async fn tracks_by_credit_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        library_ids: &[Uuid],
        entries: &[&PlaylistFileEntry],
    ) -> Result<HashMap<String, Vec<Uuid>>, ServiceError> {
        let mut wanted: Vec<String> = entries
            .iter()
            .filter_map(|entry| entry.artist_and_title())
            .map(|(artist, title)| format!("{artist}\n{title}"))
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        let folders = folder_filter(library_ids);
        let keys = serde_json::to_string(&wanted).map_err(|_| ServiceError::Invalid)?;
        let mut found: HashMap<String, Vec<Uuid>> = HashMap::new();
        for row in sqlx::query(
            "SELECT t.id, \
                    t.artist_display || char(10) || COALESCE(ovr.title, t.title) AS credit \
             FROM track t \
             JOIN library_member m ON m.library_id=t.library_id \
             LEFT JOIN track_override ovr ON ovr.track_id=t.id \
             WHERE m.user_id=? AND t.is_available=1 \
               AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?))) \
               AND (t.artist_display || char(10) || COALESCE(ovr.title, t.title)) \
                   COLLATE NOCASE IN (SELECT value FROM json_each(?))",
        )
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(&keys)
        .fetch_all(&mut *connection)
        .await?
        {
            let credit: String = row.try_get("credit")?;
            found
                .entry(credit.to_ascii_lowercase())
                .or_default()
                .push(parse_uuid(row.try_get("id")?)?);
        }
        Ok(found)
    }

    /// Renders a playlist this account may read back as an extended `.m3u8`:
    /// its own, or another account's public one, which an export reads as any
    /// other read does — through this account's libraries.
    ///
    /// The paths are library-relative, which is what the import reads: a
    /// playlist exported here and imported again resolves by the same ladder,
    /// and that round trip is the strongest evidence the reader is right — a
    /// test asserts it rather than a reviewer believing it.
    ///
    /// It reads `relative_path` itself rather than through `SongItem`, which
    /// does not carry it. Putting a server-side path on every song of every
    /// listing to serve one export would widen a payload the whole catalogue
    /// answers with.
    ///
    /// UTF-8, hence `.m3u8`, with no cleverness about it: the only honest
    /// reason the other encoding exists is that old files were written in it,
    /// which is a reading problem rather than a writing one.
    ///
    /// Answers the name beside the file so the caller needs no second read for
    /// it. The handler had one, and this had another through `playlist_on`:
    /// two full hydrations of up to ten thousand `SongItem`s and their
    /// relations, to write a text file out of four columns.
    pub async fn export_playlist(
        &self,
        user_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<(String, String), ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        // The name, and the refusal: a playlist this account may not read
        // answers as one that does not exist. One column rather than
        // `playlist_on`, which would hydrate every track of it to read a
        // string.
        let name = sqlx::query_scalar::<_, String>(
            "SELECT p.name FROM playlist p JOIN account a ON a.id=p.owner_user_id \
             WHERE (p.owner_user_id=? OR (p.public=1 AND a.disabled=0)) AND p.id=?",
        )
        .bind(user_id.to_string())
        .bind(playlist_id.to_string())
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(ServiceError::NotFound)?;
        let rows = sqlx::query(
            "SELECT t.relative_path, COALESCE(ovr.title, t.title) AS title, \
                    t.artist_display, t.duration_ms \
             FROM playlist_track pt \
             JOIN playlist p ON p.id=pt.playlist_id \
             JOIN account a ON a.id=p.owner_user_id \
             JOIN track t ON t.id=pt.track_id \
             JOIN library_member m ON m.library_id=t.library_id \
             LEFT JOIN track_override ovr ON ovr.track_id=t.id \
             WHERE m.user_id=? AND (p.owner_user_id=? OR (p.public=1 AND a.disabled=0)) \
               AND p.id=? AND t.is_available=1 \
             ORDER BY pt.position",
        )
        .bind(user_id.to_string())
        .bind(user_id.to_string())
        .bind(playlist_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut out = String::from("#EXTM3U\n");
        out.push_str(&format!("#PLAYLIST:{}\n", one_line(&name)));
        for row in rows {
            let path: String = row.try_get("relative_path")?;
            let title: String = row.try_get("title")?;
            let artist: Option<String> = row.try_get("artist_display")?;
            let duration_ms: i64 = row.try_get("duration_ms")?;
            let credit = match artist.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
                Some(artist) => format!("{} - {}", one_line(artist), one_line(&title)),
                None => one_line(&title),
            };
            // `./`, and `%` written as `%25`. Both are about the reader on the
            // other side, which is also this one.
            //
            // A backslash needs no escape, and that is a measured claim rather
            // than an assumption: every `relative_path` in the catalogue is
            // written by `scanner::relative_path`, which folds `\` into `/`
            // before the row exists — `apply_catalog_tracks` is reached from
            // the scanner alone, and the metadata re-apply reuses the stored
            // string. So the source of this line cannot carry one. (That the
            // scanner turns a legal `\` in a Unix file name into a separator
            // is its own defect, upstream of here and not fixable from an
            // export.)
            //
            // A line starting with `#` is a directive by the format's own
            // definition, so a track at a library's root called `#1 Hit.flac`
            // written bare would be read as a comment and **disappear without
            // being reported** — not even counted. And a path holding a `%`
            // would come back percent-decoded, so `a%41.flac` would be read as
            // `aA.flac` and match nothing. The prefix is dropped as a `.`
            // segment and the escape decodes back to `%`, so what this writes
            // is what the import reads — which is the round trip the guide
            // claims and a test now asserts on exactly those two names.
            out.push_str(&format!(
                "#EXTINF:{},{}\n./{}\n",
                duration_ms / 1000,
                credit,
                one_line(&path).replace('%', "%25")
            ));
        }
        Ok((name, out))
    }
}

/// What the path rung made of one entry.
///
/// `Unmatched` is deliberately not `Ambiguous`: only the first has a rung left
/// to try, and collapsing the two into an `Option` would have sent an entry
/// the catalogue already answered twice on to a credit lookup that cannot
/// improve on it.
enum Rung {
    Found(Uuid),
    Ambiguous,
    NotAPath,
    Unmatched,
}

/// The path ladder: among the tracks sharing this entry's file name, the ones
/// that agree with it for as many trailing segments as possible.
///
/// Climbed from the longest tail down, and both sides are bounded by
/// themselves rather than by a constant — the entry may name four parents and
/// the catalogue two, or the other way round, and the deepest agreement either
/// one can express is the strongest answer available.
///
/// The first depth at which exactly one candidate agrees resolves the entry. A
/// depth where several agree ends the climb as ambiguous: a shorter tail is a
/// weaker question, so it cannot separate what this one could not. A depth
/// where none agree is simply too specific, and the next one down is tried.
fn climb(segments: &[String], by_file_name: &HashMap<String, Vec<(Uuid, Vec<String>)>>) -> Rung {
    let Some(file_name) = segments.last() else {
        return Rung::NotAPath;
    };
    let Some(candidates) = by_file_name.get(file_name) else {
        return Rung::Unmatched;
    };
    let deepest = candidates
        .iter()
        .map(|(_, stored)| stored.len())
        .max()
        .unwrap_or(1)
        .min(segments.len());
    for depth in (1..=deepest).rev() {
        let wanted = &segments[segments.len() - depth..];
        let agreeing = candidates
            .iter()
            .filter(|(_, stored)| {
                stored.len() >= depth && &stored[stored.len() - depth..] == wanted
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        match agreeing.as_slice() {
            [only] => return Rung::Found(*only),
            [_, ..] => return Rung::Ambiguous,
            [] => {}
        }
    }
    Rung::Unmatched
}

/// A stored path, split and folded the way an entry's segments are, so the two
/// can be compared without a second spelling of either rule.
fn lowercased_segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// A field flattened onto one line.
///
/// A playlist file is line-structured, so a newline inside a title would end
/// the line early and make the next one look like a path — a track called
/// "x\n/etc/passwd" would otherwise write an entry of its own. Tabs go too:
/// they are invisible in the output and some readers split on them.
fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '\r' | '\n' | '\t' => ' ',
            other => other,
        })
        .collect::<String>()
        .trim()
        .to_owned()
}
