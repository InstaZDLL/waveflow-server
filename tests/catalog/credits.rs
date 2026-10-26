//! Credits and participants: how a credit is cut, and who counts as an artist
//! of a track or album.

use super::*;

/// An album artist tag holding two credits is two artists, and the album now
/// hangs off both.
///
/// Feeding the joined string to the artist table minted an entity named after
/// it, gave that entity the album, and left both real artists with nothing:
/// DSub browsed to either and found no album. Splitting the credit fixed the
/// entity but not the browse — the album still pointed at one artist through a
/// single column, so the second credit found nothing. The album's participants
/// are what answer now, and both credits reach it.
#[tokio::test]
async fn an_album_hangs_off_every_artist_it_is_credited_to() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("credits", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("credit-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Credits",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 4, false).await.unwrap();

    // Four albums whose credits differ only in where the boundaries fall.
    for (index, album, credit) in [
        (0usize, "Live", "Nova Kern; Lior Sand"),
        (1, "Live", "Nova Kern; Ada Vale"),
        (2, "Split", "A; B C"),
        (3, "Split", "A; B; C"),
    ] {
        let mut input = catalog_input(index, credit);
        input.title = format!("Track {index}");
        input.album = Some(album.into());
        input.album_artist = Some(credit.into());
        input.is_compilation = false;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();

    // No entity is named after a joined string.
    let artists = state
        .services
        .list_artists(owner, None, Default::default())
        .await
        .unwrap();
    let mut names: Vec<String> = artists
        .iter()
        .map(|summary| summary.artist.name.clone())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "A".to_owned(),
            "Ada Vale".to_owned(),
            "B".to_owned(),
            "B C".to_owned(),
            "C".to_owned(),
            "Lior Sand".to_owned(),
            "Nova Kern".to_owned(),
        ],
        "each credit is its own artist and the joined string is nobody"
    );

    // Sharing a title and a lead credit is not sharing an album.
    let albums = state
        .services
        .list_albums(owner, &Default::default())
        .await
        .unwrap();
    assert_eq!(
        albums.iter().filter(|album| album.title == "Live").count(),
        2,
        "`A; B` and `A; C` are two records, not one"
    );
    assert_eq!(
        albums.iter().filter(|album| album.title == "Split").count(),
        2,
        "where the boundary falls is part of the identity"
    );

    // And the second credit reaches the album, which is what the single
    // `album_artist_id` column could never answer.
    let by_name = |name: &str| {
        artists
            .iter()
            .find(|summary| summary.artist.name == name)
            .unwrap_or_else(|| panic!("{name} was indexed"))
            .artist
            .id
    };
    for (name, credited_on) in [("Nova Kern", 2), ("Lior Sand", 1), ("Ada Vale", 1)] {
        let detail = state.services.artist(owner, by_name(name)).await.unwrap();
        assert_eq!(
            detail.albums.len(),
            credited_on,
            "browsing to {name} finds every album it is credited to"
        );
        assert!(detail.albums.iter().all(|album| album.title == "Live"));
        assert_eq!(
            detail.album_count, credited_on as i64,
            "{name}'s album count counts the credits, not a single column"
        );
    }

    // The display string stays what the file wrote, joins and all.
    let live = albums
        .iter()
        .find(|album| album.title == "Live")
        .expect("an album titled Live");
    assert!(
        live.artist
            .as_deref()
            .is_some_and(|credit| credit.contains("; ")),
        "the album still renders the credit as written: {:?}",
        live.artist
    );
}

/// The separators the reference cuts on, reaching the catalogue.
///
/// The old rule cut on `;` and nothing else, so a file crediting
/// "Nova Kern / Lior Sand" held one artist named after the whole string. The
/// new one cuts on a padded slash — padded so that `AC/DC`, which is one band,
/// survives it.
#[tokio::test]
async fn the_catalogue_cuts_a_credit_where_the_reference_cuts_it() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("separators", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("separator-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Separators",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 3, false).await.unwrap();

    for (index, credit, album) in [
        (0usize, "Nova Kern / Lior Sand", "Split By Slash"),
        (1, "AC/DC", "Kept Whole"),
        (2, "Ada Vale feat. Nova Kern", "Split By Feat"),
    ] {
        let mut input = catalog_input(index, credit);
        input.title = format!("Track {index}");
        input.album = Some(album.into());
        input.album_artist = Some(credit.into());
        input.is_compilation = false;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let artists = state
        .services
        .list_artists(owner, None, Default::default())
        .await
        .unwrap();
    let mut names: Vec<String> = artists
        .into_iter()
        .map(|summary| summary.artist.name)
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "AC/DC".to_owned(),
            "Ada Vale".to_owned(),
            "Lior Sand".to_owned(),
            "Nova Kern".to_owned(),
        ],
        "a padded slash cuts, a bare one inside a name does not, and `feat.` cuts"
    );
}

