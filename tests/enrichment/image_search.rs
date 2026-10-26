//! Searching for an artist image to pick: candidates, previews, deadlines and
//! the sweep.

use super::*;

/// A fake Deezer that offers, for any query, twenty-four artists with a
/// picture of their own each — and two it cannot show among them: one with
/// Deezer's placeholder, one whose picture is not an image.
#[derive(Clone)]
struct Gallery {
    searches: Arc<AtomicUsize>,
    images: Arc<AtomicUsize>,
    /// Pictures asked at any size but Deezer's big one.
    other_sizes: Arc<AtomicUsize>,
    /// A search waits here for a permit. A test that holds a search in
    /// flight starts it with none and adds them when it is done holding.
    gate: Arc<tokio::sync::Semaphore>,
    /// Every picture answers 503: a host that cannot be reached.
    failing: bool,
    origin: Arc<String>,
}

/// The fake's ids, in the order its search lists them.
const PLACEHOLDER_ID: i64 = 900;
const NOT_AN_IMAGE_ID: i64 = 901;

async fn gallery_search(State(gallery): State<Gallery>) -> Response {
    gallery.searches.fetch_add(1, Ordering::SeqCst);
    let _permit = gallery.gate.acquire().await.unwrap();
    let origin = &*gallery.origin;
    let mut data = Vec::new();
    for id in 1..=24_i64 {
        if id == 3 {
            data.push(serde_json::json!({
                "id": PLACEHOLDER_ID,
                "name": "Nobody",
                "picture_big": format!("{origin}/images/artist//500x500-000000-80-0-0.jpg"),
            }));
        }
        if id == 5 {
            data.push(serde_json::json!({
                "id": NOT_AN_IMAGE_ID,
                "name": "Broken",
                "picture_big": format!("{origin}/img/{NOT_AN_IMAGE_ID}/big.jpg"),
            }));
        }
        data.push(serde_json::json!({
            "id": id,
            "name": format!("Björk {id}"),
            "nb_fan": id * 1000,
            "picture_small": format!("{origin}/img/{id}/small.jpg"),
            "picture_big": format!("{origin}/img/{id}/big.jpg"),
            "picture_xl": format!("{origin}/img/{id}/xl.jpg"),
        }));
    }
    json(serde_json::json!({ "data": data }).to_string())
}

async fn gallery_picture(
    State(gallery): State<Gallery>,
    axum::extract::Path((id, size)): axum::extract::Path<(i64, String)>,
) -> Response {
    gallery.images.fetch_add(1, Ordering::SeqCst);
    if size != "big.jpg" {
        gallery.other_sizes.fetch_add(1, Ordering::SeqCst);
    }
    if gallery.failing {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let bytes = if id == NOT_AN_IMAGE_ID {
        b"no picture here".to_vec()
    } else {
        let mut bytes = portrait_bytes();
        bytes.extend_from_slice(format!("-{id}").as_bytes());
        bytes
    };
    ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response()
}

async fn fake_gallery(failing: bool, open: bool) -> (Gallery, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let gallery = Gallery {
        searches: Arc::default(),
        images: Arc::default(),
        other_sizes: Arc::default(),
        gate: Arc::new(tokio::sync::Semaphore::new(if open { 1_000 } else { 0 })),
        failing,
        origin: Arc::new(base_url.clone()),
    };
    let router = Router::new()
        .route("/search/artist", get(gallery_search))
        .route("/img/{id}/{size}", get(gallery_picture))
        .with_state(gallery.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (gallery, base_url)
}

fn gallery_engine(
    state: &waveflow_server::AppState,
    base_url: &str,
    settings: &MetadataSettings,
) -> Enrichment {
    Enrichment::with_endpoints(
        state.db.clone(),
        settings,
        state.artwork_dir.clone(),
        Endpoints {
            deezer: base_url.to_owned(),
            deezer_image_hosts: vec!["127.0.0.1".to_owned()],
            image_addresses: AddressPolicy::AllowLoopback,
            ..Endpoints::default()
        },
    )
    .unwrap()
}

/// Björk and Sigur Rós in a library of the owner's, and the services asking
/// `gallery` for portraits.
async fn picker(
    failing: bool,
    open: bool,
) -> (
    tempfile::TempDir,
    waveflow_server::Config,
    waveflow_server::AppState,
    Gallery,
    uuid::Uuid,
    uuid::Uuid,
) {
    let (temp, config, state) = support::test_app().await;
    let owner = account(&state, "picker-owner").await;
    library_of(&state, &config, owner, "picked", &["Björk", "Sigur Rós"]).await;
    let (gallery, base_url) = fake_gallery(failing, open).await;
    state.services.use_enrichment(gallery_engine(
        &state,
        &base_url,
        &settings(vec![Provider::Deezer], &run_key()),
    ));
    let bjork = artist_id(&state, owner, "Björk").await;
    (temp, config, state, gallery, owner, bjork)
}

async fn searches_held(state: &waveflow_server::AppState) -> Vec<(i64, Option<i64>)> {
    sqlx::query_as("SELECT complete, expires_at FROM artist_image_search ORDER BY searched_at")
        .fetch_all(state.db.pool())
        .await
        .unwrap()
}

/// Waits, a few seconds at most, for the fake to have been asked `count`
/// searches.
async fn until_searched(gallery: &Gallery, count: usize) {
    for _ in 0..500 {
        if gallery.searches.load(Ordering::SeqCst) >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the fake was never asked");
}

async fn send(
    router: &Router,
    method: axum::http::Method,
    uri: String,
    token: &str,
    body: Option<serde_json::Value>,
) -> Response {
    use tower::ServiceExt;
    let request = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string())),
        None => request.body(axum::body::Body::empty()),
    };
    router.clone().oneshot(request.unwrap()).await.unwrap()
}

