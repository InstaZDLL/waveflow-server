//! Smart playlists (RFC-025 decisions 1 to 6): rules validated against an
//! allowlist, evaluated in one static statement under the owner's tenancy,
//! read through the viewer's libraries, and shown read-only by the façade.

use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::storage::catalog::CatalogTrackInput;
use waveflow_server::storage::database::{AccountRole, LibraryRole, LibraryVisibility};

#[allow(dead_code)]
mod support;
use support::*;

/// An account with a password and a Subsonic key, both drawn at run time.
async fn account(state: &waveflow_server::AppState, username: &str) -> (Uuid, String, String) {
    let password = Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let id = state
        .db
        .create_account(username, &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let api_key = format!("wfsk_{}", Uuid::new_v4().simple());
    state
        .db
        .set_subsonic_credential(
            id,
            id,
            &state
                .secret_box
                .encrypt(Uuid::new_v4().to_string().as_bytes())
                .unwrap(),
            &security::token_hash(&api_key),
            now_ms(),
        )
        .await
        .unwrap();
    (id, password, api_key)
}

struct Seed {
    title: &'static str,
    artist: &'static str,
    genre: &'static str,
    year: i64,
    seconds: i64,
    extension: &'static str,
}

/// A library of `owner`'s holding these tracks, and their ids in order.
async fn library(
    state: &waveflow_server::AppState,
    config: &waveflow_server::Config,
    owner: Uuid,
    name: &str,
    first: usize,
    seeds: &[Seed],
) -> (Uuid, Vec<Uuid>) {
    let root = config.data_dir.join(name);
    std::fs::create_dir_all(&root).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            name,
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
    state
        .db
        .start_scan_job(scan, seeds.len() as i64, false)
        .await
        .unwrap();
    for (offset, seed) in seeds.iter().enumerate() {
        let index = first + offset;
        let input = CatalogTrackInput {
            relative_path: format!("smart-{index}.{}", seed.extension),
            genre: Some(seed.genre.into()),
            year: Some(seed.year),
            duration_ms: seed.seconds * 1000,
            ..browse_input(
                index,
                seed.title,
                &format!("{name} album"),
                seed.artist,
                Some(offset as i64 + 1),
                Some(1),
            )
        };
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
    let mut ids = Vec::new();
    for seed in seeds {
        let id: String = sqlx::query_scalar("SELECT id FROM track WHERE library_id=? AND title=?")
            .bind(library.to_string())
            .bind(seed.title)
            .fetch_one(state.db.pool())
            .await
            .unwrap();
        ids.push(id.parse().unwrap());
    }
    (library, ids)
}

const SEEDS: [Seed; 5] = [
    Seed {
        title: "Army of Me",
        artist: "Björk",
        genre: "Rock",
        year: 1995,
        seconds: 234,
        extension: "flac",
    },
    Seed {
        title: "Hyperballad",
        artist: "Björk",
        genre: "Electronic",
        year: 1995,
        seconds: 321,
        extension: "mp3",
    },
    Seed {
        title: "Paranoid Android",
        artist: "Radiohead",
        genre: "Rock",
        year: 1997,
        seconds: 387,
        extension: "flac",
    },
    Seed {
        title: "Idioteque",
        artist: "Radiohead",
        genre: "Electronic",
        year: 2000,
        seconds: 309,
        extension: "flac",
    },
    Seed {
        title: "' OR 1=1 --",
        artist: "Nobody",
        genre: "Noise",
        year: 2010,
        seconds: 60,
        extension: "ogg",
    },
];

fn titles(songs: &serde_json::Value) -> Vec<String> {
    songs
        .as_array()
        .expect("a list of songs")
        .iter()
        .map(|song| song["title"].as_str().unwrap().to_owned())
        .collect()
}

async fn preview(router: &axum::Router, token: &str, rules: serde_json::Value) -> Vec<String> {
    let response = send_json(
        router,
        "POST",
        "/api/v2/smart-playlists/preview",
        token,
        Some(json!({ "rules": rules })),
    )
    .await;
    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    titles(&body["songs"])
}

/// Decisions 2 and 3: every field kind, both connectives, a sort in each
/// direction and a limit, answered by the one statement.
#[tokio::test]
async fn rules_select_sort_and_limit_what_the_owner_can_read() {
    let (_temp, config, state) = test_app().await;
    let (owner, password, _) = account(&state, "smart-owner").await;
    let (_, ids) = library(&state, &config, owner, "smart-lib", 7_000, &SEEDS).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "smart-owner", &password).await;

    // all: Rock from the nineties, by year then title.
    assert_eq!(
        preview(
            &router,
            &token,
            json!({
                "match": "all",
                "conditions": [
                    {"field": "genre", "operator": "is", "value": "rock"},
                    {"field": "year", "operator": "between", "value": [1990, 1999]},
                ],
                "sort": "year", "order": "desc",
            })
        )
        .await,
        vec!["Paranoid Android", "Army of Me"]
    );
    // any: the union, limited.
    assert_eq!(
        preview(
            &router,
            &token,
            json!({
                "match": "any",
                "conditions": [
                    {"field": "artist", "operator": "is", "value": "BJÖRK"},
                    {"field": "year", "operator": "greater_than", "value": 1999},
                ],
                "sort": "title", "limit": 3,
            })
        )
        .await,
        vec!["' OR 1=1 --", "Army of Me", "Hyperballad"]
    );
    // Text operators, and a value that looks like SQL is only ever a value:
    // it matches the one title that contains it, and nothing else.
    assert_eq!(
        preview(
            &router,
            &token,
            json!({"conditions": [{"field": "title", "operator": "contains", "value": "' OR 1=1"}]})
        )
        .await,
        vec!["' OR 1=1 --"]
    );
    assert_eq!(
        preview(
            &router,
            &token,
            json!({"conditions": [
                {"field": "format", "operator": "is_not", "value": "flac"},
                {"field": "artist", "operator": "is_not", "value": "Nobody"},
            ]})
        )
        .await,
        vec!["Hyperballad"]
    );
    assert_eq!(
        preview(
            &router,
            &token,
            json!({"conditions": [{"field": "duration", "operator": "less_than", "value": 300}],
                   "sort": "duration"})
        )
        .await,
        vec!["' OR 1=1 --", "Army of Me"]
    );

    // The owner's own annotations: a star, a rating, two plays.
    state
        .services
        .set_star(owner, "track", ids[3], true)
        .await
        .unwrap();
    state
        .services
        .set_rating(owner, "track", ids[2], 5)
        .await
        .unwrap();
    for _ in 0..2 {
        state
            .services
            .scrobble(owner, ids[1], true, None)
            .await
            .unwrap();
    }
    assert_eq!(
        preview(
            &router,
            &token,
            json!({"conditions": [{"field": "starred", "operator": "is", "value": true}]})
        )
        .await,
        vec!["Idioteque"]
    );
    assert_eq!(
        preview(
            &router,
            &token,
            json!({"conditions": [{"field": "rating", "operator": "greater_than", "value": 3}]})
        )
        .await,
        vec!["Paranoid Android"]
    );
    assert_eq!(
        preview(
            &router,
            &token,
            json!({"conditions": [{"field": "last_played", "operator": "in_the_last", "value": 7}]})
        )
        .await,
        vec!["Hyperballad"]
    );
    assert_eq!(
        preview(
            &router,
            &token,
            json!({"sort": "play_count", "order": "desc", "limit": 1})
        )
        .await,
        vec!["Hyperballad"]
    );

    // Stored, then read back evaluated; a change to the catalogue shows on
    // the next read with nothing rebuilt.
    let created = send_json(
        &router,
        "POST",
        "/api/v2/smart-playlists",
        &token,
        Some(json!({
            "name": "Starred",
            "rules": {"conditions": [{"field": "starred", "operator": "is", "value": true}]},
        })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    let id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(titles(&created["songs"]), vec!["Idioteque"]);
    state
        .services
        .set_star(owner, "track", ids[0], true)
        .await
        .unwrap();
    let read = json_body(
        send_json(
            &router,
            "GET",
            &format!("/api/v2/smart-playlists/{id}"),
            &token,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(titles(&read["songs"]), vec!["Army of Me", "Idioteque"]);
    assert_eq!(read["song_count"], 2);
    let listed =
        json_body(send_json(&router, "GET", "/api/v2/smart-playlists", &token, None).await).await;
    assert_eq!(listed[0]["song_count"], 2);
    assert!(
        listed[0].get("songs").is_none(),
        "a listing carries counts, not tracks"
    );

    // The rules are replaced whole by a patch, and checked as a creation's are.
    let refused = send_json(
        &router,
        "PATCH",
        &format!("/api/v2/smart-playlists/{id}"),
        &token,
        Some(json!({"rules": {"match": "nand"}})),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let patched = send_json(
        &router,
        "PATCH",
        &format!("/api/v2/smart-playlists/{id}"),
        &token,
        Some(json!({"name": "Nineties", "rules": {"conditions": [
            {"field": "year", "operator": "less_than", "value": 2000}], "sort": "title"}})),
    )
    .await;
    assert_eq!(patched.status(), StatusCode::OK);
    let patched = json_body(patched).await;
    assert_eq!(patched["name"], "Nineties");
    assert_eq!(
        titles(&patched["songs"]),
        vec!["Army of Me", "Hyperballad", "Paranoid Android"]
    );
    let deleted = send_json(
        &router,
        "DELETE",
        &format!("/api/v2/smart-playlists/{id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
}

/// Decision 2: anything outside the allowlist is a 422 naming where it is, and
/// nothing is stored. The connective is the one Ampache 8.4.0 concatenated.
#[tokio::test]
async fn rules_outside_the_allowlist_are_refused_and_nothing_is_stored() {
    let (_temp, config, state) = test_app().await;
    let (owner, password, _) = account(&state, "smart-hostile").await;
    library(&state, &config, owner, "smart-hostile", 7_100, &SEEDS[..2]).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "smart-hostile", &password).await;
    for (rules, at) in [
        (json!({"match": "all) OR 1=1 --"}), "match"),
        (json!({"match": "or"}), "match"),
        (
            json!({"conditions": [{"field": "t.title", "operator": "is", "value": "x"}]}),
            "conditions[0].field",
        ),
        (
            json!({"conditions": [{"field": "title", "operator": "LIKE", "value": "%"}]}),
            "conditions[0].operator",
        ),
        (
            json!({"conditions": [{"field": "year", "operator": "is", "value": "1995 OR 1"}]}),
            "conditions[0].value",
        ),
        (json!({"sort": "random()"}), "sort"),
        (json!({"limit": 1_000_000}), "limit"),
    ] {
        let response = send_json(
            &router,
            "POST",
            "/api/v2/smart-playlists",
            &token,
            Some(json!({"name": "Hostile", "rules": rules})),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = json_body(response).await;
        assert_eq!(body["code"], "invalid_rules");
        assert_eq!(body["at"], at, "{body}");
    }
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM smart_playlist")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(stored, 0);
    // And the catalogue the injection would have reached is intact.
    let tracks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM track")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(tracks, 2);
}

/// Decision 5: a playlist is evaluated as its owner and read through the
/// viewer's own libraries. A private one is another account's 404, public or
/// not it is read-only to anyone but its owner, and the façade shows it.
#[tokio::test]
async fn a_public_smart_playlist_is_read_through_the_viewers_libraries_and_only_read() {
    let (_temp, config, state) = test_app().await;
    let (owner, owner_password, owner_key) = account(&state, "smart-pub-owner").await;
    let (viewer, viewer_password, viewer_key) = account(&state, "smart-pub-viewer").await;
    let (shared, _) = library(&state, &config, owner, "smart-shared", 7_200, &SEEDS[..2]).await;
    library(&state, &config, owner, "smart-private", 7_300, &SEEDS[2..4]).await;
    state
        .db
        .add_library_member(owner, shared, viewer, LibraryRole::Listener, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "smart-pub-owner", &owner_password).await;
    let viewer_token = login_token(&router, "smart-pub-viewer", &viewer_password).await;

    let created = json_body(
        send_json(
            &router,
            "POST",
            "/api/v2/smart-playlists",
            &owner_token,
            Some(json!({
                "name": "Everything",
                "rules": {"conditions": [{"field": "year", "operator": "less_than", "value": 2001}],
                          "sort": "title"},
            })),
        )
        .await,
    )
    .await;
    let id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["song_count"], 4);

    // Private: another account's 404, on every surface.
    let foreign = send_json(
        &router,
        "GET",
        &format!("/api/v2/smart-playlists/{id}"),
        &viewer_token,
        None,
    )
    .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    let listed = subsonic_json(&router, "getPlaylists", &viewer_key, "").await;
    assert!(listed["subsonic-response"]["playlists"]["playlist"]
        .as_array()
        .is_none_or(|all| all.is_empty()));

    send_json(
        &router,
        "PATCH",
        &format!("/api/v2/smart-playlists/{id}"),
        &owner_token,
        Some(json!({"public": true})),
    )
    .await;
    let read = json_body(
        send_json(
            &router,
            "GET",
            &format!("/api/v2/smart-playlists/{id}"),
            &viewer_token,
            None,
        )
        .await,
    )
    .await;
    // The viewer sees what they may play of it, and nothing else.
    assert_eq!(titles(&read["songs"]), vec!["Army of Me", "Hyperballad"]);
    assert_eq!(read["owner_username"], "smart-pub-owner");

    // Read-only to anyone but its owner: the same 404 a missing one answers.
    for (method, body) in [
        ("PATCH", Some(json!({"name": "Mine now"}))),
        ("DELETE", None),
    ] {
        let refused = send_json(
            &router,
            method,
            &format!("/api/v2/smart-playlists/{id}"),
            &viewer_token,
            body,
        )
        .await;
        assert_eq!(refused.status(), StatusCode::NOT_FOUND, "{method}");
    }

    // The façade: listed after the ordinary playlists, read-only, counted over
    // what the viewer can play.
    let listed = subsonic_json(&router, "getPlaylists", &viewer_key, "").await;
    let listed = listed["subsonic-response"]["playlists"]["playlist"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], id.as_str());
    assert_eq!(listed[0]["readonly"], true);
    assert_eq!(listed[0]["songCount"], 2);
    assert_eq!(listed[0]["owner"], "smart-pub-owner");
    let one = subsonic_json(&router, "getPlaylist", &viewer_key, &format!("&id={id}")).await;
    let entries = one["subsonic-response"]["playlist"]["entry"]
        .as_array()
        .unwrap();
    assert_eq!(entries.len(), 2);
    let owners = subsonic_json(&router, "getPlaylist", &owner_key, &format!("&id={id}")).await;
    assert_eq!(
        owners["subsonic-response"]["playlist"]["entry"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    // No façade write reaches it, the owner's included: error 70.
    for (method, extra) in [
        ("updatePlaylist", format!("&playlistId={id}&name=Renamed")),
        ("deletePlaylist", format!("&id={id}")),
    ] {
        let refused = subsonic_json(&router, method, &owner_key, &extra).await;
        assert_eq!(refused["subsonic-response"]["status"], "failed", "{method}");
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 70,
            "{method}"
        );
    }
    // An ordinary playlist the owner makes still lists first.
    let _ = viewer;
    subsonic_json(&router, "createPlaylist", &owner_key, "&name=Plain").await;
    let mine = subsonic_json(&router, "getPlaylists", &owner_key, "").await;
    let mine = mine["subsonic-response"]["playlists"]["playlist"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(mine[0]["name"], "Plain");
    assert!(
        mine[0].get("readonly").is_none(),
        "an ordinary playlist is unchanged on the wire"
    );
    assert_eq!(mine[1]["readonly"], true);
}

/// Decision 4: a listing evaluates nothing. Another account publishing many
/// expensive smart playlists costs a reader's `getPlaylists` nothing beyond
/// reading the rows; the counts a listing carries are what that reader was
/// last shown, and none before they open one.
#[tokio::test]
async fn listing_smart_playlists_evaluates_none_of_them() {
    let (_temp, config, state) = test_app().await;
    let (owner, owner_password, _) = account(&state, "smart-many-owner").await;
    let (viewer, viewer_password, viewer_key) = account(&state, "smart-many-viewer").await;
    let (shared, _) = library(&state, &config, owner, "smart-many", 7_400, &SEEDS).await;
    state
        .db
        .add_library_member(owner, shared, viewer, LibraryRole::Listener, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "smart-many-owner", &owner_password).await;
    let viewer_token = login_token(&router, "smart-many-viewer", &viewer_password).await;
    let mut ids = Vec::new();
    for index in 0..30 {
        // The most expensive shape there is: every correlated column, sorted.
        let created = send_json(
            &router,
            "POST",
            "/api/v2/smart-playlists",
            &owner_token,
            Some(json!({
                "name": format!("Heavy {index}"),
                "public": true,
                "rules": {"match": "any", "conditions": [
                    {"field": "play_count", "operator": "greater_than", "value": 0},
                    {"field": "last_played", "operator": "not_in_the_last", "value": 1},
                    {"field": "artist", "operator": "contains", "value": "a"},
                ], "sort": "last_played", "order": "desc"},
            })),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        ids.push(json_body(created).await["id"].as_str().unwrap().to_owned());
    }

    let before = state.services.smart_playlist_evaluations();
    let native = json_body(
        send_json(
            &router,
            "GET",
            "/api/v2/smart-playlists?include_public=true",
            &viewer_token,
            None,
        )
        .await,
    )
    .await;
    let listed = subsonic_json(&router, "getPlaylists", &viewer_key, "").await;
    assert_eq!(
        state.services.smart_playlist_evaluations(),
        before,
        "a listing evaluated rule sets"
    );
    assert_eq!(native.as_array().unwrap().len(), 30);
    assert!(
        native[0]["song_count"].is_null(),
        "never opened, so never counted"
    );
    let playlists = listed["subsonic-response"]["playlists"]["playlist"]
        .as_array()
        .unwrap();
    assert_eq!(playlists.len(), 30);
    assert_eq!(playlists[0]["songCount"], 0);
    assert!(playlists[0].get("validUntil").is_none());

    // Opened once: one evaluation, and the listings after it say what it found.
    let opened = subsonic_json(
        &router,
        "getPlaylist",
        &viewer_key,
        &format!("&id={}", ids[29]),
    )
    .await;
    assert_eq!(opened["subsonic-response"]["playlist"]["songCount"], 5);
    assert_eq!(state.services.smart_playlist_evaluations(), before + 1);
    let listed = subsonic_json(&router, "getPlaylists", &viewer_key, "").await;
    assert_eq!(state.services.smart_playlist_evaluations(), before + 1);
    let newest = &listed["subsonic-response"]["playlists"]["playlist"][0];
    assert_eq!(newest["id"], ids[29].as_str());
    assert_eq!(newest["songCount"], 5);
    assert!(newest.get("validUntil").is_some());

    // A rule edit makes what was shown stale, and it is no longer reported.
    send_json(
        &router,
        "PATCH",
        &format!("/api/v2/smart-playlists/{}", ids[29]),
        &owner_token,
        Some(json!({"rules": {"conditions": [
            {"field": "title", "operator": "is", "value": "Idioteque"}]}})),
    )
    .await;
    let after_edit = json_body(
        send_json(
            &router,
            "GET",
            "/api/v2/smart-playlists?include_public=true",
            &viewer_token,
            None,
        )
        .await,
    )
    .await;
    let edited = after_edit
        .as_array()
        .unwrap()
        .iter()
        .find(|playlist| playlist["id"] == ids[29].as_str())
        .unwrap();
    assert!(edited["song_count"].is_null(), "{edited}");
}

/// Decision 4: one evaluation runs under a budget of work, and one that passes
/// it is stopped — answered 503, nothing partial — and leaves its connection
/// as it found it.
#[tokio::test]
async fn an_evaluation_that_passes_its_budget_is_stopped() {
    let (_temp, config, state) = test_app().await;
    let (owner, password, api_key) = account(&state, "smart-budget").await;
    library(&state, &config, owner, "smart-budget", 7_500, &SEEDS).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "smart-budget", &password).await;
    let rules = json!({"conditions": [{"field": "play_count", "operator": "less_than", "value": 5}],
                       "sort": "play_count"});
    let created = json_body(
        send_json(
            &router,
            "POST",
            "/api/v2/smart-playlists",
            &token,
            Some(json!({"name": "Budget", "rules": rules})),
        )
        .await,
    )
    .await;
    let id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["song_count"], 5);

    state.services.limit_smart_playlist_evaluation(1);
    let stopped = send_json(
        &router,
        "GET",
        &format!("/api/v2/smart-playlists/{id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(stopped.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json_body(stopped).await["code"], "evaluation_budget");
    let previewed = send_json(
        &router,
        "POST",
        "/api/v2/smart-playlists/preview",
        &token,
        Some(json!({ "rules": rules })),
    )
    .await;
    assert_eq!(previewed.status(), StatusCode::SERVICE_UNAVAILABLE);
    let facade = subsonic_json(&router, "getPlaylist", &api_key, &format!("&id={id}")).await;
    assert_eq!(facade["subsonic-response"]["status"], "failed");

    // Every connection of the pool answers ordinary reads afterwards: no
    // handler was left behind on any of them.
    state
        .services
        .limit_smart_playlist_evaluation(waveflow_server::services::SMART_EVALUATION_BUDGET_OPS);
    for _ in 0..12 {
        let read = send_json(&router, "GET", "/api/v2/smart-playlists", &token, None).await;
        assert_eq!(read.status(), StatusCode::OK);
        let full = send_json(
            &router,
            "GET",
            &format!("/api/v2/smart-playlists/{id}"),
            &token,
            None,
        )
        .await;
        assert_eq!(full.status(), StatusCode::OK);
    }
}

/// Decision 4, and a deadlock a review found: reading a smart playlist held a
/// pooled connection while its evaluation waited for a second one, so as many
/// concurrent reads as the pool has connections stalled the whole server until
/// the acquire timeout. Twelve concurrent reads, and twelve previews, against
/// a pool of four: every one completes, and no more evaluations than half the
/// pool ever run at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reads_beyond_the_pool_size_all_complete() {
    let (_temp, config, state) = test_app().await;
    assert_eq!(config.db_max_connections, 4);
    let (owner, _, _) = account(&state, "smart-pool").await;
    library(&state, &config, owner, "smart-pool", 7_600, &SEEDS).await;
    let rules = waveflow_server::services::SmartRules::parse(&json!({
        "conditions": [{"field": "play_count", "operator": "less_than", "value": 5}],
        "sort": "play_count",
    }))
    .unwrap();
    let created = state
        .services
        .create_smart_playlist(owner, "Pool", None, true, &rules)
        .await
        .unwrap();
    let reads: Vec<_> = (0..12)
        .map(|_| {
            let services = state.services.clone();
            tokio::spawn(async move { services.smart_playlist(owner, created.id).await })
        })
        .collect();
    let previews: Vec<_> = (0..12)
        .map(|_| {
            let services = state.services.clone();
            let rules = rules.clone();
            tokio::spawn(async move {
                services
                    .preview_smart_playlist(owner, &rules)
                    .await
                    .map(|preview| preview.song_count)
            })
        })
        .collect();
    let all = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let mut counts = Vec::new();
        for read in reads {
            counts.push(read.await.unwrap().unwrap().song_count.unwrap());
        }
        for preview in previews {
            counts.push(preview.await.unwrap().unwrap());
        }
        counts
    })
    .await
    .expect("concurrent reads stalled on the pool");
    assert_eq!(all, vec![5; 24]);
    assert!(
        state.services.smart_playlist_peak_evaluations() <= 2,
        "{} evaluations ran at once against a pool of four",
        state.services.smart_playlist_peak_evaluations()
    );
}

/// A library id is matched whatever spelling the client sent it in: compared
/// as sent, capitals made `is` match nothing and `is_not` match everything.
#[tokio::test]
async fn a_library_condition_matches_whatever_spelling_the_id_was_sent_in() {
    let (_temp, config, state) = test_app().await;
    let (owner, password, _) = account(&state, "smart-library").await;
    let (first, _) = library(&state, &config, owner, "smart-lib-a", 7_700, &SEEDS[..2]).await;
    library(&state, &config, owner, "smart-lib-b", 7_800, &SEEDS[2..4]).await;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "smart-library", &password).await;
    for sent in [first.to_string().to_uppercase(), format!("{{{first}}}")] {
        assert_eq!(
            preview(
                &router,
                &token,
                json!({"conditions": [{"field": "library", "operator": "is", "value": sent}],
                       "sort": "title"})
            )
            .await,
            vec!["Army of Me", "Hyperballad"],
            "{sent}"
        );
        assert_eq!(
            preview(
                &router,
                &token,
                json!({"conditions": [{"field": "library", "operator": "is_not", "value": sent}],
                       "sort": "title"})
            )
            .await,
            vec!["Idioteque", "Paranoid Android"],
            "{sent}"
        );
    }
}
