//! Internet radio stations on the native API.

use super::*;

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