/// RFC-013 decision 4 through the router: twenty candidates in Deezer's order,
/// each previewed from a picture the server fetched at the big size and holds,
/// and no picture URL; the same query answered again from the stored search;
/// the preview served to owners and managers only; a pick accepted only for an
/// id a live search of that artist offered; and both routes closed the moment
/// Deezer is. Passwords are drawn at run time, for CodeQL.
#[tokio::test]
async fn candidates_are_previewed_by_the_server_and_picked_from_a_live_search() {
    use axum::http::Method;
    use waveflow_server::database::{AccountRole, LibraryRole};

    let (_temp, config, state, gallery, owner, bjork) = picker(false, true).await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = waveflow_server::security::hash_password(&password).unwrap();
    let now = waveflow_server::authentication::now_ms();
    sqlx::query("UPDATE account SET username='gallery-owner', password_hash=? WHERE id=?")
        .bind(&hash)
        .bind(owner.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let listener = state
        .db
        .create_account("gallery-listener", &hash, AccountRole::User, now)
        .await
        .unwrap();
    state
        .db
        .create_account("gallery-stranger", &hash, AccountRole::User, now)
        .await
        .unwrap();
    let library: String = sqlx::query_scalar("SELECT id FROM library WHERE name='picked'")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner,
            library.parse().unwrap(),
            listener,
            LibraryRole::Listener,
            now,
        )
        .await
        .unwrap();
    let sigur = artist_id(&state, owner, "Sigur Rós").await;
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = support::login_token(&router, "gallery-owner", &password).await;
    let listener_token = support::login_token(&router, "gallery-listener", &password).await;
    let stranger_token = support::login_token(&router, "gallery-stranger", &password).await;
    let candidates_uri = |artist: uuid::Uuid, query: &str| {
        format!(
            "/api/v2/artists/{artist}/image/candidates?q={}",
            query.replace(' ', "%20")
        )
    };
    let pick_uri = |artist: uuid::Uuid| format!("/api/v2/artists/{artist}/image/portrait/deezer");

    let offered = send(
        &router,
        Method::GET,
        candidates_uri(bjork, "Björk"),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(offered.status(), StatusCode::OK);
    let offered = support::json_body(offered).await;
    let offered = offered.as_array().unwrap().clone();
    let ids: Vec<i64> = offered
        .iter()
        .map(|candidate| candidate["deezer_id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, (1..=20).collect::<Vec<_>>());
    assert_eq!(offered[0]["name"], "Björk 1");
    assert_eq!(offered[0]["fans"], 1000);
    for candidate in &offered {
        let mut keys: Vec<_> = candidate.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["artwork_hash", "deezer_id", "fans", "name"]);
    }
    // Twenty previews and the picture that was not an image; the placeholder
    // never asked; every picture at the big size.
    assert_eq!(gallery.searches.load(Ordering::SeqCst), 1);
    assert_eq!(gallery.images.load(Ordering::SeqCst), 21);
    assert_eq!(gallery.other_sizes.load(Ordering::SeqCst), 0);

    let preview = offered[0]["artwork_hash"].as_str().unwrap().to_owned();
    let artwork_uri = format!("/api/v2/artwork/{preview}");
    let served = send(
        &router,
        Method::GET,
        artwork_uri.clone(),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(served.status(), StatusCode::OK);
    let refused = send(&router, Method::GET, artwork_uri, &listener_token, None).await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);

    let again = send(
        &router,
        Method::GET,
        candidates_uri(bjork, " BJÖRK "),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(again.status(), StatusCode::OK);
    assert_eq!(
        support::json_body(again).await.as_array().unwrap(),
        &offered
    );
    assert_eq!(gallery.searches.load(Ordering::SeqCst), 1);
    assert_eq!(gallery.images.load(Ordering::SeqCst), 21);

    for token in [&listener_token, &stranger_token] {
        let refused = send(
            &router,
            Method::GET,
            candidates_uri(bjork, "Björk"),
            token,
            None,
        )
        .await;
        assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    }
    let empty = send(
        &router,
        Method::GET,
        candidates_uri(bjork, "  "),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let pick = |id: i64| Some(serde_json::json!({ "deezer_id": id }));
    // Offered, but for another artist; never offered; not the listener's.
    let elsewhere = send(&router, Method::PUT, pick_uri(sigur), &owner_token, pick(3)).await;
    assert_eq!(elsewhere.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let unoffered = send(
        &router,
        Method::PUT,
        pick_uri(bjork),
        &owner_token,
        pick(PLACEHOLDER_ID),
    )
    .await;
    assert_eq!(unoffered.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let listened = send(
        &router,
        Method::PUT,
        pick_uri(bjork),
        &listener_token,
        pick(3),
    )
    .await;
    assert_eq!(listened.status(), StatusCode::NOT_FOUND);

    let picked = send(&router, Method::PUT, pick_uri(bjork), &owner_token, pick(3)).await;
    assert_eq!(picked.status(), StatusCode::OK);
    let picked = support::json_body(picked).await;
    assert_eq!(picked["artwork_hash"], offered[2]["artwork_hash"]);
    assert_eq!(picked["image_choice"]["portrait"]["source"], "deezer");
    assert_eq!(gallery.images.load(Ordering::SeqCst), 21);

    // The operator's switch is read at every call: a live search notwithstanding.
    state.services.use_enrichment(Enrichment::disabled());
    let closed = send(
        &router,
        Method::GET,
        candidates_uri(bjork, "Björk"),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(closed.status(), StatusCode::NOT_FOUND);
    let closed = send(&router, Method::PUT, pick_uri(bjork), &owner_token, pick(4)).await;
    assert_eq!(closed.status(), StatusCode::NOT_FOUND);
}

/// One search at a time per account: a second while the first is fetching
/// answers 429, and the next one after it starts at once.
#[tokio::test]
async fn a_second_search_while_the_first_fetches_is_refused() {
    let (_temp, _config, state, gallery, owner, bjork) = picker(false, false).await;
    let services = state.services.clone();
    let first = tokio::spawn(async move {
        services
            .artist_image_candidates(owner, bjork, "Björk")
            .await
    });
    until_searched(&gallery, 1).await;

    let second = state
        .services
        .artist_image_candidates(owner, bjork, "Sigur Rós")
        .await;
    assert!(matches!(second, Err(ServiceError::Busy)), "{second:?}");

    gallery.gate.add_permits(1_000);
    assert_eq!(first.await.unwrap().unwrap().len(), 20);
    let third = state
        .services
        .artist_image_candidates(owner, bjork, "Sigur Rós")
        .await;
    assert!(third.is_ok(), "{third:?}");
}

/// The fetching is not the request's: a request dropped mid-search — a closed
/// tab — leaves the task to complete the search, and the next picker that
/// asks the same query is answered from it.
#[tokio::test]
async fn a_search_whose_request_is_dropped_still_completes() {
    let (_temp, _config, state, gallery, owner, bjork) = picker(false, false).await;
    let services = state.services.clone();
    let request = tokio::spawn(async move {
        services
            .artist_image_candidates(owner, bjork, "Björk")
            .await
    });
    until_searched(&gallery, 1).await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());

    gallery.gate.add_permits(1_000);
    let mut completed = false;
    for _ in 0..500 {
        if searches_held(&state).await.first().map(|search| search.0) == Some(1) {
            completed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(completed, "the search was left incomplete");

    let again = state
        .services
        .artist_image_candidates(owner, bjork, "björk")
        .await
        .unwrap();
    assert_eq!(again.len(), 20);
    assert_eq!(gallery.searches.load(Ordering::SeqCst), 1);
}

/// A search whose previews cannot be fetched is deleted by its own task, and
/// so does not hold the account's next search at 429.
#[tokio::test]
async fn a_failed_search_is_deleted_and_does_not_hold_the_account() {
    let (_temp, _config, state, gallery, owner, bjork) = picker(true, true).await;
    for _ in 0..2 {
        let failed = state
            .services
            .artist_image_candidates(owner, bjork, "Björk")
            .await;
        assert!(
            matches!(failed, Err(ServiceError::Unavailable)),
            "{failed:?}"
        );
        assert!(searches_held(&state).await.is_empty());
    }
    assert_eq!(gallery.searches.load(Ordering::SeqCst), 2);
}

/// The task runs under its deadline as a whole: a Deezer that never answers
/// fails the search at the deadline, not at the request timeout, and the
/// search is deleted. The deadline is cut to a second here and the request
/// timeout set to a minute, so which of the two stopped it shows in the time
/// the search took.
#[tokio::test]
async fn a_search_stops_at_its_deadline() {
    let (_temp, _config, state, gallery, owner, bjork) = picker(false, false).await;
    let patient = MetadataSettings {
        request_timeout: Duration::from_secs(60),
        ..settings(vec![Provider::Deezer], &run_key())
    };
    state
        .services
        .use_enrichment(gallery_engine(&state, gallery.origin.as_str(), &patient));
    state
        .services
        .shorten_image_search_deadline(Duration::from_secs(1));
    let started = std::time::Instant::now();
    let stopped = state
        .services
        .artist_image_candidates(owner, bjork, "Björk")
        .await;
    let waited = started.elapsed();
    assert!(
        matches!(stopped, Err(ServiceError::Unavailable)),
        "{stopped:?}"
    );
    assert!(waited < Duration::from_secs(30), "stopped after {waited:?}");
    assert!(searches_held(&state).await.is_empty());
}

/// An incomplete search past its deadline has no task behind it: it holds
/// nobody at 429, and the sweep deletes it — while a search still fetching,
/// and the previews of a live one, are left alone. Once the live search
/// expires, its previews go with the sweep, all but the one chosen.
#[tokio::test]
async fn the_sweep_takes_spent_searches_and_their_previews_only() {
    let (_temp, _config, state, _gallery, owner, bjork) = picker(false, true).await;
    let now = waveflow_server::authentication::now_ms();
    let stale = uuid::Uuid::new_v4().to_string();
    let fetching = uuid::Uuid::new_v4().to_string();
    for (id, by, at) in [
        (&stale, owner.to_string(), now - 6 * 60 * 1000),
        (&fetching, uuid::Uuid::new_v4().to_string(), now),
    ] {
        sqlx::query(
            "INSERT INTO artist_image_search (id, artist_id, query_key, searched_by, searched_at) \
             VALUES (?, ?, 'stale', ?, ?)",
        )
        .bind(id)
        .bind(bjork.to_string())
        .bind(by)
        .bind(at)
        .execute(state.db.pool())
        .await
        .unwrap();
    }

    let offered = state
        .services
        .artist_image_candidates(owner, bjork, "Björk")
        .await
        .expect("a search past its deadline holds nobody");
    let previews: Vec<String> = offered
        .iter()
        .map(|candidate| candidate.artwork_hash.clone())
        .collect();
    let held = |state: &waveflow_server::AppState, previews: Vec<String>| {
        let pool = state.db.pool().clone();
        async move {
            let mut count = 0;
            for hash in previews {
                let found: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork WHERE hash=?")
                    .bind(hash)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                count += found;
            }
            count
        }
    };

    state.services.sweep_artwork_store().await.unwrap();
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM artist_image_search WHERE complete=0")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    assert_eq!(ids, std::slice::from_ref(&fetching));
    assert_eq!(held(&state, previews.clone()).await, 20);

    state
        .services
        .choose_deezer_portrait(owner, bjork, 2)
        .await
        .unwrap();
    sqlx::query("UPDATE artist_image_search SET expires_at=? WHERE complete=1")
        .bind(now - 1)
        .execute(state.db.pool())
        .await
        .unwrap();
    let late = state.services.choose_deezer_portrait(owner, bjork, 3).await;
    assert!(matches!(late, Err(ServiceError::Invalid)), "{late:?}");

    state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(searches_held(&state).await, [(0, None)]);
    assert_eq!(held(&state, previews.clone()).await, 1);
    assert_eq!(
        held(&state, vec![offered[1].artwork_hash.clone()]).await,
        1,
        "the chosen preview went with the search"
    );
}

/// Nothing can be fetching for a search across a restart: boot deletes every
/// incomplete one and keeps the complete ones.
#[tokio::test]
async fn boot_deletes_the_searches_left_fetching() {
    let (_temp, config, state, _gallery, owner, bjork) = picker(false, true).await;
    let now = waveflow_server::authentication::now_ms();
    for (complete, expires_at) in [(0, None), (1, Some(now + 60 * 60 * 1000))] {
        sqlx::query(
            "INSERT INTO artist_image_search \
               (id, artist_id, query_key, searched_by, searched_at, expires_at, complete) \
             VALUES (?, ?, 'björk', ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(bjork.to_string())
        .bind(owner.to_string())
        .bind(now)
        .bind(expires_at)
        .bind(complete)
        .execute(state.db.pool())
        .await
        .unwrap();
    }

    waveflow_server::initialize(&config).await.unwrap();

    let held = searches_held(&state).await;
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].0, 1);
}

/// Every request to Deezer keeps enrichment's pace, the previews as much as
/// the search: at twenty a second, a search and its twenty-one picture
/// requests cannot take less than a second.
#[tokio::test]
async fn previews_keep_deezer_s_pace() {
    let (_temp, _config, state, gallery, owner, bjork) = picker(false, true).await;
    let paced = MetadataSettings {
        rate_per_sec: 20,
        ..settings(vec![Provider::Deezer], &run_key())
    };
    state
        .services
        .use_enrichment(gallery_engine(&state, gallery.origin.as_str(), &paced));
    let started = std::time::Instant::now();
    state
        .services
        .artist_image_candidates(owner, bjork, "Björk")
        .await
        .unwrap();
    assert_eq!(gallery.images.load(Ordering::SeqCst), 21);
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "twenty-two requests in {:?}",
        started.elapsed()
    );
}

/// A search outlives the ordinary request timeout: the route keeps a timeout
/// of its own past the search's deadline, so a slow Deezer answers the
/// request with what the task found, not a 408 that sends the retry into a
/// 429. The ordinary timeout is cut to a second here, and the search held for
/// two. Passwords are drawn at run time, for CodeQL.
#[tokio::test]
async fn a_search_outlives_the_ordinary_request_timeout() {
    let (_temp, config, state, gallery, owner, bjork) = picker(false, false).await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = waveflow_server::security::hash_password(&password).unwrap();
    sqlx::query("UPDATE account SET username='slow-owner', password_hash=? WHERE id=?")
        .bind(&hash)
        .bind(owner.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    // Logged in under the ordinary timeout: hashing the password can take
    // longer than a second on a loaded machine, and that is not what is tested.
    let token = support::login_token(
        &waveflow_server::app(&config, state.clone()),
        "slow-owner",
        &password,
    )
    .await;
    let impatient = waveflow_server::Config {
        request_timeout: Duration::from_secs(1),
        ..config.clone()
    };
    let router = waveflow_server::app(&impatient, state.clone());

    let search = {
        let router = router.clone();
        tokio::spawn(async move {
            send(
                &router,
                axum::http::Method::GET,
                format!("/api/v2/artists/{bjork}/image/candidates?q=Bj%C3%B6rk"),
                &token,
                None,
            )
            .await
        })
    };
    until_searched(&gallery, 1).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    gallery.gate.add_permits(1_000);

    let answered = search.await.unwrap();
    assert_eq!(answered.status(), StatusCode::OK);
    assert_eq!(
        support::json_body(answered).await.as_array().unwrap().len(),
        20
    );
}
