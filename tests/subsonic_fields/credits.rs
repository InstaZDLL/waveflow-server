//! Artist references and credits on the wire, in both encodings.

use super::*;

/// Two artist projections that are deliberately not the same shape, pinned in
/// both encodings so neither drifts into the other.
///
/// `artists[]` and `albumArtists[]` are *references*: an identifier and a
/// display name, and nothing else. The entries `getMusicDirectory` renders as
/// `child` are the artist and album nodes themselves, minus `musicBrainzId`,
/// so they do carry `sortName` and the rest. A field added to the node reaches
/// the second and must not leak into the first.
#[tokio::test]
async fn an_artist_reference_is_not_an_artist_record() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_reference-key";
    let admin = state
        .db
        .create_account(
            "reference-admin",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state.secret_box.encrypt(b"subsonic-secret-123").unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("reference-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(admin, "Refs", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    let mut input = catalog_input(0, "The Nocturnes");
    input.title = "Opening".into();
    input.album = Some("The Night Sessions".into());
    input.album_artist = Some("The Nocturnes".into());
    input.is_compilation = false;
    input.sort_album = Some("Night Sessions, The".into());
    input.sort_album_artist = Some("Nocturnes, The".into());
    input.sort_artist = Some("Nocturnes, The".into());
    // A contributor, so the whitelist below actually has one to check.
    input.roles = vec![(
        waveflow_server::library::tags::Role::Producer,
        vec!["Rita Sound".into()],
    )];
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
    state.db.consolidate_sort_names(library).await.unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(&config, state.clone());

    // JSON: the album carries its own sortName; its `artists[]` entry carries
    // an identifier and a name, and no third key.
    let albums = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=alphabeticalByName",
    )
    .await;
    let album = albums["subsonic-response"]["albumList2"]["album"][0].clone();
    assert_eq!(album["sortName"], serde_json::json!("Night Sessions, The"));
    let reference_keys = |value: &serde_json::Value, what: &str| {
        let entry = value
            .as_object()
            .unwrap_or_else(|| panic!("{what} is an object: {value}"))
            .clone();
        let mut keys: Vec<String> = entry.keys().cloned().collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["id".to_owned(), "name".to_owned()],
            "{what} is a reference, not an ArtistID3: {entry:?}"
        );
    };
    reference_keys(&album["artists"][0], "an album's artists[] entry");

    // `albumArtists[]` is a reference too, and it is emitted on media items
    // rather than on the album — the album's own credit is `artists[]`. So it
    // is pinned on a song of the album.
    let album_json = subsonic_json(
        &router,
        "getAlbum",
        api_key,
        &format!("&id={}", album["id"].as_str().expect("the album id")),
    )
    .await;
    let song = album_json["subsonic-response"]["album"]["song"][0].clone();
    reference_keys(&song["artists"][0], "a song's artists[] entry");
    reference_keys(&song["albumArtists"][0], "a song's albumArtists[] entry");
    // A contributor's artist is a reference on the same terms, and the
    // contributor itself carries only what names the credit.
    // Unconditional: the fixture credits a producer, so a contributor that
    // stopped being emitted would fail here rather than skip the check.
    {
        let credit = song["contributors"][0].clone();
        reference_keys(&credit["artist"], "a contributor's artist");
        let mut keys: Vec<String> = credit
            .as_object()
            .expect("a contributor is an object")
            .keys()
            .cloned()
            .collect();
        keys.sort_unstable();
        assert!(
            keys == vec!["artist".to_owned(), "role".to_owned()]
                || keys == vec!["artist".to_owned(), "role".to_owned(), "subRole".to_owned()],
            "a contributor names the credit and nothing more: {credit:?}"
        );
    }

    // XML: the same statement, in the encoding where an absent attribute is
    // absent rather than a missing key.
    let directory = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getMusicDirectory.view?apiKey={api_key}&v=1.16.1&c=fixtures&id={library}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(directory.status(), StatusCode::OK);
    let directory = body_text(directory).await;
    // The browsing child is the artist node minus musicBrainzId, so the sort
    // name reaches it, carrying the tagged value.
    assert!(
        directory.contains(r#"sortName="Nocturnes, The""#),
        "a getMusicDirectory child carries the artist's sortName: {directory}"
    );
    // ...while the references carry neither that field nor any other one
    // belonging to the record. They live on the album and on its songs, so
    // this reads the album document rather than the folder listing — where a
    // library holding no album-less track has no song child at all.
    let album_xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAlbum.view?apiKey={api_key}&v=1.16.1&c=fixtures&id={}",
                album["id"].as_str().expect("the album id")
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(album_xml.status(), StatusCode::OK);
    let album_xml = body_text(album_xml).await;
    for name in ["<artists ", "<albumArtists "] {
        let mut references = 0;
        for element in album_xml.split(name).skip(1) {
            let element = element.split("/>").next().unwrap_or_default();
            references += 1;
            // The whitelist, rather than a list of fields to reject: a field
            // added to the artist node has to fail here even if nobody thought
            // to name it.
            // Split on the quote rather than on whitespace: an attribute
            // value holds spaces, and `name="The Nocturnes"` would otherwise
            // read as two attributes.
            let mut attributes: Vec<String> = element
                .split('"')
                .step_by(2)
                .map(|key| key.trim().trim_end_matches('=').trim().to_owned())
                .filter(|key| !key.is_empty())
                .collect();
            // Sorted before comparing, like the JSON side: which attributes
            // are present is the contract, the order they are written in is
            // not, and pinning it would fail a reordering that changes
            // nothing observable.
            attributes.sort_unstable();
            assert_eq!(
                attributes,
                vec!["id".to_owned(), "name".to_owned()],
                "{name} is a reference, not an ArtistID3: {element}"
            );
        }
        assert!(
            references > 0,
            "the fixture must exercise {name}: {album_xml}"
        );
    }
}

