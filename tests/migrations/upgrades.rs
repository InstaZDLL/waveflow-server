//! Every released schema upgrades with the rows it was holding, and the
//! rebuilt lists keep their order.

use super::*;

/// The crate's version is the tag it ships as, so it must name a line of
/// `RELEASED_SCHEMAS`, in `main.rs`.
///
/// A release is cut by a PR that bumps `version` and is then tagged, and the
/// version stays put until the next one. So this holds between releases, and
/// fails exactly on the bump that forgot the schema — before the tag exists,
/// which is the only moment the baseline is still the newest migration and
/// needs no archaeology to find.
#[test]
fn the_version_being_shipped_has_its_schema_recorded() {
    let tag = concat!("v", env!("CARGO_PKG_VERSION"));
    assert!(
        RELEASED_SCHEMAS
            .iter()
            .any(|(released, _)| *released == tag),
        "Cargo.toml is at {tag}, and RELEASED_SCHEMAS has no line for it: add \
         (\"{tag}\", <version of the newest file under migrations-v2/>)"
    );
}

#[tokio::test]
async fn every_released_schema_upgrades_with_the_rows_it_was_holding() {
    let total = MIGRATOR.iter().count();

    for (tag, baseline) in RELEASED_SCHEMAS {
        let temp = tempfile::tempdir().expect("temporary directory");
        let config = Config::for_data_dir(temp.path().join("data"));

        // Step one: a database at the released schema, holding rows.
        let database = Database::open(&config).await.expect("open the database");
        let mut connection = database.pool().acquire().await.expect("connection");
        connection
            .ensure_migrations_table(&MIGRATOR.table_name)
            .await
            .expect("migrations table");
        // A baseline that names no migration is a typo, and a typo here is
        // silent: the filter below would still apply *some* prefix, and every
        // assertion would then hold about a schema no tag ever shipped
        // (constat de revue, pris).
        assert!(
            MIGRATOR
                .iter()
                .any(|migration| migration.version == baseline),
            "{tag}: no migration has version {baseline}"
        );
        for migration in MIGRATOR
            .iter()
            .filter(|migration| migration.version <= baseline)
        {
            connection
                .apply(&MIGRATOR.table_name, migration)
                .await
                .unwrap_or_else(|error| {
                    panic!("{tag}: migration {} failed: {error}", migration.version)
                });
        }
        seed_an_installation(&mut connection).await;

        // **This is what keeps the test from being a fresh-install test in
        // disguise.** If the prefix silently applied everything, the rows
        // above would have been written into today's schema and the upgrade
        // below would have nothing left to do — green, on the right lines, and
        // proving none of what it claims.
        let applied_before = count(&mut connection, "SELECT COUNT(*) FROM _sqlx_migrations").await;
        assert!(
            (applied_before as usize) < total,
            "{tag}: the baseline applied all {total} migrations, so nothing is being upgraded"
        );
        drop(connection);
        database.pool().close().await;
        drop(database);

        // Step two: the upgrade, through the boot path rather than a bare
        // `migrate()` call — an installation that updates runs all of it, and
        // the key binding and identity reconciliation read the same rows.
        let state = waveflow_server::initialize(&config)
            .await
            .unwrap_or_else(|error| panic!("{tag}: a populated database refused to boot: {error}"));

        let mut connection = state.db.pool().acquire().await.expect("connection");
        let applied_after = count(&mut connection, "SELECT COUNT(*) FROM _sqlx_migrations").await;
        assert_eq!(
            applied_after as usize, total,
            "{tag}: the upgrade left migrations unapplied"
        );

        // Step three: the rows are still there — and still *mean* something.
        // A count proves the statements ran; reading them back through the
        // services proves the upgraded schema is the one the server queries.
        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *connection)
            .await
            .expect("foreign key check");
        assert!(
            violations.is_empty(),
            "{tag}: the upgrade left {} broken reference(s)",
            violations.len()
        );
        drop(connection);

        let account: uuid::Uuid = ACCOUNT.parse().expect("account id");
        let album: uuid::Uuid = ALBUM.parse().expect("album id");
        let track: uuid::Uuid = TRACK.parse().expect("track id");

        let albums = state
            .services
            .list_albums(account, &AlbumListQuery::default())
            .await
            .expect("list albums");
        assert_eq!(albums.len(), 1, "{tag}: the album did not survive");
        assert_eq!(albums[0].title, "Convergence", "{tag}");
        assert_eq!(
            albums[0]
                .artists
                .iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Nova Kern"],
            "{tag}: the album credit did not survive the upgrade"
        );

        let found = state
            .services
            .catalog_search(
                account,
                &[],
                "Zenith",
                BrowsePage::default(),
                BrowsePage::default(),
                BrowsePage::default(),
            )
            .await
            .expect("search");
        assert_eq!(
            found.songs.len(),
            1,
            "{tag}: the search index did not come through the upgrade"
        );

        let playlists = state.services.playlists(account).await.expect("playlists");
        assert_eq!(playlists.len(), 1, "{tag}: the playlist did not survive");
        assert_eq!(playlists[0].name, "Carried over", "{tag}");

        // The three relations the fixture carries that nothing read back
        // until now (constat de revue, pris). A seeded row no assertion reads
        // is how a fixture comes to claim more than it proves — and the
        // second credit is the one that matters most here, because a credit
        // model that could only ever see the first is exactly what the
        // participants rebuild replaced.
        let playlist = state
            .services
            .playlist(account, PLAYLIST.parse().expect("playlist id"))
            .await
            .expect("playlist");
        assert_eq!(
            playlist
                .songs
                .iter()
                .map(|song| song.id)
                .collect::<Vec<_>>(),
            vec![track],
            "{tag}: the playlist lost the track it held"
        );
        assert_eq!(
            playlist.songs[0]
                .artists
                .iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Nova Kern", "Vale Ortiz"],
            "{tag}: a credit did not survive the upgrade"
        );
        assert_eq!(
            playlist.songs[0].genres,
            vec!["Ambient".to_owned()],
            "{tag}: the genre relation did not survive the upgrade"
        );

        let starred = state.services.starred(account, &[]).await.expect("starred");
        assert_eq!(
            starred
                .albums
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![album],
            "{tag}: the favourite did not survive"
        );

        // Listening history is the one thing a rescan cannot rebuild, and the
        // only reader that proves the rows are still joinable to a track.
        let top = state
            .services
            .artist_top_songs_by_name(account, "Nova Kern", 10)
            .await
            .expect("top songs");
        assert_eq!(
            top.len(),
            1,
            "{tag}: the play history did not survive the upgrade"
        );
        assert_eq!(top[0].title, "Zenith", "{tag}");

        // The scan history, through the table RFC-021 rebuilt to admit two
        // more triggers: the job, its error, and the track that names it as
        // the scan that last saw it. A rebuild run with foreign keys on would
        // have cascaded the error away and nulled the track's reference — and
        // a null there makes the next scan treat the track as never seen.
        let scan: uuid::Uuid = SCAN.parse().expect("scan id");
        let job = state
            .db
            .scan_job(scan)
            .await
            .expect("read the job")
            .unwrap_or_else(|| panic!("{tag}: the scan history did not survive"));
        assert_eq!(
            (job.status.as_str(), job.added, job.errors),
            ("completed", 1, 1),
            "{tag}"
        );
        assert_eq!((job.excluded, job.skipped_links), (0, 0), "{tag}");
        let mut connection = state.db.pool().acquire().await.expect("connection");
        assert_eq!(
            count(
                &mut connection,
                "SELECT COUNT(*) FROM scan_error WHERE scan_id = '6f1d1b6e-0f1a-4a3d-9c21-0000000000a9'"
            )
            .await,
            1,
            "{tag}: the scan's error did not survive"
        );
        assert_eq!(
            count(
                &mut connection,
                "SELECT COUNT(*) FROM track WHERE last_seen_scan_id = '6f1d1b6e-0f1a-4a3d-9c21-0000000000a9'"
            )
            .await,
            1,
            "{tag}: the track lost the scan that last saw it"
        );
    }
}

