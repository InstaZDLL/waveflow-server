//! The disk tree and filed folders of a library.

use super::*;

/// The disk tree (RFC-012 decision 1), through the router: one level at a
/// time, derived from the indexed paths, open to every member and to nobody
/// else.
///
/// The paths carry a non-ASCII segment because SQLite's `substr` counts
/// characters, and a neighbour that shares the folder's name as a prefix
/// (`Björk-B`) because the range that selects "beneath" must stop at the
/// separator. Passwords are drawn at run time, for CodeQL.
#[tokio::test]
async fn the_disk_tree_lists_one_level_of_the_indexed_paths_to_members_only() {
    let (_temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let mut accounts = Vec::new();
    for name in ["tree-owner", "tree-listener", "tree-stranger"] {
        accounts.push(
            state
                .db
                .create_account(name, &hash, AccountRole::User, now_ms())
                .await
                .unwrap(),
        );
    }
    let (owner, listener) = (accounts[0], accounts[1]);
    let root = std::fs::canonicalize(&config.data_dir).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Tree", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner,
            library_id,
            listener,
            waveflow_server::storage::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    let files: [(&str, &str, Option<&str>); 11] = [
        (
            "Björk/Post/02 Hyperballad.flac",
            "Hyperballad",
            Some("Björk"),
        ),
        ("Björk/Post/01 Army Of Me.flac", "Army Of Me", Some("Björk")),
        (
            "Björk/Post/03 Possibly Maybe.flac",
            "Possibly Maybe",
            Some("Björk"),
        ),
        ("Björk/Post/Bonus/01 Extra.flac", "Extra", Some("Björk")),
        ("Björk/Homogenic/01 Hunter.flac", "Hunter", Some("Björk")),
        ("Björk-B/Side.flac", "Side", Some("Other")),
        ("Loose.flac", "Loose", None),
        (".waveflow-managed/3f.flac", "Zebra", Some("zed")),
        (".waveflow-managed/1a.flac", "Nameless", None),
        (".waveflow-managed/2b.flac", "Waterloo", Some("ABBA")),
        ("Gone/Missing.flac", "Missing", Some("Ghost")),
    ];
    state
        .db
        .start_scan_job(scan_id, files.len() as i64, false)
        .await
        .unwrap();
    for (index, (path, title, artist)) in files.iter().enumerate() {
        let mut input = browse_input(20_000 + index, title, "Album", "x", None, None);
        input.relative_path = (*path).to_owned();
        input.artist = artist.map(str::to_owned);
        input.album_artist = artist.map(str::to_owned);
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();
    // A track whose file is gone holds no folder open.
    sqlx::query("UPDATE track SET is_available = 0 WHERE relative_path = 'Gone/Missing.flac'")
        .execute(state.db.pool())
        .await
        .unwrap();
    // Nor is it counted in a folder that still has others: `Post` keeps two
    // tracks, and the counts, totals and pages below say two, never three.
    sqlx::query(
        "UPDATE track SET is_available = 0 \
         WHERE relative_path = 'Björk/Post/03 Possibly Maybe.flac'",
    )
    .execute(state.db.pool())
    .await
    .unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "tree-owner", &password).await;
    let listener_token = login_token(&router, "tree-listener", &password).await;
    let stranger_token = login_token(&router, "tree-stranger", &password).await;
    let tree = |token: &str, query: &str| {
        let router = router.clone();
        let token = token.to_owned();
        let uri = format!("/api/v2/libraries/{library_id}/tree{query}");
        async move {
            router
                .oneshot(
                    Request::get(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    let path = |path: &str| {
        format!(
            "?path={}",
            percent_encoding::utf8_percent_encode(path, percent_encoding::NON_ALPHANUMERIC)
        )
    };
    let folders = |listing: &serde_json::Value| -> Vec<(String, String, i64, i64)> {
        listing["folders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|folder| {
                (
                    folder["name"].as_str().unwrap().to_owned(),
                    folder["path"].as_str().unwrap().to_owned(),
                    folder["track_count"].as_i64().unwrap(),
                    folder["nested_track_count"].as_i64().unwrap(),
                )
            })
            .collect()
    };
    let titles = |listing: &serde_json::Value| -> Vec<String> {
        listing["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|track| track["song"]["title"].as_str().unwrap().to_owned())
            .collect()
    };
    let folder = |name: &str, path: &str, direct: i64, nested: i64| {
        (name.to_owned(), path.to_owned(), direct, nested)
    };

    // The root: the received folder under its real path, the artist folder
    // counting what is beneath it, the neighbour apart, and no `Gone`.
    let top = json_body(tree(&owner_token, "").await).await;
    assert_eq!(top["path"], "");
    assert_eq!(
        folders(&top),
        [
            folder(".waveflow-managed", ".waveflow-managed", 3, 3),
            folder("Björk", "Björk", 0, 4),
            folder("Björk-B", "Björk-B", 1, 1),
        ]
    );
    assert_eq!(
        (top["folder_total"].as_i64(), top["track_total"].as_i64()),
        (Some(3), Some(1))
    );
    assert_eq!(titles(&top), ["Loose"]);
    // A listener reads the same tree; the answer is the owner's, to the byte.
    let heard = json_body(tree(&listener_token, "").await).await;
    assert_eq!(heard, top);

    let artist = json_body(tree(&owner_token, &path("Björk")).await).await;
    assert_eq!(
        folders(&artist),
        [
            folder("Homogenic", "Björk/Homogenic", 1, 1),
            folder("Post", "Björk/Post", 2, 3),
        ]
    );
    assert_eq!(artist["track_total"], 0);

    // A folder's tracks sort by file name, and carry it; the song itself
    // keeps the shape every other listing gives it, path left out.
    let album = json_body(tree(&owner_token, &path("Björk/Post")).await).await;
    assert_eq!(folders(&album), [folder("Bonus", "Björk/Post/Bonus", 1, 1)]);
    assert_eq!(titles(&album), ["Army Of Me", "Hyperballad"]);
    assert_eq!(album["tracks"][0]["name"], "01 Army Of Me.flac");
    assert!(omits(&album["tracks"][0]["song"], "relative_path"));

    // One page runs over the folders, then the tracks.
    let pages = [
        ("&limit=1", vec!["Bonus"], vec![]),
        ("&offset=1&limit=1", vec![], vec!["Army Of Me"]),
        ("&offset=2&limit=5", vec![], vec!["Hyperballad"]),
    ];
    for (window, expected_folders, expected_tracks) in pages {
        let page =
            json_body(tree(&owner_token, &format!("{}{window}", path("Björk/Post"))).await).await;
        let names: Vec<String> = folders(&page).into_iter().map(|folder| folder.0).collect();
        assert_eq!(names, expected_folders, "{window}");
        assert_eq!(titles(&page), expected_tracks, "{window}");
        assert_eq!(
            (page["folder_total"].as_i64(), page["track_total"].as_i64()),
            (Some(1), Some(2)),
            "{window}"
        );
    }

    // Received: hashes for names, so artist then title, a missing artist last.
    let received = json_body(tree(&owner_token, &path(".waveflow-managed")).await).await;
    assert!(folders(&received).is_empty());
    assert_eq!(titles(&received), ["Waterloo", "Zebra", "Nameless"]);

    // Nothing through it for a stranger, and nothing for a folder no
    // available track runs through.
    for (token, query) in [
        (&stranger_token, String::new()),
        (&stranger_token, path("Björk")),
        (&owner_token, path("Gone")),
        (&owner_token, path("Nowhere")),
        (&owner_token, path("Björk/Post/01 Army Of Me.flac")),
    ] {
        assert_eq!(
            tree(token, &query).await.status(),
            StatusCode::NOT_FOUND,
            "{query}"
        );
    }
    // One spelling per folder, and a page that means something.
    for query in [
        path("Björk/"),
        path("/Björk"),
        path("Björk//Post"),
        path("../Björk"),
        "?limit=0".to_owned(),
        "?limit=501".to_owned(),
        "?offset=-1".to_owned(),
    ] {
        assert_eq!(
            tree(&owner_token, &query).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{query}"
        );
    }
}

/// Filed folders (RFC-012 decisions 2 and 3), through the router: names,
/// depth, cycles and the folder cap checked before anything is written,
/// placements all or nothing, a delete that unfiles, and a listener who reads
/// every bit of it and writes none. No file exists here at all: filing never
/// needs one. Passwords are drawn at run time, for CodeQL.
#[tokio::test]
async fn filed_folders_sort_a_library_in_the_database_only() {
    let (_temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let mut accounts = Vec::new();
    for name in ["filer-owner", "filer-listener", "filer-stranger"] {
        accounts.push(
            state
                .db
                .create_account(name, &hash, AccountRole::User, now_ms())
                .await
                .unwrap(),
        );
    }
    let (owner, listener) = (accounts[0], accounts[1]);
    let root = std::fs::canonicalize(&config.data_dir).unwrap();
    let mut libraries = Vec::new();
    let mut tracks = Vec::new();
    for (offset, name) in [(0, "Filed"), (10, "Elsewhere")] {
        let library_id = state
            .db
            .create_library(
                owner,
                name,
                &root.join(name),
                LibraryVisibility::Private,
                now_ms(),
            )
            .await
            .unwrap();
        let scan_id = state
            .db
            .create_scan_job(library_id, Some(owner), "manual")
            .await
            .unwrap();
        state.db.start_scan_job(scan_id, 5, false).await.unwrap();
        for index in 0..5 {
            let title = format!("{name} {index}");
            let mut input = browse_input(
                30_000 + offset + index,
                &title,
                "Album",
                "Artist",
                Some(index as i64 + 1),
                Some(1),
            );
            input.relative_path = format!("{name}/{index}.flac");
            state
                .db
                .apply_catalog_track(library_id, scan_id, &input, None, false)
                .await
                .unwrap();
        }
        state.db.finish_scan_job(scan_id, 0).await.unwrap();
        let ids: Vec<uuid::Uuid> = state
            .db
            .list_tracks_for_user(owner, library_id)
            .await
            .unwrap()
            .into_iter()
            .map(|track| track.id)
            .collect();
        libraries.push(library_id);
        tracks.push(ids);
    }
    let (library, elsewhere) = (libraries[0], libraries[1]);
    state
        .db
        .add_library_member(
            owner,
            library,
            listener,
            waveflow_server::storage::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    // Sorted by title, so `t[0]` is "Filed 0".
    let t = &tracks[0];
    let foreign_track = tracks[1][0];

    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "filer-owner", &password).await;
    let listener_token = login_token(&router, "filer-listener", &password).await;
    let stranger_token = login_token(&router, "filer-stranger", &password).await;
    let send = |method: Method, uri: String, token: &str, body: Option<serde_json::Value>| {
        let router = router.clone();
        let token = token.to_owned();
        async move {
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"));
            let request = match body {
                Some(body) => request
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string())),
                None => request.body(Body::empty()),
            };
            router.oneshot(request.unwrap()).await.unwrap()
        }
    };
    let folders_uri = format!("/api/v2/libraries/{library}/folders");
    let placements_uri = format!("/api/v2/libraries/{library}/placements");
    let create = |token: &str, name: &str, parent: Option<&str>| {
        send(
            Method::POST,
            folders_uri.clone(),
            token,
            Some(serde_json::json!({ "name": name, "parent_id": parent })),
        )
    };
    let tree = || async {
        json_body(send(Method::GET, folders_uri.clone(), &owner_token, None).await).await
    };
    let count_of = |tree: &serde_json::Value, name: &str| -> Option<i64> {
        tree["folders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|folder| folder["name"] == name)
            .map(|folder| folder["track_count"].as_i64().unwrap())
    };
    let titles = |songs: serde_json::Value| -> Vec<String> {
        songs
            .as_array()
            .unwrap()
            .iter()
            .map(|song| song["title"].as_str().unwrap().to_owned())
            .collect()
    };

    // A fresh library is entirely unfiled.
    let fresh = tree().await;
    assert_eq!(fresh["folders"], serde_json::json!([]));
    assert_eq!(fresh["unfiled_track_count"], 5);

    // Names: trimmed, folded for siblings beyond ASCII, bounded, `/` allowed.
    let rock = create(&owner_token, "  Rock ", None).await;
    assert_eq!(rock.status(), StatusCode::CREATED);
    let rock = json_body(rock).await;
    assert_eq!(rock["name"], "Rock");
    assert_eq!(rock["parent_id"], serde_json::Value::Null);
    let rock_id = rock["id"].as_str().unwrap().to_owned();
    assert_eq!(
        create(&owner_token, "ROCK", None).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        create(&owner_token, "Été", None).await.status(),
        StatusCode::CREATED
    );
    assert_eq!(
        create(&owner_token, "ÉTÉ", None).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        create(&owner_token, "A/B", None).await.status(),
        StatusCode::CREATED
    );
    for refused in ["   ".to_owned(), "x".repeat(121), "line\nbreak".to_owned()] {
        assert_eq!(
            create(&owner_token, &refused, None).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{refused:?}"
        );
    }
    let indie = json_body(create(&owner_token, "Indie", Some(&rock_id)).await).await;
    let indie_id = indie["id"].as_str().unwrap().to_owned();
    assert_eq!(indie["parent_id"], rock_id.as_str());
    // A sibling elsewhere in the tree is no clash: "Indie" again, at the root.
    assert_eq!(
        create(&owner_token, "Indie", None).await.status(),
        StatusCode::CREATED
    );

    // A listener reads and writes nothing; a stranger does not even read.
    assert_eq!(
        send(Method::GET, folders_uri.clone(), &listener_token, None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(Method::GET, folders_uri.clone(), &stranger_token, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        create(&listener_token, "Mine", None).await.status(),
        StatusCode::NOT_FOUND
    );

    // Placements: filed, then moved, a track in one folder at a time.
    let place = |token: &str, ids: Vec<String>, folder: Option<&str>| {
        send(
            Method::PUT,
            placements_uri.clone(),
            token,
            Some(serde_json::json!({ "track_ids": ids, "folder_id": folder })),
        )
    };
    let placed = place(
        &owner_token,
        vec![t[0].to_string(), t[1].to_string()],
        Some(&indie_id),
    )
    .await;
    assert_eq!(placed.status(), StatusCode::OK);
    assert_eq!(json_body(placed).await["placed"], 2);
    let moved = place(&owner_token, vec![t[0].to_string()], Some(&rock_id)).await;
    assert_eq!(moved.status(), StatusCode::OK);
    let sorted = tree().await;
    assert_eq!(
        (count_of(&sorted, "Rock"), count_of(&sorted, "A/B")),
        (Some(1), Some(0))
    );
    assert_eq!(sorted["unfiled_track_count"], 3);
    let in_indie = send(
        Method::GET,
        format!("{folders_uri}/{indie_id}/tracks"),
        &listener_token,
        None,
    )
    .await;
    assert_eq!(titles(json_body(in_indie).await), ["Filed 1"]);
    let unfiled = send(
        Method::GET,
        format!("{folders_uri}/unfiled/tracks"),
        &listener_token,
        None,
    )
    .await;
    assert_eq!(
        titles(json_body(unfiled).await),
        ["Filed 2", "Filed 3", "Filed 4"]
    );
    assert_eq!(
        send(
            Method::GET,
            format!("{folders_uri}/nonsense/tracks"),
            &owner_token,
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    // All or nothing: one track from another library, and nothing is filed.
    let mixed = place(
        &owner_token,
        vec![t[2].to_string(), foreign_track.to_string()],
        Some(&rock_id),
    )
    .await;
    assert_eq!(mixed.status(), StatusCode::NOT_FOUND);
    // A folder of another library, the listener, and one track too many.
    let foreign_folder = json_body(
        send(
            Method::POST,
            format!("/api/v2/libraries/{elsewhere}/folders"),
            &owner_token,
            Some(serde_json::json!({ "name": "Theirs" })),
        )
        .await,
    )
    .await;
    let foreign_folder = foreign_folder["id"].as_str().unwrap().to_owned();
    assert_eq!(
        place(&owner_token, vec![t[2].to_string()], Some(&foreign_folder))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            Method::GET,
            format!("{folders_uri}/{foreign_folder}/tracks"),
            &owner_token,
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        place(&listener_token, vec![t[2].to_string()], Some(&rock_id))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let many = |count: usize| {
        (0..count)
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect::<Vec<_>>()
    };
    // Five hundred ids fit the body, and name nothing here; one more is refused
    // as a number before anything is looked up.
    assert_eq!(
        place(&owner_token, many(500), Some(&rock_id))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        place(&owner_token, many(501), Some(&rock_id))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(tree().await["unfiled_track_count"], 3);
    // Unfiling is a placement in no folder.
    assert_eq!(
        place(&owner_token, vec![t[1].to_string()], None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(tree().await["unfiled_track_count"], 4);
    place(&owner_token, vec![t[1].to_string()], Some(&indie_id)).await;

    // Moves: never under itself, and the whole sub-tree within the cap.
    let patch = |token: &str, id: &str, body: serde_json::Value| {
        send(
            Method::PATCH,
            format!("/api/v2/folders/{id}"),
            token,
            Some(body),
        )
    };
    for parent in [&rock_id, &indie_id] {
        assert_eq!(
            patch(
                &owner_token,
                &rock_id,
                serde_json::json!({ "parent_id": parent })
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    // Nor under a folder of another library, though the caller owns both.
    assert_eq!(
        patch(
            &owner_token,
            &rock_id,
            serde_json::json!({ "parent_id": foreign_folder })
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let mut chain = Vec::new();
    let mut parent: Option<String> = None;
    for depth in 1..=16 {
        let folder =
            json_body(create(&owner_token, &format!("Level {depth}"), parent.as_deref()).await)
                .await;
        let id = folder["id"].as_str().unwrap().to_owned();
        chain.push(id.clone());
        parent = Some(id);
    }
    assert_eq!(
        create(&owner_token, "Seventeen", Some(&chain[15]))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Rock carries Indie: two high. Under depth 15 it would reach 17.
    assert_eq!(
        patch(
            &owner_token,
            &rock_id,
            serde_json::json!({ "parent_id": chain[14] })
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let buried = patch(
        &owner_token,
        &rock_id,
        serde_json::json!({ "parent_id": chain[13] }),
    )
    .await;
    assert_eq!(buried.status(), StatusCode::OK);
    assert_eq!(json_body(buried).await["parent_id"], chain[13].as_str());
    // A rename alone leaves the folder where it is; `null` brings it home.
    let renamed = json_body(
        patch(
            &owner_token,
            &rock_id,
            serde_json::json!({ "name": "Rock & Roll" }),
        )
        .await,
    )
    .await;
    assert_eq!(
        (renamed["name"].as_str(), renamed["parent_id"].as_str()),
        (Some("Rock & Roll"), Some(chain[13].as_str()))
    );
    let home = json_body(
        patch(
            &owner_token,
            &rock_id,
            serde_json::json!({ "parent_id": null }),
        )
        .await,
    )
    .await;
    assert_eq!(home["parent_id"], serde_json::Value::Null);
    assert_eq!(home["track_count"], 1);
    // Back among the root's children, its name must not clash there either.
    assert_eq!(
        patch(&owner_token, &rock_id, serde_json::json!({ "name": "a/b" }))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        patch(
            &listener_token,
            &rock_id,
            serde_json::json!({ "name": "Mine" })
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    // A delete takes the sub-folders with it and unfiles what they held.
    assert_eq!(
        send(
            Method::DELETE,
            format!("/api/v2/folders/{rock_id}"),
            &listener_token,
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let removed = send(
        Method::DELETE,
        format!("/api/v2/folders/{rock_id}"),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(removed.status(), StatusCode::OK);
    assert_eq!(json_body(removed).await["unfiled"], 2);
    let after = tree().await;
    assert_eq!(after["unfiled_track_count"], 5);
    assert!(
        count_of(&after, "Indie").is_some(),
        "the root's own Indie stays"
    );
    assert_eq!(
        after["folders"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|folder| folder["name"] == "Indie")
            .count(),
        1
    );
    assert_eq!(
        send(
            Method::DELETE,
            format!("/api/v2/folders/{rock_id}"),
            &owner_token,
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    // The folder cap, filled in SQL to one short and then reached.
    let held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM library_folder WHERE library_id = ?")
        .bind(library.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO library_folder \
           (id, library_id, parent_id, name, name_key, created_by, created_at, updated_at) \
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?) \
         SELECT lower(hex(randomblob(16))), ?, NULL, 'Bulk ' || i, 'bulk ' || i, ?, 0, 0 FROM n",
    )
    .bind(4_999 - held)
    .bind(library.to_string())
    .bind(owner.to_string())
    .execute(state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        create(&owner_token, "The last", None).await.status(),
        StatusCode::CREATED
    );
    assert_eq!(
        create(&owner_token, "One too many", None).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Another library keeps its own count.
    assert_eq!(
        send(
            Method::POST,
            format!("/api/v2/libraries/{elsewhere}/folders"),
            &owner_token,
            Some(serde_json::json!({ "name": "Still room" })),
        )
        .await
        .status(),
        StatusCode::CREATED
    );
}

/// Names with numbers in them sort by value, in SQL, so a page cut there cuts
/// the order a person reads: `2 …` before `10 …` before `100 …`, `Disc 2`
/// before `Disc 10`, in the disk tree and among filed folders alike.
#[tokio::test]
async fn numbered_names_sort_by_value_in_both_trees() {
    let (_temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("natural-owner", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let root = std::fs::canonicalize(&config.data_dir).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Natural",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    let paths = [
        "100 Snooze.m4a",
        "2 Paradise.m4a",
        "10 Ballad.m4a",
        "1 Morning.mp3",
        "Disc 10/a.flac",
        "Disc 2/a.flac",
        "disc 1/a.flac",
    ];
    state
        .db
        .start_scan_job(scan_id, paths.len() as i64, false)
        .await
        .unwrap();
    for (index, path) in paths.iter().enumerate() {
        let mut input = browse_input(40_000 + index, path, "Album", "Artist", None, None);
        input.relative_path = (*path).to_owned();
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "natural-owner", &password).await;
    let get = |uri: String| {
        let router = router.clone();
        let token = token.clone();
        async move {
            json_body(
                router
                    .oneshot(
                        Request::get(uri)
                            .header("authorization", format!("Bearer {token}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap(),
            )
            .await
        }
    };
    let names = |listing: &serde_json::Value, key: &str| -> Vec<String> {
        listing[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap().to_owned())
            .collect()
    };

    let top = get(format!("/api/v2/libraries/{library_id}/tree")).await;
    assert_eq!(names(&top, "folders"), ["disc 1", "Disc 2", "Disc 10"]);
    assert_eq!(
        names(&top, "tracks"),
        [
            "1 Morning.mp3",
            "2 Paradise.m4a",
            "10 Ballad.m4a",
            "100 Snooze.m4a"
        ]
    );
    // The page is cut in that order: past the three folders, the second and
    // third tracks are 2 and 10, not 10 and 100.
    let page = get(format!(
        "/api/v2/libraries/{library_id}/tree?offset=4&limit=2"
    ))
    .await;
    assert_eq!(names(&page, "tracks"), ["2 Paradise.m4a", "10 Ballad.m4a"]);

    let folders_uri = format!("/api/v2/libraries/{library_id}/folders");
    for name in ["Vol 10", "Vol 2", "vol 1"] {
        let created = router
            .clone()
            .oneshot(
                Request::post(folders_uri.clone())
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({ "name": name }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
    }
    let filed = get(folders_uri).await;
    assert_eq!(names(&filed, "folders"), ["vol 1", "Vol 2", "Vol 10"]);
}