/// The credits OpenSubsonic asks for, and the presence rule they follow.
///
/// These three fields were absent because the columns they need did not
/// exist — and under the presence rule absent is a statement: it says the
/// server does not read them. Now that it does, they are emitted with their
/// default on a track that names nobody, which is the difference between
/// "unsupported" and "this file credits no composer".
#[tokio::test]
async fn credits_reach_the_wire_in_both_encodings() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_credits-key";
    let admin = state
        .db
        .create_account(
            "credit-admin",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state.secret_box.encrypt(b"subsonic-secret-123").unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("credit-wire");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            admin,
            "Credits",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 2, false).await.unwrap();

    let mut credited = catalog_input(0, "Nova Kern");
    credited.title = "Credited".into();
    credited.album = Some("The Record".into());
    credited.album_artist = Some("Nova Kern; Lior Sand".into());
    credited.is_compilation = false;
    credited.roles = vec![
        (
            waveflow_server::library::tags::Role::Composer,
            vec!["Otto Pen; Ada Vale".into()],
        ),
        (
            waveflow_server::library::tags::Role::Producer,
            vec!["Rita Sound".into()],
        ),
    ];
    credited.performer_pairs = vec![("guitar".into(), "Jimmy Page".into())];
    state
        .db
        .apply_catalog_track(library, scan, &credited, None, false)
        .await
        .unwrap();

    let mut bare = catalog_input(1, "Nova Kern");
    bare.title = "Bare".into();
    bare.album = Some("The Record".into());
    bare.album_artist = Some("Nova Kern; Lior Sand".into());
    bare.is_compilation = false;
    state
        .db
        .apply_catalog_track(library, scan, &bare, None, false)
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let albums = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=alphabeticalByName",
    )
    .await;
    let album_id = albums["subsonic-response"]["albumList2"]["album"][0]["id"]
        .as_str()
        .expect("the album is listed")
        .to_owned();
    let album = subsonic_json(&router, "getAlbum", api_key, &format!("&id={album_id}")).await;
    let songs = album["subsonic-response"]["album"]["song"]
        .as_array()
        .expect("the album lists its songs")
        .clone();
    let song = |title: &str| {
        songs
            .iter()
            .find(|song| song["title"] == title)
            .unwrap_or_else(|| panic!("missing {title}"))
            .clone()
    };

    let credited = song("Credited");
    let contributors = credited["contributors"]
        .as_array()
        .expect("contributors is an array");
    let mut named: Vec<(String, String)> = contributors
        .iter()
        .map(|credit| {
            (
                credit["role"].as_str().unwrap().to_owned(),
                credit["artist"]["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    named.sort();
    assert_eq!(
        named,
        vec![
            ("composer".to_owned(), "Ada Vale".to_owned()),
            ("composer".to_owned(), "Otto Pen".to_owned()),
            ("performer".to_owned(), "Jimmy Page".to_owned()),
            ("producer".to_owned(), "Rita Sound".to_owned()),
        ],
        "every role but artist and albumartist is a contributor"
    );
    let performer = contributors
        .iter()
        .find(|credit| credit["role"] == "performer")
        .expect("the performer is credited");
    assert_eq!(
        performer["subRole"], "Guitar",
        "a performer carries the instrument, title-cased"
    );
    assert_eq!(
        credited["displayComposer"], "Otto Pen \u{2022} Ada Vale",
        "the composers, in tag order, joined the way the reference joins them"
    );
    let album_artists: Vec<&str> = credited["albumArtists"]
        .as_array()
        .expect("albumArtists is an array")
        .iter()
        .map(|artist| artist["name"].as_str().unwrap())
        .collect();
    assert_eq!(album_artists, vec!["Nova Kern", "Lior Sand"]);

    let bare = song("Bare");
    assert_eq!(bare["contributors"], serde_json::json!([]));
    assert_eq!(bare["displayComposer"], "");

    let artists = subsonic_json(&router, "getArtists", api_key, "").await;
    let indexed: Vec<serde_json::Value> = artists["subsonic-response"]["artists"]["index"]
        .as_array()
        .expect("the artist index")
        .iter()
        .flat_map(|index| index["artist"].as_array().unwrap().clone())
        .collect();
    let named = |name: &str| {
        indexed
            .iter()
            .find(|artist| artist["name"] == name)
            .unwrap_or_else(|| panic!("{name} is indexed"))
            .clone()
    };
    assert_eq!(
        named("Nova Kern")["roles"],
        serde_json::json!(["albumartist", "artist"]),
        "an album artist who also performs says both, in a stable order"
    );
    // A composer holds no album, so the index does not list them — but the
    // artist is still in the catalogue, findable by name, and still says what
    // it is. The second half is what makes the first acceptable: filtering the
    // index would otherwise put a credited artist out of reach entirely.
    let found = subsonic_json(&router, "search3", api_key, "&query=Otto").await;
    let found_artists: Vec<&str> = found["subsonic-response"]["searchResult3"]["artist"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|artist| artist["name"].as_str())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        found_artists.contains(&"Otto Pen"),
        "a composer is reachable by searching their own name: {found_artists:?}"
    );
    assert!(
        !indexed.iter().any(|artist| artist["name"] == "Otto Pen"),
        "an artist credited on no album is not one of the library's artists"
    );
    let catalogue = state
        .services
        .list_artists(admin, None, Default::default())
        .await
        .unwrap();
    let composer = catalogue
        .iter()
        .find(|summary| summary.artist.name == "Otto Pen")
        .expect("the composer is in the catalogue");
    assert_eq!(composer.artist.roles, vec!["composer".to_owned()]);

    // An artist whose credits nothing has counted yet still says the field is
    // supported. The presence rule turns on emitting the default, and a record
    // that omits its own array is a server saying it does not read roles at
    // all — which is what the classification above must not do.
    let indexed_artist = named("Nova Kern");
    assert!(
        indexed_artist["roles"].is_array(),
        "a full artist record carries its roles array: {indexed_artist:?}"
    );
    // The same record through a second surface, because `getStarred2` builds
    // its artists from the same projection and the same node.
    let artist_id = indexed_artist["id"].as_str().expect("the artist id");
    subsonic_json(&router, "star", api_key, &format!("&artistId={artist_id}")).await;
    let starred = subsonic_json(&router, "getStarred2", api_key, "").await;
    let starred_artist = starred["subsonic-response"]["starred2"]["artist"]
        .as_array()
        .expect("the starred artists are an array")
        .iter()
        .find(|artist| artist["id"] == artist_id)
        .expect("the artist that was just starred")
        .clone();
    assert!(
        starred_artist["roles"].is_array(),
        "and carries it there too: {starred_artist:?}"
    );
    // A browsing child is an artist or an album under the element name a song
    // uses, and only `isDir` tells them apart. A folder entry must not answer
    // with a track's relations: an artist reporting `isrc: []` would be the
    // server claiming it read a recording identifier off a directory.
    let folders = subsonic_json(&router, "getMusicFolders", api_key, "").await;
    let folder_id = folders["subsonic-response"]["musicFolders"]["musicFolder"][0]["id"]
        .as_str()
        .expect("the library is listed")
        .to_owned();
    let directory = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={folder_id}"),
    )
    .await;
    let entry = directory["subsonic-response"]["directory"]["child"]
        .as_array()
        .and_then(|children| children.iter().find(|child| child["isDir"] == true))
        .expect("the folder lists its artists")
        .clone();
    for absent in [
        "isrc",
        "moods",
        "albumArtists",
        "contributors",
        "artists",
        "genres",
    ] {
        assert!(
            entry.get(absent).is_none(),
            "a directory entry carries no {absent}: {entry:?}"
        );
    }
    assert!(
        entry["roles"].is_array(),
        "but it keeps the artist record shape it is rendered from: {entry:?}"
    );

    let xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAlbum.view?apiKey={api_key}&v=1.16.1&c=fixtures&id={album_id}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let xml = body_text(xml).await;
    assert!(
        xml.contains("<contributors role=\"performer\" subRole=\"Guitar\">"),
        "the performer credit is an element carrying its instrument: {xml}"
    );
    assert!(
        xml.contains("displayComposer=\"Otto Pen \u{2022} Ada Vale\""),
        "the composer display string reaches XML too: {xml}"
    );
    // And the case that needs the field injected rather than rendered: an
    // artist whose files have all gone missing keeps its row and its credits,
    // but nothing counts them any more, so it holds no role at all. It still
    // has to say the field is supported.
    sqlx::query("UPDATE track SET is_available = 0 WHERE library_id = ?")
        .bind(library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    let bereft = subsonic_json(&router, "getArtist", api_key, &format!("&id={artist_id}")).await;
    assert_eq!(
        bereft["subsonic-response"]["artist"]["roles"],
        serde_json::json!([]),
        "empty, not absent: absent would say the server does not read roles"
    );
}
