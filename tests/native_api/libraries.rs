//! The library routes: who may see a library, its lifecycle, and what a foreign
//! one answers.

use super::*;

#[tokio::test]
async fn catalog_and_scan_routes_blur_foreign_libraries() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("route-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("route-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("route-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Private.wav"));
    write_test_wav(&music.join("Private 2.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Private library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Private library".into(),
            root_path: root,
        },
    )
    .await;
    let router = waveflow_server::app(&config, state);
    let owner_token = login_token(&router, "route-owner", password).await;
    let intruder_token = login_token(&router, "route-intruder", password).await;

    let owner_response = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/libraries/{library_id}/tracks"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(owner_response.status(), StatusCode::OK);
    assert_eq!(json_body(owner_response).await.as_array().unwrap().len(), 2);

    let page = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v2/libraries/{library_id}/tracks?limit=1&offset=1"
            ))
            .header("authorization", format!("Bearer {owner_token}"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(json_body(page).await.as_array().unwrap().len(), 1);

    let invalid_page = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/libraries/{library_id}/tracks?limit=501"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_page.status(), StatusCode::UNPROCESSABLE_ENTITY);

    for (method, path) in [
        ("GET", "tracks"),
        ("POST", "scans"),
        // Membership answers 404 for the same reason the others do: telling a
        // stranger who the members are would first tell them the library is
        // there.
        ("GET", "members"),
    ] {
        let uri = format!("/api/v2/libraries/{library_id}/{path}");
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {intruder_token}"))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

/// Who may see a library, and in what standing.
///
/// `PUT` and `DELETE` on `/libraries/{id}/members/{user}` have existed since M4
/// with nothing to read them back, so an interface could grant and revoke
/// without ever showing who already had access — and no client can work that
/// out for itself, since an account's own membership says nothing about anyone
/// else's.
///
/// The refusal side lives in `catalog_and_scan_routes_blur_foreign_libraries`,
/// beside the other routes that answer 404 rather than confirm a library
/// exists. This is the half that says what the route returns when it does.
#[tokio::test]
async fn the_members_route_names_who_may_see_a_library() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("member-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let guest = state
        .db
        .create_account("member-guest", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("member-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Member library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO library_member (library_id, user_id, role, created_at) \
         VALUES (?, ?, 'listener', ?)",
    )
    .bind(library_id.to_string())
    .bind(guest.to_string())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();

    let router = waveflow_server::app(&config, state);
    let token = login_token(&router, "member-guest", password).await;
    let response = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/libraries/{library_id}/members"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let listed = json_body(response).await;
    let listed = listed.as_array().expect("a list");
    assert_eq!(listed.len(), 2, "the owner and the listener");
    // Ordered by username, so the answer does not depend on insertion order.
    assert_eq!(listed[0]["username"], "member-guest");
    assert_eq!(listed[0]["role"], "listener");
    assert_eq!(listed[1]["username"], "member-owner");
    assert_eq!(listed[1]["role"], "owner");
    // A listener sees the list: the route is about who may read the library,
    // and every member may ask. Restricting it to owners would leave a
    // listener unable to tell whether a library is shared at all.
    assert_eq!(listed[0]["user_id"], guest.to_string());

    // A library nobody has answers the same 404 as one the caller cannot see,
    // so the two cannot be told apart from outside.
    let missing = router
        .oneshot(
            Request::get("/api/v2/libraries/00000000-0000-4000-8000-000000000000/members")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

/// An account written straight into the table, with an API token minted for
/// it at run time: these routes need a bearer, never a password.
async fn account_with_token(
    state: &waveflow_server::AppState,
    username: &str,
    role: &str,
) -> (uuid::Uuid, String) {
    let id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-signs-in-by-token-only', ?, 0, ?, ?)",
    )
    .bind(id.to_string())
    .bind(username)
    .bind(role)
    .bind(now_ms())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let token = security::generate_token("wfapi_");
    state
        .db
        .create_api_token(
            waveflow_server::database::NewApiToken {
                actor_id: id,
                user_id: id,
                name: "lifecycle",
                token_hash: &security::token_hash(&token),
                scopes: &[],
                expires_at: None,
            },
            now_ms(),
        )
        .await
        .unwrap();
    (id, token)
}

fn bearer_json(method: &str, uri: &str, token: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// RFC-021 over HTTP: the routes of a library's lifecycle and of its missing
/// tracks, who each one answers, and the confirmation a deletion carries.
#[tokio::test]
async fn the_library_lifecycle_routes_answer_by_standing() {
    let (_temp, config, state) = test_app().await;
    let (_admin, admin_token) = account_with_token(&state, "lifecycle-admin", "admin").await;
    let (owner, owner_token) = account_with_token(&state, "lifecycle-owner", "user").await;
    let (listener, listener_token) = account_with_token(&state, "lifecycle-listener", "user").await;
    let music = config.data_dir.join("lifecycle-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Only.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Lifecycle",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner,
            library_id,
            listener,
            waveflow_server::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Lifecycle".into(),
            root_path: root.clone(),
        },
    )
    .await;
    let router = waveflow_server::app(&config, state);
    let library = format!("/api/v2/libraries/{library_id}");
    let send = |request: Request<Body>| {
        let router = router.clone();
        async move { router.oneshot(request).await.unwrap() }
    };

    // The owner renames; a listener cannot even see that there is something
    // to rename; the path is an administrator's.
    let renamed = send(bearer_json(
        "PATCH",
        &library,
        &owner_token,
        serde_json::json!({"name": "Renamed"}),
    ))
    .await;
    assert_eq!(renamed.status(), StatusCode::OK);
    assert!(json_body(renamed).await["scan_id"].is_null());
    let refused = send(bearer_json(
        "PATCH",
        &library,
        &listener_token,
        serde_json::json!({"name": "Mine"}),
    ))
    .await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    let repath = send(bearer_json(
        "PATCH",
        &library,
        &owner_token,
        serde_json::json!({"path": root.to_string_lossy()}),
    ))
    .await;
    assert_eq!(repath.status(), StatusCode::FORBIDDEN);
    let settings = send(bearer_json(
        "GET",
        &format!("{library}/settings"),
        &owner_token,
        serde_json::json!({}),
    ))
    .await;
    assert_eq!(settings.status(), StatusCode::FORBIDDEN);
    let settings = send(bearer_json(
        "GET",
        &format!("{library}/settings"),
        &admin_token,
        serde_json::json!({}),
    ))
    .await;
    assert_eq!(settings.status(), StatusCode::OK);
    let settings = json_body(settings).await;
    assert_eq!(settings["name"], "Renamed");
    assert_eq!(settings["follow_links"], false);

    // Missing tracks: the owner reads them, a listener does not, and a purge
    // that names nothing purges nothing.
    std::fs::remove_file(root.join("Only.wav")).unwrap();
    let scan = send(bearer_json(
        "POST",
        &format!("{library}/scans"),
        &owner_token,
        serde_json::json!({}),
    ))
    .await;
    assert_eq!(scan.status(), StatusCode::ACCEPTED);
    let scan_id = json_body(scan).await["scan_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Until the scan ends, and only a completed one counts: a failed or
    // unfinished scan would leave the assertions below reading a catalogue
    // halfway written.
    let mut status = serde_json::Value::Null;
    for _ in 0..500 {
        let job = send(bearer_json(
            "GET",
            &format!("/api/v2/scans/{scan_id}"),
            &owner_token,
            serde_json::json!({}),
        ))
        .await;
        status = json_body(job).await["status"].clone();
        if status == "completed" || status == "failed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(status, "completed", "the scan did not complete");
    let missing = send(bearer_json(
        "GET",
        &format!("{library}/unavailable-tracks"),
        &owner_token,
        serde_json::json!({}),
    ))
    .await;
    assert_eq!(missing.status(), StatusCode::OK);
    let missing = json_body(missing).await;
    assert_eq!(missing["total"], 1);
    assert_eq!(missing["tracks"][0]["relative_path"], "Only.wav");
    let hidden = send(bearer_json(
        "GET",
        &format!("{library}/unavailable-tracks"),
        &listener_token,
        serde_json::json!({}),
    ))
    .await;
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
    let neither = send(bearer_json(
        "POST",
        &format!("{library}/unavailable-tracks/purge"),
        &owner_token,
        serde_json::json!({}),
    ))
    .await;
    assert_eq!(neither.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let purged = send(bearer_json(
        "POST",
        &format!("{library}/unavailable-tracks/purge"),
        &owner_token,
        serde_json::json!({"all": true}),
    ))
    .await;
    assert_eq!(purged.status(), StatusCode::OK);
    assert_eq!(json_body(purged).await["purged"], 1);

    // A deletion is an administrator's, and carries the name it deletes.
    let by_owner = send(bearer_json(
        "DELETE",
        &library,
        &owner_token,
        serde_json::json!({"confirm": "Renamed"}),
    ))
    .await;
    assert_eq!(by_owner.status(), StatusCode::FORBIDDEN);
    let unconfirmed = send(bearer_json(
        "DELETE",
        &library,
        &admin_token,
        serde_json::json!({}),
    ))
    .await;
    assert_eq!(unconfirmed.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let mistyped = send(bearer_json(
        "DELETE",
        &library,
        &admin_token,
        serde_json::json!({"confirm": "renamed"}),
    ))
    .await;
    assert_eq!(mistyped.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let deleted = send(bearer_json(
        "DELETE",
        &library,
        &admin_token,
        serde_json::json!({"confirm": "Renamed"}),
    ))
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);
    let listed = send(bearer_json(
        "GET",
        "/api/v2/libraries",
        &owner_token,
        serde_json::json!({}),
    ))
    .await;
    assert_eq!(json_body(listed).await, serde_json::json!([]));
    let again = send(bearer_json(
        "DELETE",
        &library,
        &admin_token,
        serde_json::json!({"confirm": "Renamed"}),
    ))
    .await;
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

/// An administrator's account is not enough: a token whose scopes stop short
/// of `admin` acts with the account's memberships only, so it cannot rename,
/// purge or delete a library its account is not a member of — while the same
/// account on an unrestricted credential can.
#[tokio::test]
async fn a_narrowed_admin_token_has_no_say_over_a_foreign_library() {
    let (_temp, config, state) = test_app().await;
    let (admin, full_token) = account_with_token(&state, "scoped-admin", "admin").await;
    let (owner, _) = account_with_token(&state, "scoped-owner", "user").await;
    let write_token = security::generate_token("wfapi_");
    state
        .db
        .create_api_token(
            waveflow_server::database::NewApiToken {
                actor_id: admin,
                user_id: admin,
                name: "write only",
                token_hash: &security::token_hash(&write_token),
                scopes: &["write".to_owned()],
                expires_at: None,
            },
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("scoped-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Foreign",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let library = format!("/api/v2/libraries/{library_id}");
    let send = |request: Request<Body>| {
        let router = router.clone();
        async move { router.oneshot(request).await.unwrap().status() }
    };

    assert_eq!(
        send(bearer_json(
            "PATCH",
            &library,
            &write_token,
            serde_json::json!({"name": "Taken"}),
        ))
        .await,
        StatusCode::NOT_FOUND,
        "a narrowed token renamed a library its account is not a member of"
    );
    assert_eq!(
        send(bearer_json(
            "POST",
            &format!("{library}/unavailable-tracks/purge"),
            &write_token,
            serde_json::json!({"all": true}),
        ))
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(bearer_json(
            "GET",
            &format!("{library}/unavailable-tracks"),
            &write_token,
            serde_json::json!({}),
        ))
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(bearer_json(
            "DELETE",
            &library,
            &write_token,
            serde_json::json!({"confirm": "Foreign"}),
        ))
        .await,
        StatusCode::FORBIDDEN
    );
    let name: String = sqlx::query_scalar("SELECT name FROM library WHERE id = ?")
        .bind(library_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(name, "Foreign");

    // The same account, unrestricted: its authority is an administrator's.
    assert_eq!(
        send(bearer_json(
            "PATCH",
            &library,
            &full_token,
            serde_json::json!({"name": "Renamed by the operator"}),
        ))
        .await,
        StatusCode::OK
    );
    assert_eq!(
        send(bearer_json(
            "DELETE",
            &library,
            &full_token,
            serde_json::json!({"confirm": "Renamed by the operator"}),
        ))
        .await,
        StatusCode::OK
    );
}
