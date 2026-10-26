//! Playlists through the façade: duplicates, positions, public playlists and
//! their owners.

use super::*;

fn entry_ids(playlist: &serde_json::Value) -> Vec<String> {
    playlist["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| entry["id"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// `createPlaylist` and `updatePlaylist` take a track more than once, and
/// `songIndexToRemove` names a position in the list the client was shown
/// (RFC-020 decision 1).
///
/// The second half is what makes the first usable. With duplicates, a
/// position is the only way to say which copy goes — and the positions a
/// client sees skip every track it can no longer read, while the stored list
/// still holds them. Applying the index to the stored list removed the wrong
/// entry, and the one the listener pointed at stayed.
#[tokio::test]
async fn subsonic_playlists_take_duplicates_and_remove_by_the_position_shown() {
    let (_temp, config, state) = test_app().await;
    let (owner, api_key) = subsonic_account(&state, "dup-subsonic").await;
    let titles = ["Gone Soon", "Again", "Beside"];
    let (library, ids) = seed_titles(&state, &config, owner, "dup-sub", 4_000, &titles).await;
    let (gone, a, b) = (ids[0], ids[1], ids[2]);
    let router = waveflow_server::app(&config, state.clone());

    let created = subsonic_json(
        &router,
        "createPlaylist",
        &api_key,
        &format!("&name=Twice&songId={a}&songId={a}"),
    )
    .await;
    let created = &created["subsonic-response"]["playlist"];
    assert_eq!(created["songCount"], 2, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();

    let updated = subsonic_json(
        &router,
        "updatePlaylist",
        &api_key,
        &format!("&playlistId={id}&songIdToAdd={b}&songIdToAdd={a}"),
    )
    .await;
    assert_eq!(updated["subsonic-response"]["status"], "ok", "{updated}");
    let read = subsonic_json(&router, "getPlaylist", &api_key, &format!("&id={id}")).await;
    assert_eq!(
        entry_ids(&read["subsonic-response"]["playlist"]),
        vec![a.to_string(), a.to_string(), b.to_string(), a.to_string()]
    );

    subsonic_json(
        &router,
        "updatePlaylist",
        &api_key,
        &format!("&playlistId={id}&songIndexToRemove=1"),
    )
    .await;
    let read = subsonic_json(&router, "getPlaylist", &api_key, &format!("&id={id}")).await;
    assert_eq!(
        entry_ids(&read["subsonic-response"]["playlist"]),
        vec![a.to_string(), b.to_string(), a.to_string()]
    );

    // A playlist whose first track then goes missing from disk: the client
    // is shown two entries and removes the first of those.
    let hidden_first = subsonic_json(
        &router,
        "createPlaylist",
        &api_key,
        &format!("&name=Hidden%20first&songId={gone}&songId={a}&songId={b}"),
    )
    .await;
    let hidden_first = hidden_first["subsonic-response"]["playlist"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let rescan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(rescan, 2, false).await.unwrap();
    // The rescan sees the other two files and not the first.
    for track in [a, b] {
        state.db.mark_track_seen(track, rescan).await.unwrap();
    }
    assert_eq!(
        state
            .db
            .mark_unseen_unavailable(library, rescan)
            .await
            .unwrap(),
        1
    );
    state.db.finish_scan_job(rescan, 0).await.unwrap();
    let shown = subsonic_json(
        &router,
        "getPlaylist",
        &api_key,
        &format!("&id={hidden_first}"),
    )
    .await;
    assert_eq!(
        entry_ids(&shown["subsonic-response"]["playlist"]),
        vec![a.to_string(), b.to_string()]
    );
    subsonic_json(
        &router,
        "updatePlaylist",
        &api_key,
        &format!("&playlistId={hidden_first}&songIndexToRemove=0"),
    )
    .await;
    let shown = subsonic_json(
        &router,
        "getPlaylist",
        &api_key,
        &format!("&id={hidden_first}"),
    )
    .await;
    assert_eq!(
        entry_ids(&shown["subsonic-response"]["playlist"]),
        vec![b.to_string()],
        "the entry the client pointed at is the one that goes"
    );
    // And the track it could not see is still held, for when it comes back.
    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT track_id FROM playlist_track WHERE playlist_id=? ORDER BY position",
    )
    .bind(&hidden_first)
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(stored, vec![gone.to_string(), b.to_string()]);
}

/// `getPlaylists` and `getPlaylist` answer the public playlists of other
/// accounts, with `owner` and `public` saying whose they are, and every write
/// to one answers error 70 for anyone but its owner (RFC-020 decision 2).
#[tokio::test]
async fn subsonic_lists_public_playlists_of_other_accounts_under_their_owner() {
    let (_temp, config, state) = test_app().await;
    let (owner, owner_key) = subsonic_account(&state, "pub-owner").await;
    let (viewer, viewer_key) = subsonic_account(&state, "pub-viewer").await;
    let (shared, open) = seed_titles(
        &state,
        &config,
        owner,
        "pub-shared",
        4_100,
        &["Heard One", "Heard Two"],
    )
    .await;
    let (_private, kept) =
        seed_titles(&state, &config, owner, "pub-private", 4_200, &["Unheard"]).await;
    state
        .db
        .add_library_member(owner, shared, viewer, LibraryRole::Listener, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let created = subsonic_json(
        &router,
        "createPlaylist",
        &owner_key,
        &format!(
            "&name=Open&songId={}&songId={}&songId={}",
            open[0], kept[0], open[1]
        ),
    )
    .await;
    let id = created["subsonic-response"]["playlist"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let listed = subsonic_json(&router, "getPlaylists", &viewer_key, "").await;
    assert!(
        listed["subsonic-response"]["playlists"]["playlist"]
            .as_array()
            .is_none_or(|playlists| playlists.is_empty()),
        "a private playlist is not listed to another account: {listed}"
    );

    subsonic_json(
        &router,
        "updatePlaylist",
        &owner_key,
        &format!("&playlistId={id}&public=true"),
    )
    .await;

    let listed = subsonic_json(&router, "getPlaylists", &viewer_key, "").await;
    let listed = listed["subsonic-response"]["playlists"]["playlist"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], id.as_str());
    assert_eq!(listed[0]["owner"], "pub-owner");
    assert_eq!(listed[0]["public"], true);
    // Counted over what the viewer can play, and nothing else.
    assert_eq!(listed[0]["songCount"], 2);
    assert_eq!(listed[0]["duration"], 240);

    let read = subsonic_json(&router, "getPlaylist", &viewer_key, &format!("&id={id}")).await;
    let read = &read["subsonic-response"]["playlist"];
    assert_eq!(read["owner"], "pub-owner");
    assert_eq!(
        entry_ids(read),
        vec![open[0].to_string(), open[1].to_string()]
    );
    assert!(!read.to_string().contains(&kept[0].to_string()));

    // The owner's own list says the playlist is theirs.
    let own = subsonic_json(&router, "getPlaylists", &owner_key, "").await;
    assert_eq!(
        own["subsonic-response"]["playlists"]["playlist"][0]["owner"],
        "pub-owner"
    );

    for (method, extra) in [
        ("updatePlaylist", format!("&playlistId={id}&name=Taken")),
        (
            "updatePlaylist",
            format!("&playlistId={id}&songIdToAdd={}", open[0]),
        ),
        (
            "createPlaylist",
            format!("&playlistId={id}&songId={}", open[0]),
        ),
        ("deletePlaylist", format!("&id={id}")),
    ] {
        let refused = subsonic_json(&router, method, &viewer_key, &extra).await;
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 70,
            "{method} by a non-owner: {refused}"
        );
    }
    let intact = subsonic_json(&router, "getPlaylist", &owner_key, &format!("&id={id}")).await;
    let intact = &intact["subsonic-response"]["playlist"];
    assert_eq!(intact["name"], "Open");
    assert_eq!(intact["songCount"], 3);

    subsonic_json(
        &router,
        "updatePlaylist",
        &owner_key,
        &format!("&playlistId={id}&public=false"),
    )
    .await;
    let withdrawn = subsonic_json(&router, "getPlaylist", &viewer_key, &format!("&id={id}")).await;
    assert_eq!(withdrawn["subsonic-response"]["error"]["code"], 70);
}

// A playlist as long as the domain allows fits in one form POST, and the body
/// is still bounded (RFC-022 decision 7).
///
/// `formPost` exists for requests too long for a URL, and the longest one a
/// client sends is a playlist edit naming every track. The old fixed 64 KiB
/// refused `updatePlaylist` short of 1,338 identifiers — `songIdToAdd=<uuid>&` is
/// forty-nine bytes — against a ceiling of ten thousand. The edit here names
/// exactly that ceiling, so it fails under the old bound and passes under the
/// new one; the second half sends one byte more than the bound and must still
/// be refused, so the bound cannot have quietly gone away.
#[tokio::test]
async fn a_playlist_as_long_as_the_domain_allows_fits_in_one_form_post() {
    let (_temp, config, state) = test_app().await;
    // Inserted rather than created, every credential drawn at run time: see
    // the radio test above for why.
    let owner = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, 'form-owner', 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(owner.to_string())
    .bind(now_ms())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let api_key = format!("wfsk_{}", Uuid::new_v4());
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state
                .secret_box
                .encrypt(Uuid::new_v4().to_string().as_bytes())
                .unwrap(),
            &security::token_hash(&api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let root = config.data_dir.join("form-library");
    std::fs::create_dir_all(&root).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "form-library",
            &std::fs::canonicalize(&root).unwrap(),
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
    let input = browse_input(0, "Repeat", "Form Album", "Form Artist", Some(1), Some(1));
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
    let song = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .songs[0]
        .id;
    // A playlist holds a track once (`playlist_track` is keyed on the pair), so
    // reaching the ceiling takes that many distinct tracks. Ten thousand
    // through the catalogue writer would take minutes; copying the one row in
    // SQL gives the same projection, since everything a playlist reads of a
    // track is on that row. Each copy gets its own id and path, the two
    // columns the table holds unique.
    let ceiling = waveflow_server::services::MAX_PLAYLIST_TRACKS;
    // One connection throughout: a temporary table belongs to the connection
    // that made it.
    let mut connection = state.db.pool().acquire().await.unwrap();
    sqlx::query(
        "CREATE TEMP TABLE copies AS \
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?) \
         SELECT t.*, n.i AS copy_number FROM track t, n WHERE t.id = ?",
    )
    .bind(ceiling as i64 - 1)
    .bind(song.to_string())
    .execute(&mut *connection)
    .await
    .unwrap();
    for statement in [
        "UPDATE copies SET id = printf('%08x-0000-4000-8000-%012x', copy_number, copy_number), \
         relative_path = 'copy-' || copy_number || '.flac'",
        "ALTER TABLE copies DROP COLUMN copy_number",
        "INSERT INTO track SELECT * FROM copies",
        "DROP TABLE copies",
    ] {
        sqlx::query(statement)
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    drop(connection);
    let tracks = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .songs
        .into_iter()
        .map(|song| song.id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(tracks.len(), ceiling);
    let router = waveflow_server::app(&config, state.clone());
    let created = subsonic_json(&router, "createPlaylist", &api_key, "&name=Long").await;
    let playlist = created["subsonic-response"]["playlist"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let post = |body: String| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(
                    // `f` in the query string as well: a body refused for its
                    // size is never read, so it cannot carry the format.
                    Request::post("/rest/updatePlaylist.view?f=json")
                        .header("content-type", "application/x-www-form-urlencoded")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            json_body(response).await
        }
    };

    let mut body = format!("apiKey={api_key}&v=1.16.1&c=golden&f=json&playlistId={playlist}");
    for track in &tracks {
        body.push_str("&songIdToAdd=");
        body.push_str(track);
    }
    assert!(
        body.len() > 64 * 1024,
        "the request must be one the old bound refused"
    );
    let filled = post(body).await;
    assert_eq!(
        filled["subsonic-response"]["status"], "ok",
        "{}",
        filled["subsonic-response"]["error"]
    );
    let listed = subsonic_json(&router, "getPlaylists", &api_key, "").await;
    assert_eq!(
        listed["subsonic-response"]["playlists"]["playlist"][0]["songCount"],
        ceiling
    );

    // One byte past the bound is refused before anything reads it, and the
    // refusal is the façade's own, in the body of an HTTP 200.
    let limit = waveflow_server::subsonic::MAX_FORM_BYTES;
    let mut over = format!("apiKey={api_key}&v=1.16.1&c=golden&f=json&padding=");
    over.push_str(&"a".repeat(limit + 1 - over.len()));
    assert_eq!(over.len(), limit + 1);
    let refused = post(over).await;
    assert_eq!(refused["subsonic-response"]["error"]["code"], 10);
    assert_eq!(
        refused["subsonic-response"]["error"]["message"],
        "Invalid form body"
    );
    // A body exactly at the bound is read: whatever refuses it, if anything,
    // is not its size.
    let mut exact = format!("apiKey={api_key}&v=1.16.1&c=golden&f=json&padding=");
    exact.push_str(&"a".repeat(limit - exact.len()));
    assert_eq!(exact.len(), limit);
    let read = post(exact).await;
    assert_ne!(
        read["subsonic-response"]["error"]["message"],
        "Invalid form body"
    );
}

/// `getPlaylists?username=` is an administrator's view of another account's
/// playlists, private ones included and read through the administrator's own
/// libraries; to anyone else, and for a name no account carries, it answers
/// error 70, the not-found every other refusal here gives. The writes stay
/// the owner's.
#[tokio::test]
async fn subsonic_username_is_an_administrators_view_and_blurred_for_everyone_else() {
    let (_temp, config, state) = test_app().await;
    let (owner, owner_key) = subsonic_account(&state, "named-owner").await;
    let (admin, admin_key) = subsonic_account(&state, "named-admin").await;
    let (_outsider, outsider_key) = subsonic_account(&state, "named-outsider").await;
    sqlx::query("UPDATE account SET role='admin' WHERE id=?")
        .bind(admin.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let (shared, seen) =
        seed_titles(&state, &config, owner, "named-shared", 4_300, &["Seen"]).await;
    let (_kept, unseen) =
        seed_titles(&state, &config, owner, "named-kept", 4_400, &["Unseen"]).await;
    state
        .db
        .add_library_member(owner, shared, admin, LibraryRole::Listener, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let created = subsonic_json(
        &router,
        "createPlaylist",
        &owner_key,
        &format!("&name=Private&songId={}&songId={}", seen[0], unseen[0]),
    )
    .await;
    let id = created["subsonic-response"]["playlist"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // That account's smart playlists (RFC-025): the public one is listed,
    // the private one is its owner's own and stays out, as `getPlaylist`
    // would answer it error 70.
    let rules = waveflow_server::services::SmartRules::parse(&serde_json::json!({
        "conditions": [{"field": "play_count", "operator": "less_than", "value": 5}],
    }))
    .unwrap();
    let shown = state
        .services
        .create_smart_playlist(owner, "Smart open", None, true, &rules)
        .await
        .unwrap();
    state
        .services
        .create_smart_playlist(owner, "Smart closed", None, false, &rules)
        .await
        .unwrap();

    let listed = subsonic_json(&router, "getPlaylists", &admin_key, "&username=named-owner").await;
    let listed = listed["subsonic-response"]["playlists"]["playlist"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let names: Vec<&str> = listed
        .iter()
        .filter_map(|playlist| playlist["name"].as_str())
        .collect();
    assert_eq!(names, ["Private", "Smart open"], "{listed:?}");
    assert_eq!(listed[1]["id"], shown.id.to_string().as_str());
    assert_eq!(listed[1]["readonly"], true);
    assert_eq!(listed[0]["id"], id.as_str());
    assert_eq!(listed[0]["owner"], "named-owner");
    assert_eq!(listed[0]["public"], false);
    assert_eq!(
        listed[0]["songCount"], 1,
        "counted through the administrator's libraries"
    );
    let opened = subsonic_json(&router, "getPlaylist", &admin_key, &format!("&id={id}")).await;
    assert_eq!(
        entry_ids(&opened["subsonic-response"]["playlist"]),
        vec![seen[0].to_string()]
    );
    // Its own listing is unchanged by the parameter naming itself: nothing
    // of its own, and the other account's public smart playlist, as any
    // reader is shown it.
    let own = subsonic_json(&router, "getPlaylists", &admin_key, "&username=NAMED-ADMIN").await;
    let own: Vec<String> = own["subsonic-response"]["playlists"]["playlist"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|playlist| playlist["name"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(own, ["Smart open"]);

    for (key, extra, who) in [
        (
            &outsider_key,
            "&username=named-owner",
            "a non-administrator",
        ),
        (&outsider_key, "&username=nobody-here", "an unknown name"),
        (
            &admin_key,
            "&username=nobody-here",
            "an unknown name to an admin",
        ),
    ] {
        let refused = subsonic_json(&router, "getPlaylists", key, extra).await;
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 70,
            "{who}: {refused}"
        );
    }
    let outsider_open =
        subsonic_json(&router, "getPlaylist", &outsider_key, &format!("&id={id}")).await;
    assert_eq!(outsider_open["subsonic-response"]["error"]["code"], 70);
    let outsider_self = subsonic_json(
        &router,
        "getPlaylists",
        &outsider_key,
        "&username=named-outsider",
    )
    .await;
    assert_eq!(outsider_self["subsonic-response"]["status"], "ok");

    let renamed = subsonic_json(
        &router,
        "updatePlaylist",
        &admin_key,
        &format!("&playlistId={id}&name=Taken"),
    )
    .await;
    assert_eq!(
        renamed["subsonic-response"]["error"]["code"], 70,
        "the administrator reads, and writes nothing"
    );
}

/// Subsonic has no move verb, so a client reorders by sending the whole list
/// again through `createPlaylist?playlistId=` — which replaces the track list
/// in the order given (RFC-020, what was still open).
#[tokio::test]
async fn subsonic_reorders_a_playlist_by_sending_it_again() {
    let (_temp, config, state) = test_app().await;
    let (owner, api_key) = subsonic_account(&state, "reorder-subsonic").await;
    let (_library, ids) = seed_titles(
        &state,
        &config,
        owner,
        "reorder-sub",
        4_500,
        &["One", "Two", "Three"],
    )
    .await;
    let router = waveflow_server::app(&config, state.clone());
    let created = subsonic_json(
        &router,
        "createPlaylist",
        &api_key,
        &format!(
            "&name=Order&songId={}&songId={}&songId={}",
            ids[0], ids[1], ids[2]
        ),
    )
    .await;
    let id = created["subsonic-response"]["playlist"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reordered = subsonic_json(
        &router,
        "createPlaylist",
        &api_key,
        &format!(
            "&playlistId={id}&songId={}&songId={}&songId={}",
            ids[2], ids[0], ids[1]
        ),
    )
    .await;
    assert_eq!(
        entry_ids(&reordered["subsonic-response"]["playlist"]),
        vec![ids[2].to_string(), ids[0].to_string(), ids[1].to_string()]
    );
    let read = subsonic_json(&router, "getPlaylist", &api_key, &format!("&id={id}")).await;
    assert_eq!(
        entry_ids(&read["subsonic-response"]["playlist"]),
        vec![ids[2].to_string(), ids[0].to_string(), ids[1].to_string()]
    );
}
