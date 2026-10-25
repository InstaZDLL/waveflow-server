//! The `/api/v2` surface.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Method;
use axum::http::Request;
use axum::http::StatusCode;
use futures_util::future::BoxFuture;
use tower::ServiceExt;
use waveflow_server::authentication::now_ms;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;
use waveflow_server::services::ServiceError;
use waveflow_server::services::MAX_HISTORY_LIMIT;
use waveflow_server::services::{
    ScrobbleEnvelope, ScrobbleProvider, ScrobbleTarget, ScrobbleVerdict,
};

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

/// A destination that never says whether it recorded anything.
///
/// The whole of decision 5 in three lines: the queue cannot know, so a person
/// has to. Local to this target because the doubles in `tests/scrobbling.rs`
/// belong to a file whose accounts are inserted with a placeholder hash and
/// never log in — which is precisely what a route test needs them to do.
struct AlwaysAmbiguous;

impl ScrobbleTarget for AlwaysAmbiguous {
    fn submit<'a>(
        &'a self,
        _envelope: &'a ScrobbleEnvelope,
        _secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict> {
        Box::pin(async { ScrobbleVerdict::Ambiguous })
    }
}

/// An account poses and withdraws its own authorisation, and sees only its own.
///
/// RFC-010 decision 9: a destination's base URL belongs to the deployment, an
/// authorisation belongs to a person. So these routes are self-scoped, and the
/// thing a route can get wrong that a service method cannot is *whose* id it
/// passes — which is why a second account is here rather than only the first.
#[tokio::test]
async fn an_account_poses_and_withdraws_its_own_scrobble_link() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    for name in ["scrobble-owner", "scrobble-stranger"] {
        state
            .db
            .create_account(name, &hash, AccountRole::User, now_ms())
            .await
            .unwrap();
    }
    let router = waveflow_server::app(&config, state.clone());

    let bearer = |username: &'static str| {
        let router = router.clone();
        async move {
            let login = router
                .oneshot(json_request(
                    "/api/v2/auth/login",
                    serde_json::json!({
                        "username": username,
                        "password": "correct horse battery staple",
                        "device_name": "Integration"
                    }),
                ))
                .await
                .unwrap();
            json_body(login).await["access_token"]
                .as_str()
                .unwrap()
                .to_owned()
        }
    };
    let owner = bearer("scrobble-owner").await;
    let stranger = bearer("scrobble-stranger").await;

    let send = |method: Method, path: String, access: String, body: Option<serde_json::Value>| {
        let router = router.clone();
        async move {
            let mut request = Request::builder().method(method).uri(path);
            if !access.is_empty() {
                request = request.header("authorization", format!("Bearer {access}"));
            }
            let request = match body {
                Some(body) => request
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
                None => request.body(Body::empty()).unwrap(),
            };
            router.oneshot(request).await.unwrap()
        }
    };

    // Nothing linked yet, and asking is not an error.
    let response = send(
        Method::GET,
        "/api/v2/scrobble-links".into(),
        owner.clone(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);

    let response = send(
        Method::PUT,
        "/api/v2/scrobble-links/listenbrainz/default".into(),
        owner.clone(),
        Some(serde_json::json!({"secret": "lb-token"})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = send(
        Method::GET,
        "/api/v2/scrobble-links".into(),
        owner.clone(),
        None,
    )
    .await;
    let links = json_body(response).await;
    assert_eq!(links.as_array().unwrap().len(), 1);
    // The spelling a client reads back is the one it may send in. Four of them
    // are held together by a unit test; this is the fifth reader.
    assert_eq!(links[0]["provider"], "listenbrainz");
    assert_eq!(links[0]["health"], "healthy");
    assert_eq!(links[0]["pending"], 0);
    // Counters, never content, and never the secret: decision 12.
    assert!(links[0].get("secret").is_none());

    // The stranger's own list stays empty, which is the half a single-account
    // test cannot distinguish from a route that ignores the caller entirely.
    let response = send(
        Method::GET,
        "/api/v2/scrobble-links".into(),
        stranger.clone(),
        None,
    )
    .await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);

    // A destination this server cannot name is refused before anything is
    // sealed, and refused as a bad request rather than a missing one.
    let response = send(
        Method::PUT,
        "/api/v2/scrobble-links/spotify/default".into(),
        owner.clone(),
        Some(serde_json::json!({"secret": "whatever"})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // And an instance nobody declared is a different refusal, on a recipient
    // this server does know. A member picks among the names the server
    // publishes; a path naming one it never published is a resource that is not
    // there, not a request it cannot parse. Without this case the 422 above
    // would be the only refusal covered, and the two arrive from different
    // places for different reasons.
    let response = send(
        Method::PUT,
        "/api/v2/scrobble-links/listenbrainz/an-instance-nobody-declared".into(),
        owner.clone(),
        Some(serde_json::json!({"secret": "lb-token"})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // A secret that is nothing but spaces is refused for a destination this
    // server *does* know, which is the half the case above cannot reach: there
    // the provider parse answers first, and the secret is never looked at. The
    // spaces matter too — an empty string would pass a check written before the
    // trim rather than after it, and this one would not.
    let response = send(
        Method::PUT,
        "/api/v2/scrobble-links/listenbrainz/default".into(),
        owner.clone(),
        Some(serde_json::json!({"secret": "   "})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // And the authorisation posed earlier is untouched: a refusal replaces
    // nothing.
    let response = send(
        Method::GET,
        "/api/v2/scrobble-links".into(),
        owner.clone(),
        None,
    )
    .await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 1);

    // And no credential route is reachable without one.
    let response = send(
        Method::GET,
        "/api/v2/scrobble-links".into(),
        String::new(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = send(
        Method::DELETE,
        "/api/v2/scrobble-links/listenbrainz/default".into(),
        owner.clone(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = send(Method::GET, "/api/v2/scrobble-links".into(), owner, None).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);
}

/// The two gestures decision 13 grants, over HTTP.
///
/// The domain half is pinned in `tests/scrobbling.rs`. What only this can show
/// is that the routes exist, name an entry the way the outside names it, and
/// answer a stranger's id with a 404 rather than a decision.
#[tokio::test]
async fn an_uncertain_entry_is_listed_and_answered_over_http() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("uncertain-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("uncertain-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Uncertain",
            &std::fs::canonicalize(&music).unwrap(),
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
    let mut input = browse_input(240, "Ambiguous", "Unknowns", "Nobody", Some(1), Some(1));
    input.relative_path = "uncertain-0.flac".into();
    input.quick_hash = format!("{:064x}", 71_000);
    input.full_hash = format!("{:064x}", 72_000);
    state
        .db
        .apply_catalog_track(library, scan, &input, None, false)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let track = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .songs[0]
        .id;

    state
        .services
        .link_scrobble(owner, ScrobbleProvider::ListenBrainz, "default", "lb-token")
        .await
        .unwrap();
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::new(AlwaysAmbiguous) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .scrobble(owner, track, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let login = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "uncertain-owner",
                "password": "correct horse battery staple",
                "device_name": "Integration"
            }),
        ))
        .await
        .unwrap();
    let access = json_body(login).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let send = |method: Method, path: String| {
        let router = router.clone();
        let access = access.clone();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {access}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };

    let response = send(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let waiting = json_body(response).await;
    assert_eq!(waiting.as_array().unwrap().len(), 1);
    let entry = waiting[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(waiting[0]["provider"], "listenbrainz");
    // The envelope stays where it is: a state, never an echo of what was heard.
    assert!(waiting[0].get("title").is_none());

    // **Somebody else's id, not nobody's.** This assertion used to send a fresh
    // `Uuid::new_v4()`, which belongs to no account at all — a handler that had
    // dropped the tenancy binding entirely and looked the entry up by
    // `public_id` alone would have answered 404 to that and passed. The thing a
    // route can get wrong that a service method cannot is *whose* id it passes,
    // so the id below is the owner's real entry, presented by a stranger.
    state
        .db
        .create_account("uncertain-stranger", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let stranger_login = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "uncertain-stranger",
                "password": "correct horse battery staple",
                "device_name": "Integration"
            }),
        ))
        .await
        .unwrap();
    let stranger_access = json_body(stranger_login).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let send_as_stranger = |method: Method, path: String| {
        let router = router.clone();
        let access = stranger_access.clone();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {access}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };

    // Their own queue is empty, and the owner's entry is not theirs to answer.
    let response = send_as_stranger(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);
    let response = send_as_stranger(
        Method::POST,
        format!("/api/v2/scrobble-queue/uncertain/{entry}/retry"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = send_as_stranger(
        Method::DELETE,
        format!("/api/v2/scrobble-queue/uncertain/{entry}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // And it is still there afterwards: refused, not quietly consumed.
    let response = send(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 1);

    let response = send(
        Method::POST,
        format!("/api/v2/scrobble-queue/uncertain/{entry}/retry"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let retried = json_body(response).await;
    assert_ne!(
        retried["id"].as_str().unwrap(),
        entry,
        "a new entry, not the old one"
    );

    // Answered, so it stops asking.
    let response = send(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);

    // The copy the retry queued comes back ambiguous in its turn — the double
    // above answers that way every time — and *that* is the entry this discards.
    // Until here the delete route was only ever measured refusing a stranger,
    // which a handler that refused everybody would satisfy just as well.
    state.services.drain_scrobble_outbox().await.unwrap();
    let response = send(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    let waiting = json_body(response).await;
    assert_eq!(waiting.as_array().unwrap().len(), 1);
    let second = waiting[0]["id"].as_str().unwrap().to_owned();
    assert_ne!(second, entry, "the copy, not the listen it was copied from");

    let response = send(
        Method::DELETE,
        format!("/api/v2/scrobble-queue/uncertain/{second}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Thrown away rather than hidden from one listing: it is gone from the
    // queue, and discarding it again finds nothing to discard.
    let response = send(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);
    let response = send(
        Method::DELETE,
        format!("/api/v2/scrobble-queue/uncertain/{second}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Bookmarks and API tokens were reachable from one surface each: bookmarks
/// only from Subsonic, tokens only from a shell on the host.
#[tokio::test]
async fn native_bookmarks_and_api_tokens_round_trip() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let admin = state
        .db
        .create_account("token-admin", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("token-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            admin,
            "Tokens",
            &std::fs::canonicalize(&music).unwrap(),
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
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    let mut input = browse_input(600, "Long Read", "Chapters", "Narrator", Some(1), Some(1));
    input.relative_path = "token-0.flac".into();
    input.quick_hash = format!("{:064x}", 61_000);
    input.full_hash = format!("{:064x}", 62_000);
    state
        .db
        .apply_catalog_track(library, scan, &input, None, false)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let login = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "token-admin",
                "password": "correct horse battery staple",
                "device_name": "Integration"
            }),
        ))
        .await
        .unwrap();
    let access = json_body(login).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let track = state
        .services
        .catalog_snapshot(admin, &[])
        .await
        .unwrap()
        .songs[0]
        .id;

    let json_request = |method: Method, path: String, body: serde_json::Value| {
        let router = router.clone();
        let access = access.clone();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {access}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    let get = |path: String| {
        let router = router.clone();
        let access = access.clone();
        async move {
            router
                .oneshot(
                    Request::get(path)
                        .header("authorization", format!("Bearer {access}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };

    // Setting a bookmark twice moves it rather than adding a second, and the
    // comment is replaced rather than patched.
    let response = json_request(
        Method::PUT,
        format!("/api/v2/bookmarks/{track}"),
        serde_json::json!({"position_ms": 90_000, "comment": "chapter two"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = json_request(
        Method::PUT,
        format!("/api/v2/bookmarks/{track}"),
        serde_json::json!({"position_ms": 180_000}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let listed = json_body(get("/api/v2/bookmarks".into()).await).await;
    assert_eq!(listed.as_array().expect("a list").len(), 1);
    assert_eq!(listed[0]["position_ms"], 180_000);
    assert!(listed[0]["comment"].is_null(), "the comment was replaced");

    // A negative position is not a position.
    let response = json_request(
        Method::PUT,
        format!("/api/v2/bookmarks/{track}"),
        serde_json::json!({"position_ms": -1}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // The facade sees the same bookmark: one domain method, two surfaces.
    assert_eq!(
        state.services.bookmarks(admin).await.unwrap()[0].position_ms,
        180_000
    );

    // Deleting is idempotent, for the same reason it is on the facade.
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/v2/bookmarks/{track}"))
                    .header("authorization", format!("Bearer {access}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
    assert!(state.services.bookmarks(admin).await.unwrap().is_empty());

    // An API token can now be issued without a shell on the host. The secret
    // appears once and is never listed.
    let created = json_request(
        Method::POST,
        "/api/v2/admin/users/token-admin/tokens".into(),
        serde_json::json!({"name": "backup script", "scopes": ["catalog:read"]}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    let secret = created["secret"].as_str().expect("the secret is returned");
    assert!(secret.starts_with("wfapi_"));
    let token_id = created["id"].as_str().expect("the record carries its id");
    assert_eq!(created["scopes"][0], "catalog:read");

    let listed = json_body(get("/api/v2/admin/users/token-admin/tokens".into()).await).await;
    assert_eq!(listed.as_array().expect("a list").len(), 1);
    assert_eq!(listed[0]["name"], "backup script");
    assert!(
        listed[0].get("secret").is_none() && listed[0].get("token_hash").is_none(),
        "a listing must not carry the secret: {listed}"
    );
    assert!(listed[0]["revoked_at"].is_null());

    // The token authenticates, and stops doing so once revoked.
    let with_token = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(with_token.status(), StatusCode::OK);

    // Scopes are enforced, not decorated. The token names `catalog:read`, so it
    // reads the catalogue and nothing else, even though the account behind it
    // is an administrator. Storing a scope list, returning it from the API and
    // printing it from the CLI while ignoring it is worse than having none: the
    // operator believes the token is limited.
    let admin_route = router
        .clone()
        .oneshot(
            Request::get("/api/v2/admin/users")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_route.status(), StatusCode::FORBIDDEN);
    // The session it was issued from still reaches that route, so the refusal
    // belongs to the token and not to the account.
    assert_eq!(
        get("/api/v2/admin/users".into()).await.status(),
        StatusCode::OK
    );

    // Reading is all a `catalog:read` token may do. Before the scope check
    // reached the mutations it could still write playlists, shares, ratings,
    // the queue and these very bookmarks: only the administrative door was
    // closed, which shut the worst case and left the principle open.
    let write_attempt = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/api/v2/bookmarks/{track}"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"position_ms": 1}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(write_attempt.status(), StatusCode::FORBIDDEN);
    // And it still reads, so the refusal is the mutation and not the token.
    let read_attempt = router
        .clone()
        .oneshot(
            Request::get("/api/v2/bookmarks")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_attempt.status(), StatusCode::OK);

    // The media routes go through the same door now, and reading needs no
    // scope, so a read-only token still plays. Requiring a literal `read`
    // scope would strand every token an operator has already issued.
    let ticket = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v2/tracks/{track}/stream-ticket"))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ticket.status(), StatusCode::OK);
    // An unauthenticated one is still refused, so the door did not open.
    let anonymous = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v2/tracks/{track}/stream-ticket"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    // `write` admits the mutation and stops at the instance: the two levels
    // are separate, and `admin` implies `write` rather than the reverse.
    let writer = json_body(
        json_request(
            Method::POST,
            "/api/v2/admin/users/token-admin/tokens".into(),
            serde_json::json!({"name": "sync agent", "scopes": ["write"]}),
        )
        .await,
    )
    .await;
    let writer = writer["secret"].as_str().unwrap().to_owned();
    let allowed = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/api/v2/bookmarks/{track}"))
                .header("authorization", format!("Bearer {writer}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"position_ms": 1}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::NO_CONTENT);
    let refused = router
        .clone()
        .oneshot(
            Request::get("/api/v2/admin/users")
                .header("authorization", format!("Bearer {writer}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    state.services.delete_bookmark(admin, track).await.unwrap();

    // A scope list grants the union of its entries: naming `admin` beside
    // another scope admits these routes, because a token that explicitly
    // carries a permission must not be refused it. The stored form is
    // normalised, so what the listing shows is what authorization compares.
    let combined = json_body(
        json_request(
            Method::POST,
            "/api/v2/admin/users/token-admin/tokens".into(),
            serde_json::json!({"name": "ops", "scopes": ["  admin  ", "catalog:read"]}),
        )
        .await,
    )
    .await;
    assert_eq!(combined["scopes"][0], "admin", "scopes are stored trimmed");
    let combined = combined["secret"].as_str().unwrap().to_owned();
    let admitted = router
        .clone()
        .oneshot(
            Request::get("/api/v2/admin/users")
                .header("authorization", format!("Bearer {combined}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admitted.status(), StatusCode::OK);

    // A token issued without scopes is unrestricted, which is what the CLI has
    // always produced and what existing tokens carry.
    let unscoped = json_body(
        json_request(
            Method::POST,
            "/api/v2/admin/users/token-admin/tokens".into(),
            serde_json::json!({"name": "full access"}),
        )
        .await,
    )
    .await;
    let unscoped = unscoped["secret"].as_str().unwrap().to_owned();
    let allowed = router
        .clone()
        .oneshot(
            Request::get("/api/v2/admin/users")
                .header("authorization", format!("Bearer {unscoped}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);

    let revoked = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/api/v2/admin/users/token-admin/tokens/{token_id}"))
                .header("authorization", format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    let after = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);

    // Revoking it again is not found: it is already not working.
    let again = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/api/v2/admin/users/token-admin/tokens/{token_id}"))
                .header("authorization", format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);

    // Only an administrator mints one.
    let listener = state
        .db
        .create_account("token-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    assert!(matches!(
        state
            .services
            .create_api_token(listener, &[], "token-admin", "stolen", &[])
            .await,
        Err(ServiceError::Forbidden)
    ));
}

#[tokio::test]
async fn native_browse_endpoints_page_search_and_isolate_tenants() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("browse-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("browse-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("browse-music");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Browse",
            &std::fs::canonicalize(&music).unwrap(),
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
    // Tracks are applied out of sleeve order on purpose: the album drill-down
    // must sort them, not echo insertion order.
    for (index, (title, album, artist, track, disc)) in [
        (
            "Slow Tide",
            "Aurora Fields",
            "Lumen Drift",
            Some(2),
            Some(1),
        ),
        (
            "First Light",
            "Aurora Fields",
            "Lumen Drift",
            Some(1),
            Some(1),
        ),
        // Incomplete tags are common in real libraries and must not jump ahead.
        ("Hidden Track", "Aurora Fields", "Lumen Drift", None, None),
        (
            "Rivière Noire",
            "Nocturne Bleue",
            "Écho Solaire",
            Some(2),
            Some(1),
        ),
        (
            "Prélude",
            "Nocturne Bleue",
            "Écho Solaire",
            Some(1),
            Some(1),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = browse_input(index, title, album, artist, track, disc);
        match title {
            // The album still has an artist, but this track has no credited
            // artist and therefore no track_artist row.
            "Hidden Track" => {
                input.artist = None;
                input.album_artist = Some("Lumen Drift".into());
            }
            // Materializes positions 0 and 1 while preserving the existing
            // album identity. The public primary must remain Écho Solaire.
            "Prélude" => {
                input.artist = Some("Écho Solaire; Lumen Drift".into());
                input.album_artist = Some("Écho Solaire".into());
            }
            _ => {}
        }
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let router = waveflow_server::app(&config, state);
    let owner_token = login_token(&router, "browse-owner", password).await;
    let intruder_token = login_token(&router, "browse-intruder", password).await;

    let get = |uri: String, token: String| {
        let router = router.clone();
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

    // Albums are ordered by title, so Aurora Fields precedes Nocturne Bleue.
    let albums = get("/api/v2/albums".into(), owner_token.clone()).await;
    assert_eq!(albums.status(), StatusCode::OK);
    let albums = json_body(albums).await;
    let albums = albums.as_array().unwrap();
    assert_eq!(albums.len(), 2);
    assert_eq!(albums[0]["title"], "Aurora Fields");
    assert_eq!(albums[1]["title"], "Nocturne Bleue");

    // Paging is applied in SQL, not after the fact.
    let page = get(
        "/api/v2/albums?limit=1&offset=1".into(),
        owner_token.clone(),
    )
    .await;
    let page = json_body(page).await;
    assert_eq!(page.as_array().unwrap().len(), 1);
    assert_eq!(page[0]["title"], "Nocturne Bleue");

    // The paging ceiling matches the Subsonic contract's 500-item cap.
    let rejected = get("/api/v2/albums?limit=501".into(), owner_token.clone()).await;
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let album_id = albums[0]["id"].as_str().unwrap().to_owned();
    let detail = get(format!("/api/v2/albums/{album_id}"), owner_token.clone()).await;
    assert_eq!(detail.status(), StatusCode::OK);
    let detail = json_body(detail).await;
    assert_eq!(detail["title"], "Aurora Fields");
    let album_artist_id = detail["artist_id"]
        .as_str()
        .expect("the album artist has a public id")
        .to_owned();
    let songs = detail["songs"].as_array().unwrap();
    assert_eq!(songs.len(), 3);
    assert_eq!(songs[0]["artist_id"], album_artist_id);
    assert_eq!(songs[1]["artist_id"], album_artist_id);
    assert!(songs[2]["artist_id"].is_null());
    assert_eq!(songs[0]["title"], "First Light", "sleeve order wins");
    assert_eq!(songs[1]["title"], "Slow Tide");
    assert_eq!(
        songs[2]["title"], "Hidden Track",
        "an untagged track sorts last, not ahead of track 1"
    );

    let artists = get("/api/v2/artists".into(), owner_token.clone()).await;
    let artists = json_body(artists).await;
    let artists = artists.as_array().unwrap();
    assert_eq!(artists.len(), 2);
    let echo = artists
        .iter()
        .find(|artist| artist["name"] == "Écho Solaire")
        .expect("accented artist is listed");
    assert_eq!(echo["album_count"], 1);

    let artist_id = echo["id"].as_str().unwrap().to_owned();
    let detail = get(format!("/api/v2/artists/{artist_id}"), owner_token.clone()).await;
    let detail = json_body(detail).await;
    assert_eq!(detail["name"], "Écho Solaire");
    assert_eq!(detail["albums"].as_array().unwrap().len(), 1);
    assert_eq!(detail["albums"][0]["title"], "Nocturne Bleue");

    // The same albums on their own, without the page around them: what an
    // album page reads to list the artist's other records.
    let albums_of = get(
        format!("/api/v2/artists/{artist_id}/albums"),
        owner_token.clone(),
    )
    .await;
    assert_eq!(albums_of.status(), StatusCode::OK);
    let albums_of = json_body(albums_of).await;
    assert_eq!(
        albums_of, detail["albums"],
        "the page's albums, one for one"
    );

    // FTS5 folds diacritics, so an unaccented query still reaches "Écho Solaire".
    let found = get("/api/v2/search?q=echo".into(), owner_token.clone()).await;
    assert_eq!(found.status(), StatusCode::OK);
    let found = json_body(found).await;
    // An artist answers for their own name and nobody else's. "Lumen Drift"
    // shares a track with "Écho Solaire" and is credited on the songs this
    // query returns, which is exactly what used to put it here: the artist
    // half of a search was derived from the matching tracks rather than
    // matched, so searching one name returned everyone who had played with
    // its owner.
    let names = found["artists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|artist| artist["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Écho Solaire"]);
    assert_eq!(found["albums"].as_array().unwrap().len(), 1);
    assert_eq!(found["albums"][0]["title"], "Nocturne Bleue");
    assert_eq!(found["songs"].as_array().unwrap().len(), 2);
    let found_artist_id = found["artists"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artist| artist["name"] == "Écho Solaire")
        .expect("the primary artist is included in search results")["id"]
        .as_str()
        .unwrap();
    for title in ["Prélude", "Rivière Noire"] {
        let song = found["songs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|song| song["title"] == title)
            .unwrap_or_else(|| panic!("missing search result {title}"));
        assert_eq!(song["artist_id"], found_artist_id, "{title}");
    }

    // The library projection uses TrackRecord rather than SongItem, but keeps
    // the same artist link so a client can open an artist from every track list.
    let tracks = get(
        format!("/api/v2/libraries/{library_id}/tracks"),
        owner_token.clone(),
    )
    .await;
    let tracks = json_body(tracks).await;
    assert_eq!(tracks.as_array().unwrap().len(), 5);
    let track = |title: &str| {
        tracks
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["title"] == title)
            .unwrap_or_else(|| panic!("missing library track {title}"))
    };
    assert_eq!(track("First Light")["artist_id"], album_artist_id);
    assert_eq!(track("Slow Tide")["artist_id"], album_artist_id);
    assert!(track("Hidden Track")["artist_id"].is_null());
    assert_eq!(track("Prélude")["artist_id"], artist_id);
    assert_eq!(track("Rivière Noire")["artist_id"], artist_id);

    // One track carries where its file sits, relative to the library, as the
    // library's own listing spells it; the album's song rows do not, so a
    // listing is not a path per row nobody browsing albums asked for.
    let prelude = track("Prélude");
    let single = get(
        format!("/api/v2/tracks/{}", prelude["id"].as_str().unwrap()),
        owner_token.clone(),
    )
    .await;
    assert_eq!(single.status(), StatusCode::OK);
    let single = json_body(single).await;
    assert_eq!(single["title"], "Prélude");
    assert_eq!(single["relative_path"], prelude["relative_path"]);
    let path = single["relative_path"].as_str().unwrap();
    assert!(!path.is_empty() && !path.contains('\\'), "{path}");
    assert!(
        !std::path::Path::new(path).is_absolute(),
        "the library root never leaves the server: {path}"
    );
    let album_songs = get(format!("/api/v2/albums/{album_id}"), owner_token.clone()).await;
    let album_songs = json_body(album_songs).await;
    assert!(album_songs["songs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|song| song.get("relative_path").is_none()));

    // Search-as-you-type: the trailing term matches as a prefix. This exact
    // case was reported from the Android client — "echo" returned songs,
    // albums and artists while "ech" returned nothing, because the native
    // surface still required whole tokens after the Subsonic one moved on.
    let partial = get("/api/v2/search?q=ech".into(), owner_token.clone()).await;
    assert_eq!(partial.status(), StatusCode::OK);
    let partial = json_body(partial).await;
    assert_eq!(partial["songs"].as_array().unwrap().len(), 2);
    assert_eq!(partial["albums"].as_array().unwrap().len(), 1);
    assert_eq!(partial["artists"].as_array().unwrap().len(), 1);

    // Extra terms still narrow rather than widen.
    let narrowed = get(
        "/api/v2/search?q=echo%20nonexistent".into(),
        owner_token.clone(),
    )
    .await;
    assert!(json_body(narrowed).await["songs"]
        .as_array()
        .unwrap()
        .is_empty());

    // A search with no usable term is an empty result, never a SQL error.
    let blank = get("/api/v2/search?q=%20".into(), owner_token.clone()).await;
    assert_eq!(blank.status(), StatusCode::OK);
    let blank = json_body(blank).await;
    assert!(blank["songs"].as_array().unwrap().is_empty());

    // A foreign tenant sees an empty catalogue and cannot probe ids.
    let foreign = get("/api/v2/albums".into(), intruder_token.clone()).await;
    assert_eq!(foreign.status(), StatusCode::OK);
    assert!(json_body(foreign).await.as_array().unwrap().is_empty());
    for uri in [
        format!("/api/v2/albums/{album_id}"),
        format!("/api/v2/artists/{artist_id}"),
        format!("/api/v2/artists/{artist_id}/albums"),
        format!("/api/v2/artists/{}/albums", uuid::Uuid::new_v4()),
    ] {
        let response = get(uri, intruder_token.clone()).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "foreign ids must not be distinguishable from missing ones"
        );
    }
    let foreign_search = get("/api/v2/search?q=echo".into(), intruder_token).await;
    let foreign_search = json_body(foreign_search).await;
    assert!(foreign_search["artists"].as_array().unwrap().is_empty());
    assert!(foreign_search["songs"].as_array().unwrap().is_empty());

    // Anonymous access is rejected before any catalogue work happens.
    let anonymous = router
        .clone()
        .oneshot(Request::get("/api/v2/albums").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn native_user_data_endpoints_round_trip_and_isolate_tenants() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("data-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("data-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("data-music");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "User data",
            &std::fs::canonicalize(&music).unwrap(),
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
    state.db.start_scan_job(scan_id, 2, false).await.unwrap();
    for (index, (title, artist)) in [
        ("First Light", "Lumen Drift"),
        ("Slow Tide", "Écho Solaire"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = browse_input(
            index,
            title,
            "Aurora Fields",
            artist,
            Some(index as i64 + 1),
            Some(1),
        );
        input.album_artist = Some("Lumen Drift".into());
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "data-owner", password).await;
    let intruder_token = login_token(&router, "data-intruder", password).await;

    let send =
        |method: &'static str, uri: String, token: String, body: Option<serde_json::Value>| {
            let router = router.clone();
            async move {
                let request = Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("authorization", format!("Bearer {token}"));
                let request = match body {
                    Some(body) => request
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                    None => request.body(Body::empty()).unwrap(),
                };
                router.oneshot(request).await.unwrap()
            }
        };

    // Collect the track ids through the native browse surface.
    let albums = send("GET", "/api/v2/albums".into(), owner_token.clone(), None).await;
    let albums = json_body(albums).await;
    let album_id = albums[0]["id"].as_str().unwrap().to_owned();
    let detail = send(
        "GET",
        format!("/api/v2/albums/{album_id}"),
        owner_token.clone(),
        None,
    )
    .await;
    let detail = json_body(detail).await;
    let first = detail["songs"][0]["id"].as_str().unwrap().to_owned();
    let second = detail["songs"][1]["id"].as_str().unwrap().to_owned();
    let first_artist_id = detail["songs"][0]["artist_id"]
        .as_str()
        .expect("the fixture track has an artist")
        .to_owned();
    let second_artist_id = detail["songs"][1]["artist_id"]
        .as_str()
        .expect("the second fixture track has an artist")
        .to_owned();
    assert_ne!(first_artist_id, second_artist_id);

    // Individual tracks can be resolved for favorites and queue hydration,
    // while the same public id remains opaque to another tenant.
    let track = send(
        "GET",
        format!("/api/v2/tracks/{first}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(track.status(), StatusCode::OK);
    let track = json_body(track).await;
    assert_eq!(track["id"], first);
    assert_eq!(track["artist_id"], first_artist_id);
    let foreign_track = send(
        "GET",
        format!("/api/v2/tracks/{first}"),
        intruder_token.clone(),
        None,
    )
    .await;
    assert_eq!(foreign_track.status(), StatusCode::NOT_FOUND);

    // Playlists: create, read back, mutate, then delete.
    let created = send(
        "POST",
        "/api/v2/playlists".into(),
        owner_token.clone(),
        Some(serde_json::json!({ "name": "Evening", "track_ids": [first] })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    let playlist_id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["songs"].as_array().unwrap().len(), 1);
    assert_eq!(created["songs"][0]["id"], first);
    assert_eq!(created["songs"][0]["artist_id"], first_artist_id);

    let listed = send("GET", "/api/v2/playlists".into(), owner_token.clone(), None).await;
    assert_eq!(json_body(listed).await.as_array().unwrap().len(), 1);

    let updated = send(
        "PATCH",
        format!("/api/v2/playlists/{playlist_id}"),
        owner_token.clone(),
        Some(serde_json::json!({ "comment": "late night", "add": [second] })),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let updated = json_body(updated).await;
    assert_eq!(updated["comment"], "late night");
    assert_eq!(updated["songs"].as_array().unwrap().len(), 2);
    assert_eq!(updated["songs"][0]["id"], first);
    assert_eq!(updated["songs"][0]["artist_id"], first_artist_id);
    assert_eq!(updated["songs"][1]["id"], second);
    assert_eq!(updated["songs"][1]["artist_id"], second_artist_id);

    // Favorites round-trip through the dedicated collection.
    let starred = send(
        "PUT",
        format!("/api/v2/favorites/track/{first}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(starred.status(), StatusCode::NO_CONTENT);
    let favorites = send("GET", "/api/v2/favorites".into(), owner_token.clone(), None).await;
    let favorites = json_body(favorites).await;
    assert_eq!(favorites.as_array().unwrap().len(), 1);
    assert_eq!(favorites[0]["entity_type"], "track");
    assert_eq!(favorites[0]["entity_id"], first);

    let unstarred = send(
        "DELETE",
        format!("/api/v2/favorites/track/{first}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(unstarred.status(), StatusCode::NO_CONTENT);
    let favorites = send("GET", "/api/v2/favorites".into(), owner_token.clone(), None).await;
    assert!(json_body(favorites).await.as_array().unwrap().is_empty());

    // Ratings are read back through the browse surface, not inferred.
    let rated = send(
        "PUT",
        format!("/api/v2/ratings/track/{first}"),
        owner_token.clone(),
        Some(serde_json::json!({ "rating": 4 })),
    )
    .await;
    assert_eq!(rated.status(), StatusCode::NO_CONTENT);
    let detail = send(
        "GET",
        format!("/api/v2/albums/{album_id}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(json_body(detail).await["songs"][0]["user_rating"], 4);

    // Out-of-range ratings and unknown entity kinds are refused, not stored.
    let invalid = send(
        "PUT",
        format!("/api/v2/ratings/track/{first}"),
        owner_token.clone(),
        Some(serde_json::json!({ "rating": 6 })),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let unknown_kind = send(
        "PUT",
        format!("/api/v2/favorites/banana/{first}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(unknown_kind.status(), StatusCode::UNPROCESSABLE_ENTITY);

    for invalid_time in [-1, now_ms().saturating_add(10 * 60 * 1_000)] {
        let invalid = send(
            "POST",
            "/api/v2/scrobbles".into(),
            owner_token.clone(),
            Some(serde_json::json!({
                "track_id": first,
                "submission": true,
                "played_at": invalid_time
            })),
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    let scrobbled = send(
        "POST",
        "/api/v2/scrobbles".into(),
        owner_token.clone(),
        Some(serde_json::json!({ "track_id": first, "submission": true })),
    )
    .await;
    assert_eq!(scrobbled.status(), StatusCode::NO_CONTENT);
    for invalid_limit in [-1, MAX_HISTORY_LIMIT + 1] {
        assert!(matches!(
            state.services.history(owner, invalid_limit).await,
            Err(ServiceError::Invalid)
        ));
    }

    // The queue survives a write/read round-trip.
    let saved = send(
        "PUT",
        "/api/v2/queue".into(),
        owner_token.clone(),
        Some(serde_json::json!({
            "track_ids": [first, second],
            "current": first,
            "position_ms": 4200,
            "client": "test"
        })),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::NO_CONTENT);
    let queue = send("GET", "/api/v2/queue".into(), owner_token.clone(), None).await;
    let queue = json_body(queue).await;
    assert_eq!(queue["position_ms"], 4200);
    assert_eq!(queue["current"], first);
    assert_eq!(queue["songs"].as_array().unwrap().len(), 2);

    // Multiple shares retain their independent track ordering when the
    // aggregate loader batches all share rows.
    for track_ids in [vec![second.clone(), first.clone()], vec![first.clone()]] {
        let share = send(
            "POST",
            "/api/v2/shares".into(),
            owner_token.clone(),
            Some(serde_json::json!({ "track_ids": track_ids })),
        )
        .await;
        assert_eq!(share.status(), StatusCode::CREATED);
        assert!(json_body(share).await["url"].as_str().is_some());
    }
    let shares = send("GET", "/api/v2/shares".into(), owner_token.clone(), None).await;
    let shares = json_body(shares).await;
    assert!(shares
        .as_array()
        .unwrap()
        .iter()
        .all(|share| share.get("url").is_none()));
    let song_orders = shares
        .as_array()
        .unwrap()
        .iter()
        .map(|share| {
            share["track_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|track_id| track_id.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(song_orders.contains(&vec![second.to_string(), first.to_string()]));
    assert!(song_orders.contains(&vec![first.to_string()]));
    let share_columns =
        sqlx::query_scalar::<_, String>("SELECT name FROM pragma_table_info('share')")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    assert!(share_columns.contains(&"token_hash".to_owned()));
    assert!(!share_columns.contains(&"token_nonce".to_owned()));
    assert!(!share_columns.contains(&"token_ciphertext".to_owned()));

    // An expiry set by mistake must be liftable. COALESCE alone made it
    // permanent: omitting the field and sending null were the same bind, so the
    // owner's only recourse was deleting the share and publishing a new URL.
    let expiring = send(
        "POST",
        "/api/v2/shares".into(),
        owner_token.clone(),
        Some(serde_json::json!({
            "track_ids": [first.clone()],
            "expires_at": now_ms() + 3_600_000
        })),
    )
    .await;
    let expiring_id = json_body(expiring).await["id"].as_str().unwrap().to_owned();
    let patched = send(
        "PATCH",
        format!("/api/v2/shares/{expiring_id}"),
        owner_token.clone(),
        Some(serde_json::json!({ "description": "kept" })),
    )
    .await;
    // Omitting the field still leaves it alone — clearing never fires by accident.
    assert!(json_body(patched).await["expires_at"].is_i64());
    let cleared = send(
        "PATCH",
        format!("/api/v2/shares/{expiring_id}"),
        owner_token.clone(),
        Some(serde_json::json!({ "clear": ["expires_at"] })),
    )
    .await;
    let cleared = json_body(cleared).await;
    assert!(cleared["expires_at"].is_null(), "expiry should be liftable");
    assert_eq!(cleared["description"], "kept", "clearing is per field");
    // An unknown name is refused rather than silently ignored, so a client
    // sending `expiresAt` learns it did nothing.
    let typo = send(
        "PATCH",
        format!("/api/v2/shares/{expiring_id}"),
        owner_token.clone(),
        Some(serde_json::json!({ "clear": ["expiresAt"] })),
    )
    .await;
    assert_eq!(typo.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // A foreign tenant can neither read nor mutate any of it.
    let foreign_playlists = send(
        "GET",
        "/api/v2/playlists".into(),
        intruder_token.clone(),
        None,
    )
    .await;
    assert!(json_body(foreign_playlists)
        .await
        .as_array()
        .unwrap()
        .is_empty());
    for (method, uri) in [
        ("GET", format!("/api/v2/playlists/{playlist_id}")),
        ("DELETE", format!("/api/v2/playlists/{playlist_id}")),
        ("PUT", format!("/api/v2/favorites/track/{first}")),
    ] {
        let response = send(method, uri, intruder_token.clone(), None).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{method} must not reach another tenant's data"
        );
    }
    let foreign_queue = send("GET", "/api/v2/queue".into(), intruder_token.clone(), None).await;
    assert_eq!(foreign_queue.status(), StatusCode::OK);
    assert!(json_body(foreign_queue).await.is_null());

    // Deleting the playlist makes it unreachable for its owner too.
    let deleted = send(
        "DELETE",
        format!("/api/v2/playlists/{playlist_id}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let gone = send(
        "GET",
        format!("/api/v2/playlists/{playlist_id}"),
        owner_token,
        None,
    )
    .await;
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

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

/// A correction outlives the scan that would have erased it.
///
/// Writing tags had two obvious routes and both were wrong: rewriting the file
/// breaks the invariant that makes a scan safe, and writing the track row is
/// undone by the next scan's `title=excluded.title`. The correction therefore
/// lives beside the row, and this asserts the three things that buys — it
/// shows, it survives, and the file's hash never moves.
#[tokio::test]
async fn a_track_correction_survives_the_scan_that_would_have_erased_it() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("tagger", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let listener = state
        .db
        .create_account("tag-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("tag-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Mispelled Titel.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Tag library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Tag library".into(),
        root_path: root,
    };
    run_scan(&state, owner, library.clone()).await;

    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    let track_id = tracks[0].id;
    let scanned_hash = tracks[0].full_hash.clone();
    assert_eq!(tracks[0].title, "Mispelled Titel");

    let token = login_token(&router, "tagger", password).await;
    let patch = |token: String, body: serde_json::Value| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(
                    Request::patch(format!("/api/v2/tracks/{track_id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            (response.status(), json_body(response).await)
        }
    };

    let (status, corrected) = patch(
        token.clone(),
        serde_json::json!({ "title": "Misspelled Title", "year": 1998 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(corrected["title"], "Misspelled Title");
    assert_eq!(corrected["year"], 1998);
    // The file was not touched, which is what keeps a content-based link valid
    // across an edit.
    assert_eq!(corrected["full_hash"], scanned_hash);

    // The track answers the effective value and says nothing about where it
    // came from, so an editor cannot tell a corrected field from a scanned one
    // — nor offer to restore what it cannot read. That is what the overrides
    // route is for, and it is the read half of correcting a tag.
    let overrides = {
        let response = router
            .clone()
            .oneshot(
                Request::get(format!("/api/v2/tracks/{track_id}/overrides"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await
    };
    // The scanned column is untouched by the correction: the projection merges
    // the two with COALESCE rather than writing over the file's value.
    assert_eq!(overrides["source"]["title"], "Mispelled Titel");
    assert_eq!(overrides["overrides"]["title"], "Misspelled Title");
    assert_eq!(overrides["overrides"]["year"], 1998);
    // A field nobody corrected is null, which is what its column holds.
    assert!(overrides["overrides"]["comment"].is_null());
    assert!(overrides["overrides"]["artists"].is_null());
    // Artists and genres are deliberately absent from `source`: correcting
    // either rewrites `track_participant` and the display string, so once a
    // correction exists the file's own credits are no longer in the database.
    assert!(overrides["source"].get("artists").is_none());

    // The headline. A scan applies `title=excluded.title` over the track row,
    // so a correction stored there would be gone by now.
    //
    // Forced full, because nothing about the file changed and an ordinary scan
    // would take the skip path — proving only that the apply never ran.
    state.db.request_full_scan_everywhere().await.unwrap();
    run_scan(&state, owner, library).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks[0].id, track_id);
    assert_eq!(tracks[0].full_hash, scanned_hash, "the file never moved");
    let after_scan = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(after_scan[0].title, "Misspelled Title");
    assert_eq!(after_scan[0].year, Some(1998));

    // The index holds a copy of the title, and a corrected track that still
    // answered to the old spelling would be corrected only to the eye.
    let found = state
        .services
        .catalog_search(
            owner,
            &[],
            "Misspelled",
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        found.songs.iter().map(|song| song.id).collect::<Vec<_>>(),
        vec![track_id],
        "the search index follows the correction"
    );

    // It is a library fact, so it lands on the library feed rather than in one
    // account's journal.
    let feed = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v2/libraries/{library_id}/events?after=0&limit=500"
            ))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let feed = json_body(feed).await;
    assert!(
        feed["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["entity_id"] == track_id.to_string() && event["action"] == "upsert"),
        "the correction is announced on the feed"
    );

    // A listener may read the library and may not rewrite what everyone else
    // reads. Blurred onto 404, like every refusal that would otherwise confirm
    // what a caller cannot reach.
    sqlx::query(
        "INSERT INTO library_member (library_id, user_id, role, created_at) \
         VALUES (?, ?, 'listener', ?)",
    )
    .bind(library_id.to_string())
    .bind(listener.to_string())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let listener_token = login_token(&router, "tag-listener", password).await;
    let (status, _) = patch(
        listener_token.clone(),
        serde_json::json!({ "title": "Nope" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // An empty patch writes nothing, and it is still a request to correct this
    // track: a listener is refused it the same way, or `{}` would answer
    // differently from every other patch they send.
    let (status, _) = patch(listener_token.clone(), serde_json::json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // And a manager may, which is the half of the rule the refusal above cannot
    // show: were it `Owner` alone, everything asserted so far would still pass.
    let manager = state
        .db
        .create_account("tag-manager", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO library_member (library_id, user_id, role, created_at) \
         VALUES (?, ?, 'manager', ?)",
    )
    .bind(library_id.to_string())
    .bind(manager.to_string())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let manager_token = login_token(&router, "tag-manager", password).await;
    let (status, by_manager) = patch(
        manager_token,
        serde_json::json!({ "title": "Corrected By Manager" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(by_manager["title"], "Corrected By Manager");
    // #177. The manager mentioned the title and nothing else, so the year the
    // owner corrected is still there. Wholesale, it would have been dropped.
    assert_eq!(by_manager["year"], 1998);

    // An empty body mentions nothing, so it changes nothing.
    let (status, untouched) = patch(token.clone(), serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(untouched["title"], "Corrected By Manager");
    assert_eq!(untouched["year"], 1998);

    // `null` is what removes a correction, and the file's own spelling comes
    // back.
    let (status, cleared) = patch(token, serde_json::json!({ "title": null, "year": null })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cleared["title"], "Mispelled Titel");
    assert!(cleared["year"].is_null());
}

/// A corrected artist agrees with itself everywhere it is written down.
///
/// This is what makes artists and genres different from a title: they are also
/// rows in `track_participant` and `track_genre`, rebuilt by every scan from
/// the file. Correcting the display string alone would have a track answer
/// `artist` and `artists[]` differently, and stay findable under the name it
/// was corrected away from.
#[tokio::test]
async fn a_corrected_artist_agrees_with_itself_everywhere() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("credits-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("credit-fix-music");
    std::fs::create_dir_all(&music).unwrap();
    // Tagged by FFmpeg, so the track starts with real credits and real genres
    // to be corrected away from.
    generate_audio_fixture(&music.join("Mistagged.flac"), "flac", "flac");
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Credit fix library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Credit fix library".into(),
        root_path: root,
    };
    run_scan(&state, owner, library.clone()).await;
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();

    let track_id = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()[0]
        .id;
    let before = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(
        before[0]
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha", "Beta"],
        "the file credits two artists to begin with"
    );

    let token = login_token(&router, "credits-owner", password).await;
    let response = router
        .clone()
        .oneshot(
            Request::patch(format!("/api/v2/tracks/{track_id}"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "artists": ["Corrected Performer"],
                        "genres": ["Corrected Genre"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Everything the track says about itself, checked together — the display
    // string, the structured list, and the index.
    let agrees = |songs: Vec<waveflow_server::services::SongItem>, when: &str| {
        let song = &songs[0];
        assert_eq!(
            song.artist.as_deref(),
            Some("Corrected Performer"),
            "{when}"
        );
        assert_eq!(
            song.artists
                .iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Corrected Performer"],
            "{when}: the structured list has to agree with the display string"
        );
        assert_eq!(song.genres, vec!["Corrected Genre".to_owned()], "{when}");
    };
    agrees(
        state
            .services
            .songs_by_ids(owner, &[track_id])
            .await
            .unwrap(),
        "right after the correction",
    );

    let found = state
        .services
        .catalog_search(
            owner,
            &[],
            "Corrected",
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        found.songs.iter().map(|song| song.id).collect::<Vec<_>>(),
        vec![track_id],
        "a corrected credit has to be findable under its new name"
    );

    // And the scan that rebuilds both relations from the file has to leave them
    // corrected, which is the whole reason it consults the override.
    //
    // A full scan, and not an ordinary one: nothing about the file changed, so
    // an ordinary scan takes the skip path and never re-applies the track at
    // all. Asserting against that would prove only that the apply did not run.
    state.db.request_full_scan_everywhere().await.unwrap();
    run_scan(&state, owner, library).await;
    agrees(
        state
            .services
            .songs_by_ids(owner, &[track_id])
            .await
            .unwrap(),
        "after a rescan rebuilt the relations from the file",
    );

    // A search for the name it was corrected away from must come back empty.
    // Asserting only that the new name is findable would pass while the index
    // still carried the old one alongside it.
    let stale = state
        .services
        .catalog_search(
            owner,
            &[],
            "Alpha",
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert!(
        stale.songs.is_empty(),
        "the corrected track must not answer to its old credit: {:?}",
        stale.songs.iter().map(|song| song.id).collect::<Vec<_>>()
    );

    // Dropping one list with `null` while keeping the other. Treating the pair
    // together would leave the dropped one corrected — the hole a single check
    // over both lists had, seen one field at a time.
    let patch_body = |body: serde_json::Value| {
        let router = router.clone();
        let token = token.clone();
        async move {
            let response = router
                .oneshot(
                    Request::patch(format!("/api/v2/tracks/{track_id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
    };
    let names = |song: &waveflow_server::services::SongItem| {
        song.artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<_>>()
    };

    patch_body(serde_json::json!({ "artists": ["Solo"], "genres": ["Ambient"] })).await;
    // `null` removes a list correction. Leaving the field out would keep it.
    patch_body(serde_json::json!({ "artists": ["Solo"], "genres": null })).await;
    let one_dropped = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(names(&one_dropped[0]), vec!["Solo".to_owned()], "kept");
    assert_eq!(
        one_dropped[0].genres,
        vec!["Electronic".to_owned(), "Test".to_owned()],
        "the dropped list goes back to the file while the kept one stays"
    );

    patch_body(serde_json::json!({ "genres": ["Ambient"], "artists": null })).await;
    let other_dropped = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(
        names(&other_dropped[0]),
        vec!["Alpha".to_owned(), "Beta".to_owned()],
        "and the same the other way round"
    );
    assert_eq!(other_dropped[0].genres, vec!["Ambient".to_owned()]);

    // Removing the correction hands the track back to its file. The rows the
    // correction wrote replaced what the tags said, so this can only be right
    // by re-reading them — and it must not wait for a scan. Removed by an
    // explicit `null` on each list: `{}` leaves every correction where it is.
    let cleared = router
        .oneshot(
            Request::patch(format!("/api/v2/tracks/{track_id}"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"artists":null,"genres":null}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cleared.status(), StatusCode::OK);
    let restored = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(
        restored[0]
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha", "Beta"],
        "clearing a correction restores the file's own credits"
    );
    assert_eq!(restored[0].artist.as_deref(), Some("Alpha; Beta"));
    assert_eq!(
        restored[0].genres,
        vec!["Electronic".to_owned(), "Test".to_owned()],
        "and its own genres"
    );
}

/// A correction can be removed from a track that never came from a scan.
///
/// Dropping a list correction hands the track back to its file, which the apply
/// re-reads — and the apply used to demand the scan that had last seen the
/// track. A received file has no such scan until one happens to walk past it,
/// so that demand would have answered 503 to every correction made on a file
/// before its first scan.
#[tokio::test]
async fn a_correction_clears_on_a_track_that_never_came_from_a_scan() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("unscanned-tagger", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("unscanned-tag-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Unscanned.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Unscanned library",
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
            name: "Unscanned library".into(),
            root_path: root,
        },
    )
    .await;
    let track_id = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()[0]
        .id;

    // The state a received file is written in: on disk, in the catalogue, and
    // never walked over by a scan.
    sqlx::query("UPDATE track SET last_seen_scan_id=NULL WHERE id=?")
        .bind(track_id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();

    let token = login_token(&router, "unscanned-tagger", password).await;
    let patch = |body: serde_json::Value| {
        let router = router.clone();
        let token = token.clone();
        async move {
            router
                .oneshot(
                    Request::patch(format!("/api/v2/tracks/{track_id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };

    assert_eq!(
        patch(serde_json::json!({ "artists": ["Corrected Name"] }))
            .await
            .status(),
        StatusCode::OK
    );
    // The drop: this is the path that re-reads the file, and the one that used
    // to need a scan id.
    let dropped = patch(serde_json::json!({ "artists": null })).await;
    assert_eq!(
        dropped.status(),
        StatusCode::OK,
        "removing a correction must not require a scan the track never had"
    );

    let restored = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert!(
        restored[0]
            .artists
            .iter()
            .all(|artist| artist.name != "Corrected Name"),
        "the track went back to what its file says"
    );
    let stamp: Option<String> =
        sqlx::query_scalar("SELECT last_seen_scan_id FROM track WHERE id=?")
            .bind(track_id.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert!(
        stamp.is_none(),
        "and re-deriving its tags did not invent a scan for it"
    );
}

/// A correction leaves alone every field it does not mention.
///
/// #177. The patch used to replace the whole set of corrections, so one client
/// correcting a title silently erased a comment another had corrected. Three
/// states are what fix it: a field left out keeps its correction, `null`
/// removes it, a value sets it.
///
/// The desktop's request is replayed as `drain.rs` sends it, because that client
/// is why no version header was needed: it spells its six fields out, `null`
/// included, so those have to keep meaning what they meant — while the three it
/// has no input for now survive it.
#[tokio::test]
async fn a_correction_leaves_alone_what_it_does_not_mention() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("partial-tagger", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("partial-tag-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Partial.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Partial library",
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
            name: "Partial library".into(),
            root_path: root,
        },
    )
    .await;
    let track_id = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()[0]
        .id;

    let token = login_token(&router, "partial-tagger", password).await;
    let patch = |body: serde_json::Value| {
        let router = router.clone();
        let token = token.clone();
        async move {
            router
                .oneshot(
                    Request::patch(format!("/api/v2/tracks/{track_id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
    };
    // The row as it stands, not the merged track: a correction and the file's
    // value look identical in the catalogue, and this test is about which is
    // which.
    let stored = || {
        let state = state.clone();
        async move {
            serde_json::to_value(
                state
                    .services
                    .track_overrides(owner, track_id)
                    .await
                    .unwrap()
                    .overrides,
            )
            .unwrap()
        }
    };
    let identifier = "8f3c7d52-5b0a-4d5e-9c1e-2a6b4f0d1e3a";

    // Another client corrects the three fields the desktop has no input for.
    assert_eq!(
        patch(serde_json::json!({
            "sort_title": "Partial, The",
            "comment": "Remastered",
            "musicbrainz_recording_id": identifier,
        }))
        .await,
        StatusCode::OK
    );

    // Then the desktop corrects the title and the year, as it sends them: every
    // field its editor has, the empty ones as an explicit `null`.
    assert_eq!(
        patch(serde_json::json!({
            "title": "Partial (Live)",
            "artists": null,
            "genres": null,
            "year": 2001,
            "track_number": null,
            "disc_number": null,
        }))
        .await,
        StatusCode::OK
    );
    let after_desktop = stored().await;
    assert_eq!(after_desktop["title"], "Partial (Live)");
    assert_eq!(after_desktop["year"], 2001);
    assert_eq!(
        after_desktop["comment"], "Remastered",
        "another client's correction survives a patch that never mentioned it"
    );
    assert_eq!(after_desktop["sort_title"], "Partial, The");
    assert_eq!(after_desktop["musicbrainz_recording_id"], identifier);

    // `{}` mentions nothing, so it changes nothing: not the correction, not the
    // row, and not the library feed, where an `upsert` for a change that never
    // happened would send every client to refetch the track.
    let events = || {
        let state = state.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM library_event WHERE library_id=?")
                .bind(library_id.to_string())
                .fetch_one(state.db.pool())
                .await
                .unwrap()
        }
    };
    // A sentinel rather than the stamp the last write left: two writes can land
    // in the same millisecond, and a comparison with that stamp would then pass
    // whether or not the row was rewritten.
    sqlx::query("UPDATE track_override SET updated_at=1 WHERE track_id=?")
        .bind(track_id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let events_before = events().await;
    assert_eq!(patch(serde_json::json!({})).await, StatusCode::OK);
    assert_eq!(stored().await, after_desktop, "an empty patch is a no-op");
    assert_eq!(events().await, events_before, "and announces nothing");
    let stamp: i64 = sqlx::query_scalar("SELECT updated_at FROM track_override WHERE track_id=?")
        .bind(track_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(stamp, 1, "and writes nothing");

    // `null` removes one correction, and only that one.
    assert_eq!(
        patch(serde_json::json!({ "comment": null })).await,
        StatusCode::OK
    );
    let without_comment = stored().await;
    assert!(without_comment["comment"].is_null());
    assert_eq!(without_comment["title"], "Partial (Live)");
    assert_eq!(without_comment["sort_title"], "Partial, The");

    // A blank string reads as `null`: a removal, never a stored blank.
    assert_eq!(
        patch(serde_json::json!({ "sort_title": "   " })).await,
        StatusCode::OK
    );
    let without_sort_title = stored().await;
    assert!(without_sort_title["sort_title"].is_null());
    assert_eq!(without_sort_title["title"], "Partial (Live)");

    // A value outside what is allowed refuses the whole patch before it writes
    // anything, the value beside it included.
    assert_eq!(
        patch(serde_json::json!({ "title": "Never Written", "year": 0 })).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(stored().await["title"], "Partial (Live)");

    // The desktop emptying its whole form removes its own corrections, and
    // still leaves the identifier it cannot see.
    assert_eq!(
        patch(serde_json::json!({
            "title": null,
            "artists": null,
            "genres": null,
            "year": null,
            "track_number": null,
            "disc_number": null,
        }))
        .await,
        StatusCode::OK
    );
    let cleared_by_desktop = stored().await;
    assert!(cleared_by_desktop["title"].is_null());
    assert!(cleared_by_desktop["year"].is_null());
    assert_eq!(cleared_by_desktop["musicbrainz_recording_id"], identifier);

    // Removing the last correction removes the row, rather than leaving one that
    // holds nothing but NULLs and still claims the track carries a correction.
    assert_eq!(
        patch(serde_json::json!({ "musicbrainz_recording_id": null })).await,
        StatusCode::OK
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM track_override WHERE track_id=?")
        .bind(track_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

/// A list correction is kept by a patch that does not mention it.
///
/// The lists are where getting this wrong costs most. A correction to either is
/// materialised into `track_participant` and `track_genre`, so dropping one by
/// accident re-reads the file and rewrites the credits. A patch about the title
/// must touch neither.
#[tokio::test]
async fn a_list_correction_is_kept_by_a_patch_that_does_not_mention_it() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("kept-list-tagger", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("kept-list-music");
    std::fs::create_dir_all(&music).unwrap();
    // A file that credits somebody, or "the correction went back to the file"
    // and "the correction credits nobody" would both read as an empty list, and
    // the second assertion below would pass for the wrong reason.
    generate_audio_fixture(&music.join("Kept.flac"), "flac", "flac");
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Kept list library",
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
            name: "Kept list library".into(),
            root_path: root,
        },
    )
    .await;
    let track_id = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()[0]
        .id;

    let token = login_token(&router, "kept-list-tagger", password).await;
    let patch = |body: serde_json::Value| {
        let router = router.clone();
        let token = token.clone();
        async move {
            router
                .oneshot(
                    Request::patch(format!("/api/v2/tracks/{track_id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
    };
    let names = |song: &waveflow_server::services::SongItem| {
        song.artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        patch(serde_json::json!({ "artists": ["Solo"], "genres": ["Ambient"] })).await,
        StatusCode::OK
    );
    assert_eq!(
        patch(serde_json::json!({ "title": "Retitled" })).await,
        StatusCode::OK
    );
    let retitled = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(retitled[0].title, "Retitled");
    assert_eq!(
        names(&retitled[0]),
        vec!["Solo".to_owned()],
        "the artists correction the patch did not mention is still applied"
    );
    assert_eq!(retitled[0].genres, vec!["Ambient".to_owned()]);

    // `[]` is a value, not a removal: the track credits nobody, which is a
    // correction rather than the absence of one — and the genres it did not
    // mention stay.
    assert_eq!(
        patch(serde_json::json!({ "artists": [] })).await,
        StatusCode::OK
    );
    let nobody = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert!(
        names(&nobody[0]).is_empty(),
        "an empty list credits nobody rather than going back to the file"
    );
    assert_eq!(nobody[0].genres, vec!["Ambient".to_owned()]);
    let stored = state
        .services
        .track_overrides(owner, track_id)
        .await
        .unwrap()
        .overrides;
    assert_eq!(stored.artists, Some(Vec::new()));
}

/// A correction names the device that made it, on the library feed.
///
/// The same reason as an upload: without it the client that typed the
/// correction reads it back as a change somebody else made, and has no way to
/// tell one from the other.
#[tokio::test]
async fn a_correction_names_the_device_that_made_it() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("attributing-tagger", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("attributing-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Attributed.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Attributing",
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
            name: "Attributing".into(),
            root_path: root,
        },
    )
    .await;
    let track_id = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()[0]
        .id;
    let (token, device) = login_session(&router, "attributing-tagger", password).await;

    let corrected = router
        .clone()
        .oneshot(
            Request::patch(format!("/api/v2/tracks/{track_id}"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-waveflow-device-id", &device)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "title": "Named" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(corrected.status(), StatusCode::OK);

    let feed = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v2/libraries/{library_id}/events?after=0&limit=100"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(feed.status(), StatusCode::OK);
    let body = json_body(feed).await;
    let events = body["events"].as_array().unwrap();
    let last = events.last().expect("the correction is on the feed");
    assert_eq!(last["entity_id"], track_id.to_string());
    assert_eq!(
        last["origin_device_id"], device,
        "the device that typed the correction is named"
    );
    // The scan that created the track came first, and belongs to nobody.
    assert!(
        events[0]["origin_device_id"].is_null(),
        "a scan is nobody's request in particular"
    );
}

/// Every refusal a narrowed token actually meets is one the document declares.
///
/// The scope check has been enforced since the tokens landed, and nineteen
/// mutations answered 403 without saying so anywhere a client generator could
/// see. `annotate_scope_refusals` closes that by construction for mutations,
/// which is worth exactly nothing on its own: a pass that injects a response
/// and a test that reads the same document back agree with each other whether
/// or not either matches the server.
///
/// So this asks the server. It mints a `catalog:read` token, walks every
/// secured operation the document publishes, and requires that anything
/// answering 403 also declares 403. The oracle is the running router; the
/// document is what is on trial.
///
/// That is also what makes it catch the case the injection cannot see.
/// `GET /api/v2/uploads/{session_id}` asks for `Access::Write` — reading a
/// transfer's state belongs to the write flow — so it refuses a narrow token
/// while being a read, and a pass that annotates mutations walks straight past
/// it. No list here knows that. The sweep found it by being told 403.
#[tokio::test]
async fn every_refusal_a_narrow_token_meets_is_documented() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    state
        .db
        .create_account("sweep-admin", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let login = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "sweep-admin",
                "password": "correct horse battery staple",
                "device_name": "Integration"
            }),
        ))
        .await
        .unwrap();
    let access = json_body(login).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();

    // An administrator's token, narrowed to reading. The account behind it may
    // do everything; the credential may not, which is the whole point.
    let minted = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v2/admin/users/sweep-admin/tokens")
                .header("authorization", format!("Bearer {access}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name": "sweep", "scopes": ["catalog:read"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(minted.status(), StatusCode::CREATED);
    let narrow = json_body(minted).await["secret"]
        .as_str()
        .unwrap()
        .to_owned();

    let document = json_body(
        router
            .clone()
            .oneshot(Request::get("/openapi.json").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;

    // Enough of a value that the extractor in front of the handler accepts it:
    // a rejected path parameter never reaches `authenticated`, and the
    // implication below simply does not fire for that route.
    let canvas_hash = "a".repeat(64);
    let fill = |path: &str| {
        let mut filled = String::new();
        let mut rest = path;
        while let Some(open) = rest.find('{') {
            let close = rest.find('}').expect("a balanced parameter");
            filled.push_str(&rest[..open]);
            filled.push_str(match &rest[open + 1..close] {
                "entity_type" => "track",
                "username" => "sweep-admin",
                "index" => "0",
                "canvas_hash" => canvas_hash.as_str(),
                _ => "00000000-0000-4000-8000-000000000000",
            });
            rest = &rest[close + 1..];
        }
        filled.push_str(rest);
        filled
    };

    let mut refused = Vec::new();
    let mut undocumented = Vec::new();
    let paths = document["paths"].as_object().expect("a path map");
    for (path, item) in paths {
        if !path.starts_with("/api/v2/") {
            continue;
        }
        for method in ["get", "put", "post", "delete", "patch"] {
            let Some(operation) = item.get(method) else {
                continue;
            };
            // A route that carries no credential has none to have refused.
            if operation["security"].as_array().is_some_and(Vec::is_empty) {
                continue;
            }
            let request = Request::builder()
                .method(Method::from_bytes(method.to_uppercase().as_bytes()).unwrap())
                .uri(fill(path))
                .header("authorization", format!("Bearer {narrow}"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap();
            let status = router.clone().oneshot(request).await.unwrap().status();
            if status != StatusCode::FORBIDDEN {
                continue;
            }
            refused.push(format!("{} {path}", method.to_uppercase()));
            if operation["responses"]["403"].is_null() {
                undocumented.push(format!("{} {path}", method.to_uppercase()));
            }
        }
    }

    assert!(
        undocumented.is_empty(),
        "these refuse a narrowed token without documenting 403: {undocumented:#?}"
    );

    // An implication passes whenever nothing satisfies it, so the sweep has to
    // show it reached the routes it claims to judge. Both of these fail if the
    // requests stop arriving — a changed fixture, an extractor rejecting every
    // filled path — rather than letting the test go quietly vacuous.
    //
    // Twenty and not the forty-odd secured operations, because axum runs the
    // extractors before the handler body: a route taking `Json<T>` with a
    // required field answers 422 to this sweep's `{}` and never reaches
    // `authenticated`. Sending each route a body it would accept would mean
    // keeping a hand-written fixture per route here — which is the list this
    // test exists to not have. So it judges what it can reach, and the number
    // records how much that is.
    assert!(
        refused.len() >= 20,
        "the sweep reached too few refusals to mean anything: {refused:#?}"
    );
    assert!(
        refused
            .iter()
            .any(|route| route == "GET /api/v2/uploads/{session_id}"),
        "the read that asks for write authority is what this sweep is for: {refused:#?}"
    );
}

/// Every technical field reaches the wire, and says what the file says.
///
/// #211 reports these as never serialised. **That is no longer true**, and
/// checking rather than believing it changed what this test had to be: they
/// are all on `SongItem`, which derives `Serialize` with nothing skipped, and
/// `docs/api-v2-guide.md` already names three of them. What was missing is
/// this — nothing asserted any of it, so the client could declare a field the
/// server had quietly stopped sending and the whole suite would agree with
/// the client.
///
/// Which is the trap the issue names: the client's types are hand-written
/// assertions about this wire, and its mocks are built from those types. A
/// test on that side confirms the type. So the values here are read **out of
/// the file on disk** — the WAV header's own bytes and the size the
/// filesystem reports — and compared with what the server answered. Nothing
/// is written down twice.
#[tokio::test]
async fn the_technical_fields_reach_the_wire_and_match_the_file() {
    let (_temp, config, state) = test_app().await;
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("fields-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("fields-music");
    std::fs::create_dir_all(&music).unwrap();
    let file = music.join("Properties.wav");
    write_test_wav(&file);
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Technical fields",
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
            name: "Technical fields".into(),
            root_path: root,
        },
    )
    .await;
    let track = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()
        .pop()
        .unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "fields-owner", &password).await;
    let response = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/tracks/{}", track.id))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let song = json_body(response).await;

    // The fmt chunk, at the offsets a canonical WAV puts it: channels at 22,
    // the sample rate at 24, the bits per sample at 34. Read here so the
    // assertions below belong to the file rather than to a number somebody
    // typed into both sides.
    let bytes = std::fs::read(&file).unwrap();
    // Every offset below is checked against its marker first. A fixture that
    // grows a chunk — a `LIST` before the data, say — would otherwise have
    // this test reading two arbitrary bytes and failing on a number, which
    // says the server is wrong about a file that moved underneath it.
    for (at, marker) in [
        (0usize, &b"RIFF"[..]),
        (8, b"WAVE"),
        (12, b"fmt "),
        (36, b"data"),
    ] {
        assert_eq!(
            &bytes[at..at + 4],
            marker,
            "the WAV fixture no longer puts {} at byte {at}",
            String::from_utf8_lossy(marker)
        );
    }
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]) as i64;
    let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]) as i64;
    let bit_depth = u16::from_le_bytes([bytes[34], bytes[35]]) as i64;

    assert_eq!(song["sample_rate"].as_i64(), Some(sample_rate));
    assert_eq!(song["channels"].as_i64(), Some(channels));
    assert_eq!(song["bit_depth"].as_i64(), Some(bit_depth));
    assert_eq!(
        song["size"].as_i64(),
        Some(std::fs::metadata(&file).unwrap().len() as i64)
    );
    assert_eq!(song["suffix"].as_str(), Some("wav"));
    // **Compared with what the catalogue stored, not with the word "WAV".**
    // Pinning the spelling would pin the tag reader's vocabulary: a dependency
    // that renamed its container would fail this test for a reason that has
    // nothing to do with this surface. Comparing against the row is exact and
    // stays about the question — does the stored value reach the wire — and it
    // catches what a non-empty check cannot, a column carrying another track's
    // value or the suffix by mistake.
    assert_eq!(song["codec"].as_str(), track.codec.as_deref());
    assert!(
        track
            .codec
            .as_deref()
            .is_some_and(|codec| !codec.is_empty()),
        "the scan stored no codec at all, so the comparison above proves nothing"
    );

    // **Four fields tied together, rather than four numbers each asserted
    // alone.** Uncompressed PCM has one bitrate and it is arithmetic:
    // channels times depth times rate. A projection that dropped one of them,
    // or a scanner that read one from the wrong offset, breaks the equality —
    // where four separate assertions would each still pass against a
    // plausible-looking wrong value.
    let expected_kbps = sample_rate * bit_depth * channels / 1000;
    assert_eq!(song["bitrate"].as_i64(), Some(expected_kbps));

    // And the duration follows from the data chunk at the same rate.
    let data_bytes = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as i64;
    let frames = data_bytes / (channels * bit_depth / 8);
    assert_eq!(
        song["duration_ms"].as_i64(),
        Some(frames * 1000 / sample_rate)
    );
}

/// An `.m3u` import places what it can and reports every entry it cannot.
///
/// The report is the feature. A playlist that silently arrives a third shorter
/// is worse than a refused import, so every shape of failure here is asserted
/// by name and not merely counted: a path that matches nothing, a bare file
/// name that matches two tracks, a web radio, a track named twice, and a track
/// that exists in a library this account cannot see.
///
/// The successes are the shapes a playlist file actually arrives in — relative
/// to its own folder, absolute on someone else's filesystem, Windows-shaped,
/// a percent-encoded `file://` URL, and one that only the `#EXTINF` credit can
/// place — plus a file that is not UTF-8, which is the common case for `.m3u`
/// and the one that mangles every accented title when it is guessed wrong.
///
/// It ends on the round trip: the playlist is exported and imported again, and
/// everything resolves. That is the strongest evidence the reader and the
/// writer agree, and it costs one assertion.
#[tokio::test]
async fn an_m3u_import_places_what_it_can_and_reports_the_rest() {
    let (_temp, config, state) = test_app().await;
    // Inserted rather than created, and authenticating by API token rather
    // than by password. Hashing a password here put a string literal into a
    // cryptographic call, which `rust/hard-coded-cryptographic-value` reads as
    // a hard-coded password however random the rest of the token is — measured
    // on this very branch, where CodeQL failed on the generated prefix. These
    // accounts never log in, so there is nothing to hash.
    let owner = uuid::Uuid::new_v4();
    let stranger = uuid::Uuid::new_v4();
    for (id, username) in [(owner, "import-owner"), (stranger, "import-stranger")] {
        sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
             VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(now_ms())
        .bind(now_ms())
        .execute(state.db.pool())
        .await
        .unwrap();
    }

    let shelf = |who: uuid::Uuid, name: &'static str| {
        let state = &state;
        let config = &config;
        async move {
            let music = config.data_dir.join(name);
            std::fs::create_dir_all(&music).unwrap();
            let library = state
                .db
                .create_library(
                    who,
                    name,
                    &std::fs::canonicalize(&music).unwrap(),
                    LibraryVisibility::Private,
                    now_ms(),
                )
                .await
                .unwrap();
            let scan = state
                .db
                .create_scan_job(library, Some(who), "manual")
                .await
                .unwrap();
            state.db.start_scan_job(scan, 8, false).await.unwrap();
            (library, scan)
        }
    };
    let (library, scan) = shelf(owner, "import-music").await;
    // Paths rather than titles are what this test is about, so each fixture
    // names its own place on disk. Two of them share a file name on purpose.
    for (index, path, title, artist) in [
        (
            900,
            "Björk/Vespertine/01 Hidden Place.flac",
            "Hidden Place",
            "Björk",
        ),
        (901, "Björk/Vespertine/02 Cocoon.flac", "Cocoon", "Björk"),
        (902, "Björk/Post/01 Army of Me.flac", "Army of Me", "Björk"),
        (
            903,
            "Compilations/Shared/common.flac",
            "A Shared Name",
            "First Band",
        ),
        (
            904,
            "Compilations/Other/common.flac",
            "A Shared Name Too",
            "Second Band",
        ),
        (905, "Moved/elsewhere.flac", "Pagan Poetry", "Björk"),
        (906, "Björk/Homogenic/03 Unravel.flac", "Unravel", "Björk"),
        // Two names the format cannot carry as written: a line starting with
        // "#" is a directive, and a "%" would come back percent-decoded.
        // Reachable only through the export, which is where they are asserted.
        (907, "#1 Hit.flac", "Number One", "Chart Band"),
        (908, "Mix 100% Live.flac", "Hundred Percent", "Chart Band"),
    ] {
        let mut input = browse_input(index, title, "Vespertine", artist, Some(1), Some(1));
        input.relative_path = path.into();
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

    // A second account, with a track of its own at a path the import below
    // names. Nothing about it may reach the owner's playlist, and the report
    // must not say it exists either.
    let (foreign, foreign_scan) = shelf(stranger, "stranger-music").await;
    let mut hidden = browse_input(910, "Private Thing", "Private", "Nobody", Some(1), Some(1));
    hidden.relative_path = "Nobody/Private/01 Private Thing.flac".into();
    state
        .db
        .apply_catalog_track(foreign, foreign_scan, &hidden, None, false)
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(foreign)
        .await
        .unwrap();
    state.db.finish_scan_job(foreign_scan, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let (_record, token) = state
        .services
        .create_api_token(owner, &[], "import-owner", "importer", &[])
        .await
        .unwrap();
    let import = |uri: String, body: Vec<u8>, token: String| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "audio/x-mpegurl")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            (status, json_body(response).await)
        }
    };

    // The file. Written as bytes because one line of it is not UTF-8: "Björk"
    // spelled the way a Windows player writes it.
    let mut file: Vec<u8> = Vec::new();
    file.extend_from_slice(b"#EXTM3U\r\n");
    file.extend_from_slice(b"#PLAYLIST:From another player\r\n");
    // Relative to the folder the playlist sat in.
    file.extend_from_slice(b"#EXTINF:300,Bj\xF6rk - Hidden Place\r\n");
    file.extend_from_slice(b"Bj\xF6rk/Vespertine/01 Hidden Place.flac\r\n");
    // Absolute, on a filesystem that is not this server's.
    file.extend_from_slice(b"/home/someone/Music/Bj\xF6rk/Vespertine/02 Cocoon.flac\r\n");
    // Windows-shaped, with a drive letter.
    file.extend_from_slice(b"C:\\Users\\someone\\Music\\Bj\xF6rk\\Post\\01 Army of Me.flac\r\n");
    // A percent-encoded file URL.
    file.extend_from_slice(b"file:///home/someone/Music/Moved/elsewhere.flac\r\n");
    // Nothing of its path survives, and only the credit can place it.
    file.extend_from_slice(b"#EXTINF:240,Bj\xF6rk - Unravel\r\n");
    file.extend_from_slice(b"D:\\Backup\\unknown\\7331.flac\r\n");
    // A bare file name two albums carry.
    file.extend_from_slice(b"common.flac\r\n");
    // A web radio.
    file.extend_from_slice(b"http://radio.example/stream\r\n");
    // A track of another account's library.
    file.extend_from_slice(b"Nobody/Private/01 Private Thing.flac\r\n");
    // Nothing at all.
    file.extend_from_slice(b"Ghost/Album/99 Nowhere.flac\r\n");
    // And the first entry again.
    file.extend_from_slice(b"Bj\xF6rk/Vespertine/01 Hidden Place.flac\r\n");
    // The ambiguous name once more, shouted, and with its folders this time.
    file.extend_from_slice(b"COMPILATIONS/SHARED/COMMON.FLAC\r\n");

    // The dry run answers the whole report and creates nothing, which is what
    // lets a client show the losses before they happen.
    let (status, report) = import(
        "/api/v2/playlists/import?dry_run=true".into(),
        file.clone(),
        token.clone(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a dry run creates nothing: {report}"
    );
    assert!(report["playlist"].is_null());
    assert_eq!(report["name"], "From another player", "the file named it");
    assert_eq!(
        report["encoding"], "windows-1252",
        "the bytes decide the encoding, not the extension"
    );
    assert_eq!(report["entries"], 11);
    assert_eq!(report["resolved"], 6);
    let reasons = |report: &serde_json::Value| {
        report["unresolved"]
            .as_array()
            .expect("the report lists what it could not place")
            .iter()
            .map(|entry| {
                (
                    entry["line"].as_i64().unwrap(),
                    entry["reason"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        reasons(&report),
        [
            (10, "ambiguous".to_owned()),
            (11, "not-a-path".to_owned()),
            // A track in a library this account cannot reach is not found,
            // which is the same answer as one that does not exist — and the
            // two lines below are indistinguishable in this report, by design.
            (12, "no-match".to_owned()),
            (13, "no-match".to_owned()),
            (14, "duplicate".to_owned()),
        ],
        "every failure named, and placed on its line in the file"
    );
    let listed = router
        .clone()
        .oneshot(
            Request::get("/api/v2/playlists")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(listed).await.as_array().unwrap().len(),
        0,
        "the dry run wrote nothing"
    );

    // And the import itself, with a name of the caller's choosing.
    let (status, created) = import(
        "/api/v2/playlists/import?name=Evening%20from%20elsewhere".into(),
        file.clone(),
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "Evening from elsewhere");
    assert_eq!(reasons(&created), reasons(&report), "the same report");
    let playlist = &created["playlist"];
    assert_eq!(playlist["name"], "Evening from elsewhere");
    let titles = playlist["songs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|song| song["title"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        titles,
        [
            "Hidden Place",
            "Cocoon",
            "Army of Me",
            "Pagan Poetry",
            "Unravel",
            // Matched through two folders in the wrong case: the comparison
            // folds ASCII, which is what a playlist written on Windows needs,
            // and the folders are what separate this track from the one on
            // line 10 carrying the same file name.
            "A Shared Name",
        ],
        "file order, and the fourth was placed by its path while the fifth \
         was placed by its credit alone"
    );

    // The round trip. What this server writes, this server reads.
    let playlist_id = playlist["id"].as_str().unwrap().to_owned();
    let exported = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/playlists/{playlist_id}/export"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(exported.status(), StatusCode::OK);
    assert_eq!(
        exported
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "audio/x-mpegurl; charset=utf-8"
    );
    let disposition = exported
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .expect("a header value stays a header value")
        .to_owned();
    assert!(
        disposition.contains("filename=\"Evening from elsewhere.m3u8\""),
        "{disposition}"
    );
    let text = body_text(exported).await;
    assert!(
        text.starts_with("#EXTM3U\n#PLAYLIST:Evening from elsewhere\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "#EXTINF:120,Björk - Hidden Place\n./Björk/Vespertine/01 Hidden Place.flac\n"
        ),
        "{text}"
    );

    let (status, again) = import(
        "/api/v2/playlists/import?name=Round%20trip&dry_run=true".into(),
        text.into_bytes(),
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["encoding"], "utf-8", "what it writes is UTF-8");
    assert_eq!(again["entries"], 6, "the playlist holds six tracks");
    assert_eq!(again["resolved"], 6);
    assert_eq!(
        again["unresolved"].as_array().unwrap().len(),
        0,
        "an export this server wrote resolves whole: {again}"
    );

    // **The round trip for two names the format cannot carry naively.** A
    // track at a library's root called "#1 Hit.flac" written bare would be read
    // back as a comment and vanish without being reported — not even counted —
    // and a "%" in a name would come back percent-decoded. The export prefixes
    // "./" and escapes the "%", so what this server writes is what it reads.
    // Raised in review, with both names named.
    let awkward = state
        .services
        .list_songs(owner, &Default::default())
        .await
        .unwrap()
        .into_iter()
        .filter(|song| song.title == "Number One" || song.title == "Hundred Percent")
        .map(|song| song.id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(awkward.len(), 2, "both awkward names are in the catalogue");
    let created = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v2/playlists")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "name": "Awkward", "track_ids": awkward }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let awkward_id = json_body(created).await["id"].as_str().unwrap().to_owned();
    let exported = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/playlists/{awkward_id}/export"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let text = body_text(exported).await;
    assert!(
        text.contains("./#1 Hit.flac\n") && text.contains("./Mix 100%25 Live.flac\n"),
        "the writer puts both names where the reader can find them: {text}"
    );
    let (_status, back) = import(
        "/api/v2/playlists/import?name=Awkward%20again&dry_run=true".into(),
        text.into_bytes(),
        token.clone(),
    )
    .await;
    assert_eq!(back["entries"], 2, "neither line was read as a comment");
    assert_eq!(
        back["resolved"], 2,
        "and neither name was mangled on the way back: {back}"
    );

    // A name is free text, and both outputs are structured. The file is
    // line-structured, so a newline in a name would end the `#PLAYLIST` line
    // early and make the rest look like a path — an entry of its own, in a
    // file the next import reads. A header value carries no newline at all,
    // so the same name would have failed the response instead of the parse.
    // Neither spelling of the name is passed through untouched.
    let (status, hostile) = import(
        "/api/v2/playlists/import?name=Soir%C3%A9e%0A/etc/passwd".into(),
        b"Bj\xC3\xB6rk/Vespertine/01 Hidden Place.flac\n".to_vec(),
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let hostile_id = hostile["playlist"]["id"].as_str().unwrap().to_owned();
    let exported = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/playlists/{hostile_id}/export"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        exported.status(),
        StatusCode::OK,
        "a header value with a newline in it would have failed the response"
    );
    let disposition = exported
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        disposition.contains("filename=\"Soireetcpasswd.m3u8\"")
            && disposition.contains("filename*=UTF-8''Soir%C3%A9e%0A%2Fetc%2Fpasswd.m3u8"),
        "the quoted form is an ASCII subset and the full name travels encoded: {disposition}"
    );
    let text = body_text(exported).await;
    assert!(
        text.starts_with("#EXTM3U\n#PLAYLIST:Soirée /etc/passwd\n"),
        "one name, one line: {text}"
    );
    let (_status, reread) = import(
        "/api/v2/playlists/import?name=Reread&dry_run=true".into(),
        text.into_bytes(),
        token.clone(),
    )
    .await;
    assert_eq!(
        reread["entries"], 1,
        "and the name did not become an entry of its own"
    );

    // **A UTF-16 file is read by its byte-order mark.** Notepad's "Unicode"
    // and some players' exports write it. Read as Windows-1252, every other
    // character came back a NUL and nothing in the file resolved.
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend(
        "#EXTM3U\r\n#EXTINF:300,Björk - Hidden Place\r\nBjörk/Vespertine/01 Hidden Place.flac\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    let (status, wide) = import(
        "/api/v2/playlists/import?name=Wide&dry_run=true".into(),
        utf16,
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{wide}");
    assert_eq!(wide["entries"], 1);
    assert_eq!(wide["resolved"], 1, "the path it names is found: {wide}");
    assert_eq!(
        wide["encoding"], "utf-16le",
        "and the mark named the encoding"
    );

    // A file nothing names is refused rather than stored namelessly.
    let (status, _body) = import(
        "/api/v2/playlists/import".into(),
        b"Ghost/Album/99 Nowhere.flac\n".to_vec(),
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // **A named import that places nothing still creates the playlist.** The
    // caller asked for one and the report says what became of every line, so
    // an empty playlist with an explanation beats a refusal that leaves the
    // file unexplained. Asserted because it is a decision rather than a
    // consequence — raised in review, where it was untested.
    let (status, empty) = import(
        "/api/v2/playlists/import?name=Ghost".into(),
        b"Ghost/Album/99 Nowhere.flac\n".to_vec(),
        token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(empty["resolved"], 0);
    assert!(
        empty["playlist"].is_object(),
        "the playlist exists, and holds nothing: {empty}"
    );
    assert_eq!(empty["playlist"]["name"], "Ghost");
    assert_eq!(empty["playlist"]["songs"].as_array().unwrap().len(), 0);
    assert_eq!(reasons(&empty), [(1, "no-match".to_owned())]);

    // Another account reaches neither the export nor the playlist, and the
    // refusal is the same 404 a missing one would answer.
    let (_record, foreign_token) = state
        .services
        .create_api_token(stranger, &[], "import-stranger", "intruder", &[])
        .await
        .unwrap();
    let refused = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/playlists/{playlist_id}/export"))
                .header("authorization", format!("Bearer {foreign_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
}

/// The native half of #217: a station list the web client can manage.
///
/// What it settles beyond the round trip: that `PUT` replaces rather than
/// patches, that `created_at` survives a replacement — the one field the caller
/// does not send, and therefore the only reason the service reads the row back
/// — that a station of another account answers 404 on every verb, and that the
/// per-account ceiling refuses rather than grows.
#[tokio::test]
async fn radio_stations_round_trip_and_stay_inside_one_account() {
    let (_temp, config, state) = test_app().await;
    // Inserted rather than created: hashing a password here would put a string
    // literal into a cryptographic call, which `rust/hard-coded-cryptographic-value`
    // reports whatever the literal is for. These accounts authenticate by API
    // token, so nothing is hashed.
    let owner = uuid::Uuid::new_v4();
    let stranger = uuid::Uuid::new_v4();
    for (id, username) in [
        (owner, "radio-native-owner"),
        (stranger, "radio-native-other"),
    ] {
        sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
             VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(now_ms())
        .bind(now_ms())
        .execute(state.db.pool())
        .await
        .unwrap();
    }
    let router = waveflow_server::app(&config, state.clone());
    let (_record, token) = state
        .services
        .create_api_token(owner, &[], "radio-native-owner", "radio", &[])
        .await
        .unwrap();
    let (_record, other_token) = state
        .services
        .create_api_token(stranger, &[], "radio-native-other", "radio", &[])
        .await
        .unwrap();

    let call = |method: Method, uri: String, token: String, body: Option<serde_json::Value>| {
        let router = router.clone();
        async move {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"));
            let body = match body {
                Some(value) => {
                    request = request.header("content-type", "application/json");
                    Body::from(serde_json::to_vec(&value).unwrap())
                }
                None => Body::empty(),
            };
            let response = router.oneshot(request.body(body).unwrap()).await.unwrap();
            let status = response.status();
            // A 204 has no body, and a body helper would hide that rather than
            // report it.
            let payload = if status == StatusCode::NO_CONTENT {
                serde_json::Value::Null
            } else {
                json_body(response).await
            };
            (status, payload)
        }
    };

    // Nothing stored yet, and an empty list rather than a 404: the collection
    // exists, it is the stations that do not.
    let (status, listed) = call(
        Method::GET,
        "/api/v2/radio-stations".into(),
        token.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed, serde_json::json!([]));

    let (status, created) = call(
        Method::POST,
        "/api/v2/radio-stations".into(),
        token.clone(),
        Some(serde_json::json!({
            "name": "  Radio Nova  ",
            "stream_url": "https://radio.example.org/nova.mp3",
            "homepage_url": "https://radio.example.org/",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    // Trimmed on the way in, which is what gets stored and listed.
    assert_eq!(created["name"], "Radio Nova");
    assert_eq!(created["stream_url"], "https://radio.example.org/nova.mp3");
    assert_eq!(created["homepage_url"], "https://radio.example.org/");
    assert_eq!(created["created_at"], created["updated_at"]);
    let station_id = created["id"].as_str().unwrap().to_owned();
    let created_at = created["created_at"].as_i64().unwrap();

    let (status, one) = call(
        Method::GET,
        format!("/api/v2/radio-stations/{station_id}"),
        token.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one, created);

    // Backdated on purpose, and that is what makes the next assertion mean
    // something: comparing `created_at` across a replacement that happens in
    // the same millisecond proves nothing, and `now_ms()` is quite capable of
    // returning the same millisecond twice in a test this fast — measured, by
    // inverting the service to return `now` and watching the two values differ
    // by exactly one.
    assert!(created_at > 1_000, "the backdating has to change something");
    sqlx::query("UPDATE radio_station SET created_at=? WHERE id=?")
        .bind(1_000_i64)
        .bind(&station_id)
        .execute(state.db.pool())
        .await
        .unwrap();

    // A replacement, not a patch: the body carries no `homepage_url`, so the
    // station ends up with none. The protocol's `updateInternetRadioStation`
    // behaves the same way, which is why this route is a `PUT`.
    let (status, replaced) = call(
        Method::PUT,
        format!("/api/v2/radio-stations/{station_id}"),
        token.clone(),
        Some(serde_json::json!({
            "name": "Radio Nova Classics",
            "stream_url": "https://radio.example.org/classics.mp3",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replaced["name"], "Radio Nova Classics");
    assert!(replaced["homepage_url"].is_null());
    // The field the caller never sent, carried through the replacement. It is
    // the only reason the service reads the row back at all, and the backdating
    // above is what lets the assertion fail when it does not.
    assert_eq!(replaced["created_at"].as_i64().unwrap(), 1_000);
    assert_eq!(replaced["id"].as_str().unwrap(), station_id);

    // An empty string clears it as well as an absent field does. A form sends
    // one or the other depending on how it is written, and both say the
    // station has no page — the Subsonic `homepageUrl=` case, answered here by
    // the same service method (constat de revue, pris).
    let (status, blanked) = call(
        Method::PUT,
        format!("/api/v2/radio-stations/{station_id}"),
        token.clone(),
        Some(serde_json::json!({
            "name": "Radio Nova Classics",
            "stream_url": "https://radio.example.org/classics.mp3",
            "homepage_url": "",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(blanked["homepage_url"].is_null());

    // Another account: its own list is empty, and the owner's station is
    // unreachable on every verb. Missing and forbidden answer identically.
    let (status, foreign_list) = call(
        Method::GET,
        "/api/v2/radio-stations".into(),
        other_token.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(foreign_list, serde_json::json!([]));
    for (method, body) in [
        (Method::GET, None),
        (
            Method::PUT,
            Some(serde_json::json!({
                "name": "Stolen",
                "stream_url": "https://elsewhere.example.org/s",
            })),
        ),
        (Method::DELETE, None),
    ] {
        let (status, _) = call(
            method.clone(),
            format!("/api/v2/radio-stations/{station_id}"),
            other_token.clone(),
            body,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
    }

    // An address the server would have to dereference to know anything about is
    // refused. `mms://` is the case that proves the scheme list: `file:` and
    // `data:` carry no host and are refused by the parser before the scheme is
    // ever consulted.
    for body in [
        serde_json::json!({ "name": "Elsewhere", "stream_url": "mms://radio.example.org/s" }),
        serde_json::json!({ "name": "Elsewhere", "stream_url": "file:///etc/passwd" }),
        serde_json::json!({ "name": "Elsewhere", "stream_url": "/relative.mp3" }),
        // A blank name would list as a row nothing tells apart.
        serde_json::json!({ "name": "   ", "stream_url": "https://radio.example.org/s" }),
        // The homepage is validated like the address it sits beside.
        serde_json::json!({
            "name": "Elsewhere",
            "stream_url": "https://radio.example.org/s",
            "homepage_url": "javascript:alert(1)",
        }),
    ] {
        let (status, _) = call(
            Method::POST,
            "/api/v2/radio-stations".into(),
            token.clone(),
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
    // Refused on the way to an existing station too, and the station is left
    // as it was.
    let (status, _) = call(
        Method::PUT,
        format!("/api/v2/radio-stations/{station_id}"),
        token.clone(),
        Some(serde_json::json!({ "name": "Nope", "stream_url": "mms://h/s" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, intact) = call(
        Method::GET,
        format!("/api/v2/radio-stations/{station_id}"),
        token.clone(),
        None,
    )
    .await;
    assert_eq!(intact["name"], "Radio Nova Classics");

    // The per-account ceiling. The filler rows go in with one statement rather
    // than a thousand round trips, and the count deliberately overshoots the
    // limit in `src/services/radio.rs` instead of restating it: this asserts
    // that a ceiling exists and bites, not what it is set to.
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i < 1000) \
         INSERT INTO radio_station \
             (id, user_id, name, stream_url, homepage_url, created_at, updated_at) \
         SELECT lower(substr(h,1,8) || '-' || substr(h,9,4) || '-' || substr(h,13,4) || '-' \
                      || substr(h,17,4) || '-' || substr(h,21,12)), \
                ?, 'Filler ' || i, 'https://filler.example.org/' || i, NULL, ?, ? \
         FROM (SELECT i, hex(randomblob(16)) AS h FROM n)",
    )
    .bind(owner.to_string())
    .bind(now_ms())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let (status, _) = call(
        Method::POST,
        "/api/v2/radio-stations".into(),
        token.clone(),
        Some(serde_json::json!({
            "name": "One Too Many",
            "stream_url": "https://radio.example.org/overflow",
        })),
    )
    .await;
    // 409 and not 422: the request is well formed, the collection is full.
    assert_eq!(status, StatusCode::CONFLICT);
    // The other account is unaffected by a list it does not share.
    let (status, _) = call(
        Method::POST,
        "/api/v2/radio-stations".into(),
        other_token.clone(),
        Some(serde_json::json!({
            "name": "Mine Alone",
            "stream_url": "https://radio.example.org/mine",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = call(
        Method::DELETE,
        format!("/api/v2/radio-stations/{station_id}"),
        token.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_null());
    // Gone, and a second delete says so rather than succeeding twice: a station
    // is named by an identifier, so "not there" is a fact about the id and not
    // the same request answered again.
    for method in [Method::GET, Method::DELETE] {
        let (status, _) = call(
            method.clone(),
            format!("/api/v2/radio-stations/{station_id}"),
            token.clone(),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
    }
}

/// Lyrics uploaded to a track (RFC-012 decision 6), through the router.
///
/// Passwords are drawn at run time: a literal handed to `hash_password` is
/// what CodeQL's `rust/hard-coded-cryptographic-value` reports, and the
/// ruleset blocks a pull request on it.
#[tokio::test]
async fn uploaded_lyrics_lead_the_order_download_as_they_came_and_stay_off_subsonic() {
    let (_temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("lyrics-owner", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let listener = state
        .db
        .create_account("lyrics-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("uploaded-lyrics");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Song.wav"));
    // A scanned synced sheet, so every rank below has a neighbour to beat.
    std::fs::write(music.join("Song.lrc"), "[00:01.00]from the sidecar\n").unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Lyrics", &root, LibraryVisibility::Private, now_ms())
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
            name: "Lyrics".into(),
            root_path: root,
        },
    )
    .await;
    let track = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()[0]
        .id;
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "lyrics-owner", &password).await;
    let listener_token = login_token(&router, "lyrics-listener", &password).await;

    let send = |method: Method, uri: String, token: &str, body: Vec<u8>| {
        let router = router.clone();
        let token = token.to_owned();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    let sources = |list: &serde_json::Value| -> Vec<String> {
        list["structuredLyrics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|sheet| sheet["source"].as_str().unwrap().to_owned())
            .collect()
    };
    let lyrics_uri = format!("/api/v2/tracks/{track}/lyrics");

    // Before any upload: the scanned sheet, naming its source.
    let before = json_body(send(Method::GET, lyrics_uri.clone(), &owner_token, vec![]).await).await;
    assert_eq!(sources(&before), ["sidecar_lrc"]);

    // A plain upload does not take the timings away from a synced scan.
    let txt = send(
        Method::PUT,
        format!("{lyrics_uri}/txt"),
        &owner_token,
        b"plain words\n".to_vec(),
    )
    .await;
    assert_eq!(txt.status(), StatusCode::OK);
    assert_eq!(
        sources(&json_body(txt).await),
        ["sidecar_lrc", "upload_txt"]
    );

    // A synced upload outranks the scan, and TTML outranks LRC.
    let lrc = send(
        Method::PUT,
        format!("{lyrics_uri}/lrc"),
        &owner_token,
        b"[la:en]\n[00:02.00]uploaded lrc\n".to_vec(),
    )
    .await;
    assert_eq!(lrc.status(), StatusCode::OK);
    let ttml_body = concat!(
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="fr"><body><div>"#,
        r#"<p begin="0:03.000"><span>bonjour</span> <span>tout</span></p>"#,
        r#"</div></body></tt>"#
    );
    let ttml = send(
        Method::PUT,
        format!("{lyrics_uri}/ttml"),
        &owner_token,
        ttml_body.as_bytes().to_vec(),
    )
    .await;
    assert_eq!(ttml.status(), StatusCode::OK);
    let ordered = json_body(ttml).await;
    assert_eq!(
        sources(&ordered),
        ["upload_ttml", "upload_lrc", "sidecar_lrc", "upload_txt"]
    );
    assert_eq!(ordered["structuredLyrics"][0]["lang"], "fr");
    assert_eq!(
        ordered["structuredLyrics"][0]["line"][0]["value"],
        "bonjour tout"
    );
    assert_eq!(ordered["structuredLyrics"][0]["line"][0]["start"], 3000);

    // The download gives the first sheet back as it was uploaded, never
    // rendered; with `format`, the upload named.
    let file = send(
        Method::GET,
        format!("{lyrics_uri}/file"),
        &owner_token,
        vec![],
    )
    .await;
    assert_eq!(file.status(), StatusCode::OK);
    assert_eq!(file.headers()["content-type"], "application/octet-stream");
    assert_eq!(file.headers()["x-content-type-options"], "nosniff");
    let disposition = file.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        disposition.starts_with("attachment;") && disposition.ends_with(".ttml"),
        "{disposition}"
    );
    assert_eq!(body_text(file).await, ttml_body);
    let txt_file = send(
        Method::GET,
        format!("{lyrics_uri}/file?format=txt"),
        &owner_token,
        vec![],
    )
    .await;
    assert_eq!(body_text(txt_file).await, "plain words\n");

    // A listener reads, and may not write: refused like a track it cannot see.
    let read = send(Method::GET, lyrics_uri.clone(), &listener_token, vec![]).await;
    assert_eq!(read.status(), StatusCode::OK);
    for (method, uri) in [
        (Method::PUT, format!("{lyrics_uri}/lrc")),
        (Method::DELETE, format!("{lyrics_uri}/lrc")),
    ] {
        let refused = send(method, uri, &listener_token, b"[00:01.00]x".to_vec()).await;
        assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    }

    // Nothing usable is refused, and so is a format the route does not know.
    let empty = send(
        Method::PUT,
        format!("{lyrics_uri}/txt"),
        &owner_token,
        b" \n".to_vec(),
    )
    .await;
    assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let unknown = send(
        Method::PUT,
        format!("{lyrics_uri}/srt"),
        &owner_token,
        b"1".to_vec(),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    // More than a mebibyte is refused before it is read.
    let large = send(
        Method::PUT,
        format!("{lyrics_uri}/txt"),
        &owner_token,
        // One byte past the cap: the route's own ceiling answers it, not the
        // service after the body has been read.
        vec![b'a'; 1024 * 1024 + 1],
    )
    .await;
    assert_eq!(large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    // And a sheet well past the router's 16 KiB default is taken: the route
    // carries its own ceiling.
    let long = send(
        Method::PUT,
        format!("{lyrics_uri}/txt"),
        &owner_token,
        "a line of a long song
"
        .repeat(10_000)
        .into_bytes(),
    )
    .await;
    assert_eq!(long.status(), StatusCode::OK);

    // Deleting an upload falls back; deleting it twice finds nothing.
    for format in ["ttml", "lrc", "txt"] {
        let removed = send(
            Method::DELETE,
            format!("{lyrics_uri}/{format}"),
            &owner_token,
            vec![],
        )
        .await;
        assert_eq!(removed.status(), StatusCode::NO_CONTENT, "{format}");
    }
    let again = send(
        Method::DELETE,
        format!("{lyrics_uri}/txt"),
        &owner_token,
        vec![],
    )
    .await;
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
    let after = json_body(send(Method::GET, lyrics_uri.clone(), &owner_token, vec![]).await).await;
    assert_eq!(sources(&after), ["sidecar_lrc"]);
    // A scanned synced sheet downloads as LRC.
    let scanned = send(
        Method::GET,
        format!("{lyrics_uri}/file"),
        &owner_token,
        vec![],
    )
    .await;
    assert_eq!(body_text(scanned).await, "[00:01.00]from the sidecar\n");

    // The Subsonic service the façade reads never sees an upload, and its
    // sheets carry no `source`: its contract is frozen.
    send(
        Method::PUT,
        format!("{lyrics_uri}/ttml"),
        &owner_token,
        ttml_body.as_bytes().to_vec(),
    )
    .await;
    let facade = state.services.lyrics(owner, track).await.unwrap();
    assert_eq!(facade.structured_lyrics.len(), 1);
    assert!(facade.structured_lyrics[0].source.is_none());
    assert!(serde_json::to_value(&facade.structured_lyrics[0])
        .unwrap()
        .get("source")
        .is_none());

    // A track whose file is gone takes no upload: the read refuses it, and a
    // write the read would refuse is no write at all.
    sqlx::query("UPDATE track SET is_available = 0 WHERE id = ?")
        .bind(track.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let gone = send(
        Method::PUT,
        format!("{lyrics_uri}/lrc"),
        &owner_token,
        b"[00:01.00]too late".to_vec(),
    )
    .await;
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
    // The answer alone proves nothing: the lyrics read after the write says
    // 404 for this track whether or not the row went in.
    let stored: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM track_lyrics_upload WHERE track_id = ? AND content LIKE ?",
    )
    .bind(track.to_string())
    .bind("%too late%")
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(stored, 0);
}

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
            waveflow_server::database::LibraryRole::Listener,
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
            waveflow_server::database::LibraryRole::Listener,
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

/// Artist images chosen by hand (RFC-013 step 2), through the router: a chosen
/// portrait leads every reader and outlives the provider's fill, a banner
/// rides on the artist alone, the sweep keeps both, and only owners and
/// managers choose. The images are bytes with the right magic numbers: the
/// server recognises a format from the bytes and never decodes an image.
/// Passwords are drawn at run time, for CodeQL.
#[tokio::test]
async fn chosen_artist_images_lead_every_reader_and_outlive_the_portrait_fill() {
    let (_temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let mut accounts = Vec::new();
    for name in ["picture-owner", "picture-listener", "picture-stranger"] {
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
        .create_library(
            owner,
            "Pictures",
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
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 1, false).await.unwrap();
    let mut input = browse_input(50_000, "Hyperballad", "Post", "Björk", Some(1), Some(1));
    input.relative_path = "Björk/Post/01.flac".into();
    state
        .db
        .apply_catalog_track(library_id, scan_id, &input, None, false)
        .await
        .unwrap();
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    // What RFC-011's fill does: an artwork row from a provider, linked on the
    // artist's own column.
    let provider_portrait = |seed: &str| format!("{:0>64}", seed);
    let link_provider_portrait = |hash: String| {
        let pool = state.db.pool().clone();
        async move {
            sqlx::query(
                "INSERT INTO artwork (hash, format, source, byte_size, created_at) \
                 VALUES (?, 'jpg', 'deezer', 1, 0) ON CONFLICT (hash) DO NOTHING",
            )
            .bind(&hash)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("UPDATE artist SET artwork_hash = ? WHERE name = 'Björk'")
                .bind(&hash)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    let deezer = provider_portrait("d1");
    link_provider_portrait(deezer.clone()).await;

    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "picture-owner", &password).await;
    let listener_token = login_token(&router, "picture-listener", &password).await;
    let stranger_token = login_token(&router, "picture-stranger", &password).await;
    let send = |method: Method, uri: String, token: &str, body: Vec<u8>| {
        let router = router.clone();
        let token = token.to_owned();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    let artists =
        json_body(send(Method::GET, "/api/v2/artists".into(), &owner_token, vec![]).await).await;
    let artist = artists
        .as_array()
        .unwrap()
        .iter()
        .find(|artist| artist["name"] == "Björk")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let artist_uri = format!("/api/v2/artists/{artist}");
    let image_uri = |kind: &str| format!("{artist_uri}/image/{kind}");
    let png = |seed: u8| {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend(std::iter::repeat_n(seed, 64));
        bytes
    };
    let jpeg = |seed: u8| {
        let mut bytes = vec![0xff, 0xd8, 0xff];
        bytes.extend(std::iter::repeat_n(seed, 64));
        bytes
    };
    let hash_of = |bytes: &[u8]| blake3::hash(bytes).to_hex().to_string();
    let events = || {
        let pool = state.db.pool().clone();
        let artist = artist.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM library_event WHERE entity_type = 'artist' AND entity_id = ?",
            )
            .bind(artist)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    // Before any choice: the provider's portrait, no banner, nothing chosen.
    let before = json_body(send(Method::GET, artist_uri.clone(), &owner_token, vec![]).await).await;
    assert_eq!(before["artwork_hash"], deezer.as_str());
    assert!(omits(&before, "banner_hash"));
    assert!(omits(&before, "image_choice"));

    // A chosen portrait leads, says it was chosen, and tells clients so.
    let events_before = events().await;
    let portrait = png(1);
    let chosen = send(
        Method::PUT,
        image_uri("portrait"),
        &owner_token,
        portrait.clone(),
    )
    .await;
    assert_eq!(chosen.status(), StatusCode::OK);
    let chosen = json_body(chosen).await;
    assert_eq!(chosen["artwork_hash"], hash_of(&portrait).as_str());
    assert_eq!(chosen["image_choice"]["portrait"]["source"], "upload");
    assert!(omits(&chosen["image_choice"], "banner"));
    assert_eq!(events().await, events_before + 1);

    // A banner rides on the artist alone; the portrait stays.
    let banner = jpeg(2);
    let with_banner = json_body(
        send(
            Method::PUT,
            image_uri("banner"),
            &owner_token,
            banner.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(with_banner["banner_hash"], hash_of(&banner).as_str());
    assert_eq!(with_banner["artwork_hash"], hash_of(&portrait).as_str());

    // Every reader: the listing, the artist the Subsonic façade renders from,
    // and the artwork route by hash and by the artist's id.
    let listed = json_body(
        send(
            Method::GET,
            "/api/v2/artists".into(),
            &listener_token,
            vec![],
        )
        .await,
    )
    .await;
    let listed = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "Björk")
        .unwrap()
        .clone();
    assert_eq!(listed["artwork_hash"], hash_of(&portrait).as_str());
    let artist_id = uuid::Uuid::parse_str(&artist).unwrap();
    let detail = state.services.artist(owner, artist_id).await.unwrap();
    assert_eq!(
        detail.artist.artwork_hash.as_deref(),
        Some(hash_of(&portrait).as_str())
    );
    for (id, expected) in [(hash_of(&banner), &banner), (artist.clone(), &portrait)] {
        let served = send(
            Method::GET,
            format!("/api/v2/artwork/{id}"),
            &listener_token,
            vec![],
        )
        .await;
        assert_eq!(served.status(), StatusCode::OK, "{id}");
        let bytes = axum::body::to_bytes(served.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&bytes[..], &expected[..], "{id}");
    }
    assert_eq!(
        send(
            Method::GET,
            format!("/api/v2/artwork/{}", hash_of(&banner)),
            &stranger_token,
            vec![]
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    // The provider's fill rewrites the artist's own column; the choice holds.
    let refreshed = provider_portrait("d2");
    link_provider_portrait(refreshed.clone()).await;
    let after_fill =
        json_body(send(Method::GET, artist_uri.clone(), &owner_token, vec![]).await).await;
    assert_eq!(after_fill["artwork_hash"], hash_of(&portrait).as_str());

    // The sweep keeps what a choice names, row and file.
    state.services.sweep_artwork_store().await.unwrap();
    for image in [&portrait, &banner] {
        let named: Option<String> = sqlx::query_scalar("SELECT hash FROM artwork WHERE hash = ?")
            .bind(hash_of(image))
            .fetch_optional(state.db.pool())
            .await
            .unwrap();
        assert!(named.is_some());
    }
    let served = send(
        Method::GET,
        format!("/api/v2/artwork/{}", hash_of(&banner)),
        &owner_token,
        vec![],
    )
    .await;
    assert_eq!(served.status(), StatusCode::OK);

    // Only owners and managers choose; everyone else is answered as a stranger.
    for token in [&listener_token, &stranger_token] {
        assert_eq!(
            send(Method::PUT, image_uri("portrait"), token, png(3))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(Method::DELETE, image_uri("banner"), token, vec![])
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        send(Method::GET, artist_uri.clone(), &stranger_token, vec![])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // A refused upload leaves nothing in the store: access is asked before the
    // file is written, not only under the gate after it.
    assert!(!config
        .artwork_dir
        .join(format!("{}.png", hash_of(&png(3))))
        .exists());
    // Not an image, too large, or no such kind.
    assert_eq!(
        send(
            Method::PUT,
            image_uri("portrait"),
            &owner_token,
            b"not an image".to_vec()
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut huge = png(4);
    huge.resize(10 * 1024 * 1024 + 1, 0);
    assert_eq!(
        send(Method::PUT, image_uri("portrait"), &owner_token, huge)
            .await
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        send(Method::PUT, image_uri("cover"), &owner_token, png(5))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Exactly at the cap is still an image.
    let mut largest = png(6);
    largest.resize(10 * 1024 * 1024, 0);
    assert_eq!(
        send(Method::PUT, image_uri("banner"), &owner_token, largest)
            .await
            .status(),
        StatusCode::OK
    );

    // Back to automatic: the provider's latest portrait, and no banner.
    let events_before = events().await;
    for kind in ["portrait", "banner"] {
        assert_eq!(
            send(Method::DELETE, image_uri(kind), &owner_token, vec![])
                .await
                .status(),
            StatusCode::NO_CONTENT,
            "{kind}"
        );
    }
    assert_eq!(events().await, events_before + 2);
    let automatic =
        json_body(send(Method::GET, artist_uri.clone(), &owner_token, vec![]).await).await;
    assert_eq!(automatic["artwork_hash"], refreshed.as_str());
    assert!(omits(&automatic, "banner_hash"));
    assert!(omits(&automatic, "image_choice"));
    assert_eq!(
        send(Method::DELETE, image_uri("portrait"), &owner_token, vec![])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
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
    for _ in 0..200 {
        let job = send(bearer_json(
            "GET",
            &format!("/api/v2/scans/{scan_id}"),
            &owner_token,
            serde_json::json!({}),
        ))
        .await;
        if json_body(job).await["status"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
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