#[tokio::test]
async fn the_participant_schema_replaced_the_single_artist_relation() {
    let (_temp, _config, state) = test_app().await;
    let table_exists = |name: &'static str| {
        let pool = state.db.pool().clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            )
            .bind(name)
            .fetch_one(&pool)
            .await
            .unwrap()
                == 1
        }
    };
    assert!(table_exists("track_participant").await);
    assert!(table_exists("album_participant").await);
    assert!(table_exists("artist_role_stats").await);
    assert!(
        !table_exists("track_artist").await,
        "the single-artist relation is gone, not shadowed"
    );
}

/// A producer is a credit, not an artist of the track.
///
/// This is the failure the participants model makes possible and no existing
/// test could have anticipated: widening `track_participant` to hold every
/// role, and leaving one projection without its role predicate, does not turn
/// the suite red — it leaves it green while every song reports its producer
/// among its artists, and every album reports them among its own.
#[tokio::test]
async fn a_contributor_is_not_one_of_the_track_artists() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("contributors", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("contributor-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Credits",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();

    let mut input = catalog_input(0, "Nova Kern");
    input.title = "Only Track".into();
    input.album = Some("Only Album".into());
    input.album_artist = Some("Nova Kern".into());
    input.is_compilation = false;
    input.roles = vec![
        (
            waveflow_server::tags::Role::Producer,
            vec!["Rita Sound".into()],
        ),
        (
            waveflow_server::tags::Role::Composer,
            vec!["Otto Pen".into()],
        ),
    ];
    input.performer_pairs = vec![("guitar".into(), "Jimmy Page".into())];
    state
        .db
        .apply_catalog_track(library, scan, &input, None, false)
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();

    // Every credit reached the catalogue...
    let artists = state
        .services
        .list_artists(owner, None, Default::default())
        .await
        .unwrap();
    let mut names: Vec<String> = artists
        .iter()
        .map(|summary| summary.artist.name.clone())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "Jimmy Page".to_owned(),
            "Nova Kern".to_owned(),
            "Otto Pen".to_owned(),
            "Rita Sound".to_owned(),
        ]
    );

    // ...and none of them is one of the track's artists.
    let songs = state
        .services
        .songs_without_album(owner, library, 100)
        .await
        .unwrap();
    let song = state
        .services
        .album(owner, {
            let albums = state
                .services
                .list_albums(owner, &Default::default())
                .await
                .unwrap();
            albums[0].id
        })
        .await
        .unwrap();
    assert!(songs.is_empty(), "the track belongs to an album");
    let track = &song.songs[0];
    assert_eq!(
        track
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Nova Kern"],
        "artists[] holds the track's own credit and nothing else"
    );
    assert_eq!(
        song.album
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Nova Kern"],
        "an album's artists[] holds its album artists, not every contributor"
    );
    let credited = state
        .services
        .artist(
            owner,
            artists
                .iter()
                .find(|summary| summary.artist.name == "Rita Sound")
                .expect("the producer was indexed")
                .artist
                .id,
        )
        .await
        .unwrap();
    assert_eq!(
        credited.album_count, 0,
        "a producer holds no album of their own"
    );
    assert!(credited.albums.is_empty());

    // ...and none of them answers to a query that names none of them.
    //
    // The artist half of a search used to be derived rather than matched:
    // it took the tracks the full-text index had found and returned
    // everybody credited on them. Searching a title therefore returned the
    // whole session crew, and the participants model made that worse — a
    // track carries thirteen roles now where it carried one list of names.
    let by_title = state
        .services
        .catalog_search(
            owner,
            &[],
            "Only Track",
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        by_title.songs.len(),
        1,
        "the title still finds the track it names"
    );
    assert!(
        by_title.artists.is_empty(),
        "a track title names no artist, so it returns none: {:?}",
        by_title
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>()
    );

    // The producer is still reachable — by their own name, which is the
    // whole point of indexing artists rather than deriving them.
    let by_name = state
        .services
        .catalog_search(
            owner,
            &[],
            "Rita",
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        by_name
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Rita Sound"]
    );
}
