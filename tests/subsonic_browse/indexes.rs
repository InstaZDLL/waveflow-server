//! The root of browsing by folders: `getIndexes` and its root files.

use super::*;

/// A track that belongs to no album is a root file of `getIndexes`.
///
/// Found with DSub browsing by folders ("Browse By Tags" off): it enters at
/// `getIndexes` and walks down through `getMusicDirectory`, but never asks
/// for the music folder itself — the one level that listed album-less
/// tracks. Such a track sat in no artist's directory and at no root, so
/// search was the only way to reach it.
///
/// Asserted on both surfaces, because the shape is where it breaks: one loose
/// track is the common case, and a `child` the JSON array table did not know
/// would have gone out as an object. The folder filter and the account
/// boundary are asserted alongside, since the listing reads tracks the
/// artist index never did.
#[tokio::test]
async fn album_less_tracks_are_root_children_of_get_indexes() {
    let (_temp, config, state) = test_app().await;
    let now = now_ms();
    // Inserted rather than created: these accounts authenticate by API key
    // only, so no password is ever hashed.
    let account = |username: &'static str| {
        let state = &state;
        async move {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
                 VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
            )
            .bind(id.to_string())
            .bind(username)
            .bind(now)
            .bind(now)
            .execute(state.db.pool())
            .await
            .unwrap();
            let api_key = format!("wfsk_{}", Uuid::new_v4().simple());
            let secret = Uuid::new_v4().to_string();
            state
                .db
                .set_subsonic_credential(
                    id,
                    id,
                    &state.secret_box.encrypt(secret.as_bytes()).unwrap(),
                    &security::token_hash(&api_key),
                    now,
                )
                .await
                .unwrap();
            (id, api_key)
        }
    };
    let (owner, owner_key) = account("indexes-owner").await;
    let (outsider, outsider_key) = account("indexes-outsider").await;

    // Each track is (index, path, title, album, artist); `None` is a file
    // tagged with no album, which the catalogue keeps album-less.
    type Track = (
        usize,
        &'static str,
        &'static str,
        Option<&'static str>,
        &'static str,
    );
    let seed = |owner: Uuid, name: &'static str, tracks: Vec<Track>| {
        let state = state.clone();
        let root = config.data_dir.join(name);
        async move {
            std::fs::create_dir_all(&root).unwrap();
            let library = state
                .db
                .create_library(
                    owner,
                    name,
                    &std::fs::canonicalize(&root).unwrap(),
                    LibraryVisibility::Private,
                    now,
                )
                .await
                .unwrap();
            let scan = state
                .db
                .create_scan_job(library, Some(owner), "manual")
                .await
                .unwrap();
            state
                .db
                .start_scan_job(scan, tracks.len() as i64, false)
                .await
                .unwrap();
            for (index, path, title, album, artist) in tracks {
                let mut input =
                    browse_input(index, title, album.unwrap_or(""), artist, Some(1), Some(1));
                input.album = album.map(str::to_owned);
                input.album_artist = album.map(|_| artist.to_owned());
                input.relative_path = path.to_owned();
                state
                    .db
                    .apply_catalog_track(library, scan, &input, None, false)
                    .await
                    .unwrap();
            }
            state.db.consolidate_sort_names(library).await.unwrap();
            state
                .db
                .consolidate_catalog_derivations(library)
                .await
                .unwrap();
            state.db.finish_scan_job(scan, 0).await.unwrap();
            library
        }
    };
    // The campaign's catalogue: Ivy Trench has an album, and a sketch filed
    // under `Loose/` that belongs to none.
    let home = seed(
        owner,
        "indexes-home",
        vec![
            (
                7_100,
                "Ivy Trench/Signal/01.flac",
                "Opening",
                Some("Signal"),
                "Ivy Trench",
            ),
            (
                7_101,
                "Loose/untitled-sketch.flac",
                "Untitled Sketch",
                None,
                "Ivy Trench",
            ),
        ],
    )
    .await;
    // A second library of the owner's, holding albums only.
    let other = seed(
        owner,
        "indexes-other",
        vec![(
            7_200,
            "Other/Record/01.flac",
            "Elsewhere",
            Some("Record"),
            "Other Artist",
        )],
    )
    .await;
    // And the outsider's own, with a loose track of its own.
    let foreign = seed(
        outsider,
        "indexes-foreign",
        vec![(
            7_300,
            "Loose/foreign-sketch.flac",
            "Foreign Sketch",
            None,
            "Foreign Artist",
        )],
    )
    .await;
    let router = waveflow_server::app(&config, state.clone());

    let child_titles = |indexes: &serde_json::Value| -> Vec<String> {
        indexes["child"]
            .as_array()
            .unwrap_or_else(|| panic!("indexes.child is an array: {indexes}"))
            .iter()
            .map(|child| child["title"].as_str().unwrap_or_default().to_owned())
            .collect()
    };

    // --- JSON, every folder -----------------------------------------------
    let response = subsonic_json(&router, "getIndexes", &owner_key, "").await;
    let indexes = &response["subsonic-response"]["indexes"];
    // An array with its single entry: the one shape a strictly typed client
    // decodes, and the one an unknown container would have lost.
    assert_eq!(child_titles(indexes), vec!["Untitled Sketch"]);
    let sketch = &indexes["child"][0];
    assert_eq!(sketch["isDir"], serde_json::json!(false));
    assert_eq!(sketch["artist"], "Ivy Trench");
    // Its parent is the folder it already named, the one `getMusicDirectory`
    // lists it under.
    assert_eq!(sketch["parent"].as_str(), Some(home.to_string()).as_deref());
    // The artists are still the indexes, and the album's own track is not
    // promoted to the root.
    let artists = indexes["index"]
        .as_array()
        .expect("the indexes are an array")
        .iter()
        .flat_map(|index| index["artist"].as_array().expect("an index holds artists"))
        .map(|artist| artist["name"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(artists, vec!["Ivy Trench", "Other Artist"]);

    // --- XML, every folder ------------------------------------------------
    let xml = subsonic_xml(&router, "getIndexes", &owner_key).await;
    assert_eq!(
        xml.matches("<child ").count(),
        1,
        "one root file in XML: {xml}"
    );
    assert!(
        xml.contains(r#"title="Untitled Sketch""#),
        "the root file is the album-less track: {xml}"
    );
    assert!(
        xml.find("<index ") < xml.find("<child "),
        "the schema orders the indexes before the root files: {xml}"
    );

    // --- musicFolderId ----------------------------------------------------
    let only_home = subsonic_json(
        &router,
        "getIndexes",
        &owner_key,
        &format!("&musicFolderId={home}"),
    )
    .await;
    assert_eq!(
        child_titles(&only_home["subsonic-response"]["indexes"]),
        vec!["Untitled Sketch"]
    );
    // A folder with no album-less track answers as it always did: indexes,
    // and no `child` key at all rather than an empty one.
    let only_other = subsonic_json(
        &router,
        "getIndexes",
        &owner_key,
        &format!("&musicFolderId={other}"),
    )
    .await;
    let only_other = &only_other["subsonic-response"]["indexes"];
    assert!(
        omits(only_other, "child"),
        "another folder's loose track leaked in: {only_other}"
    );
    assert_eq!(only_other["index"][0]["artist"][0]["name"], "Other Artist");

    // --- getArtists is the ID3 model, which has no root files -------------
    let id3 = subsonic_json(&router, "getArtists", &owner_key, "").await;
    assert!(
        omits(&id3["subsonic-response"]["artists"], "child"),
        "getArtists has no child: {id3}"
    );

    // --- the account boundary ---------------------------------------------
    let theirs = subsonic_json(&router, "getIndexes", &outsider_key, "").await;
    assert_eq!(
        child_titles(&theirs["subsonic-response"]["indexes"]),
        vec!["Foreign Sketch"],
        "an account sees its own loose tracks and nobody else's"
    );
    // Naming a folder it does not belong to reaches nothing in it.
    let trespass = subsonic_json(
        &router,
        "getIndexes",
        &outsider_key,
        &format!("&musicFolderId={home}"),
    )
    .await;
    assert!(
        omits(&trespass["subsonic-response"]["indexes"], "child"),
        "a foreign folder's loose track reached another account: {trespass}"
    );
    // And the owner, naming the outsider's folder, is held to the same rule.
    let reverse = subsonic_json(
        &router,
        "getIndexes",
        &owner_key,
        &format!("&musicFolderId={foreign}"),
    )
    .await;
    assert!(
        omits(&reverse["subsonic-response"]["indexes"], "child"),
        "the outsider's loose track reached the owner: {reverse}"
    );
}