const TRACK_TWO: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000a9";

const SHARE: &str = "6f1d1b6e-0f1a-4a3d-9c21-0000000000aa";

/// The two lists RFC-020 rebuilt come through with every row, in its order,
/// and take a repeat afterwards.
///
/// `playlist_track` and `share_track` were keyed on `(list, track)` and are
/// keyed on `(list, position)` since `20261026000000`. A rebuild is a copy into
/// a new table, and the failure it invites is quiet: rows dropped by a
/// mistyped column, or positions renumbered so the order changes. So the
/// fixture stores its two tracks **against** their insertion order — the
/// second track first — which a rebuild that numbered rows as it read them
/// would get wrong, and reads them back through the services. Then it adds a
/// copy of a track the playlist already holds, which is what the new key is
/// for and what the old one refused.
#[tokio::test]
async fn the_rebuilt_lists_keep_their_rows_in_order_and_then_take_a_repeat() {
    for (tag, baseline) in RELEASED_SCHEMAS {
        let temp = tempfile::tempdir().expect("temporary directory");
        let config = Config::for_data_dir(temp.path().join("data"));
        let database = Database::open(&config).await.expect("open the database");
        let mut connection = database.pool().acquire().await.expect("connection");
        connection
            .ensure_migrations_table(&MIGRATOR.table_name)
            .await
            .expect("migrations table");
        for migration in MIGRATOR
            .iter()
            .filter(|migration| migration.version <= baseline)
        {
            connection
                .apply(&MIGRATOR.table_name, migration)
                .await
                .unwrap_or_else(|error| {
                    panic!("{tag}: migration {} failed: {error}", migration.version)
                });
        }
        seed_an_installation(&mut connection).await;
        for (statement, binds) in [
            (
                "INSERT INTO track (id, library_id, album_id, relative_path, file_size, \
                                    file_modified_at, quick_hash, full_hash, title, album_title, \
                                    artist_display, duration_ms, is_available, created_at, \
                                    updated_at) \
                   VALUES (?, ?, ?, 'convergence/02.flac', 4096, 1000, ?, ?, 'Nadir', \
                           'Convergence', 'Nova Kern', 180000, 1, 1000, 1000)",
                vec![TRACK_TWO, LIBRARY, ALBUM, &"5".repeat(64), &"6".repeat(64)],
            ),
            // The fixture's playlist already holds TRACK at position 0; it is
            // moved behind the new one, so the stored order is not the order
            // the rows were written in.
            (
                "UPDATE playlist_track SET position=1 WHERE playlist_id=? AND track_id=?",
                vec![PLAYLIST, TRACK],
            ),
            (
                "INSERT INTO playlist_track (playlist_id, track_id, position, added_at) \
                   VALUES (?, ?, 0, 1000)",
                vec![PLAYLIST, TRACK_TWO],
            ),
            (
                "INSERT INTO share (id, owner_user_id, token_hash, description, created_at, \
                                    updated_at, visit_count) \
                   VALUES (?, ?, randomblob(32), 'Carried share', 1000, 1000, 7)",
                vec![SHARE, ACCOUNT],
            ),
            (
                "INSERT INTO share_track (share_id, track_id, position) VALUES (?, ?, 1)",
                vec![SHARE, TRACK],
            ),
            (
                "INSERT INTO share_track (share_id, track_id, position) VALUES (?, ?, 0)",
                vec![SHARE, TRACK_TWO],
            ),
        ] {
            let mut query = sqlx::query(statement);
            for bind in binds {
                query = query.bind(bind.to_owned());
            }
            query
                .execute(&mut *connection)
                .await
                .unwrap_or_else(|error| panic!("{tag}: `{statement}` failed: {error}"));
        }
        drop(connection);
        database.pool().close().await;
        drop(database);

        let state = waveflow_server::initialize(&config)
            .await
            .unwrap_or_else(|error| panic!("{tag}: a populated database refused to boot: {error}"));
        let account: uuid::Uuid = ACCOUNT.parse().expect("account id");
        let playlist_id: uuid::Uuid = PLAYLIST.parse().expect("playlist id");
        let track: uuid::Uuid = TRACK.parse().expect("track id");
        let track_two: uuid::Uuid = TRACK_TWO.parse().expect("track id");

        let playlist = state
            .services
            .playlist(account, playlist_id)
            .await
            .expect("playlist");
        assert_eq!(
            playlist
                .songs
                .iter()
                .map(|song| song.id)
                .collect::<Vec<_>>(),
            vec![track_two, track],
            "{tag}: the playlist lost a row or its order"
        );
        assert_eq!(playlist.owner_username, "upgrade-owner", "{tag}");

        let shares = state.services.shares(account).await.expect("shares");
        assert_eq!(shares.len(), 1, "{tag}: the share did not survive");
        assert_eq!(shares[0].visit_count, 7, "{tag}");
        assert_eq!(shares[0].max_visits, None, "{tag}: an old share has no cap");
        assert_eq!(
            shares[0]
                .songs
                .iter()
                .map(|song| song.id)
                .collect::<Vec<_>>(),
            vec![track_two, track],
            "{tag}: the share lost a row or its order"
        );

        let repeated = state
            .services
            .update_playlist(
                account,
                playlist_id,
                None,
                None,
                None,
                &[track_two],
                &[],
                Default::default(),
            )
            .await
            .unwrap_or_else(|error| panic!("{tag}: a repeat was refused: {error}"));
        assert_eq!(
            repeated
                .songs
                .iter()
                .map(|song| song.id)
                .collect::<Vec<_>>(),
            vec![track_two, track, track_two],
            "{tag}"
        );
    }
}
