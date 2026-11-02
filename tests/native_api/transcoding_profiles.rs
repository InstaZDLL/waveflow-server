//! The routes an account sets its transcoding profile through (RFC-027).

use super::*;

struct Accounts {
    _temp: tempfile::TempDir,
    state: waveflow_server::AppState,
    router: axum::Router,
    owner: uuid::Uuid,
    token: String,
    stranger_token: String,
    read_only_token: String,
}

impl Accounts {
    async fn new() -> Self {
        let (temp, config, state) = test_app().await;
        // Inserted rather than created, so no literal reaches a cryptographic
        // call: these accounts authenticate by API token.
        let owner = uuid::Uuid::new_v4();
        let stranger = uuid::Uuid::new_v4();
        for (id, username) in [(owner, "profile-owner"), (stranger, "profile-stranger")] {
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
        let (_record, token) = state
            .services
            .create_api_token(owner, &[], "profile-owner", "profiles", &[])
            .await
            .unwrap();
        let (_record, stranger_token) = state
            .services
            .create_api_token(stranger, &[], "profile-stranger", "profiles", &[])
            .await
            .unwrap();
        let (_record, read_only_token) = state
            .services
            .create_api_token(
                owner,
                &[],
                "profile-owner",
                "reader",
                &["catalog:read".to_owned()],
            )
            .await
            .unwrap();
        let router = waveflow_server::app(&config, state.clone());
        Self {
            _temp: temp,
            state,
            router,
            owner,
            token,
            stranger_token,
            read_only_token,
        }
    }

    async fn send(
        &self,
        token: &str,
        method: Method,
        query: &str,
        body: Option<serde_json::Value>,
    ) -> axum::response::Response {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/v2/transcoding-profiles{query}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json");
        let body = match body {
            Some(body) => Body::from(body.to_string()),
            None => Body::empty(),
        };
        self.router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap()
    }

    async fn put(&self, body: serde_json::Value) -> StatusCode {
        self.send(&self.token, Method::PUT, "", Some(body))
            .await
            .status()
    }

    async fn profiles(&self, token: &str) -> serde_json::Value {
        let response = self.send(token, Method::GET, "", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await["profiles"].clone()
    }
}

/// A row is replaced rather than patched, its bit rate defaults to the format's
/// and is stored as a number, the default sorts first, and a name differing only
/// in case is the same row.
#[tokio::test]
async fn a_profile_round_trips_and_is_replaced_whole() {
    let accounts = Accounts::new().await;
    assert_eq!(
        accounts.profiles(&accounts.token).await,
        serde_json::json!([])
    );

    let created = accounts
        .send(
            &accounts.token,
            Method::PUT,
            "",
            Some(serde_json::json!({"format": "opus"})),
        )
        .await;
    assert_eq!(created.status(), StatusCode::OK);
    let row = json_body(created).await;
    assert_eq!(row["client"], serde_json::Value::Null);
    assert_eq!(row["format"], "opus");
    assert_eq!(row["bitrate"], 128);

    assert_eq!(
        accounts
            .put(serde_json::json!({"client": " Symfonium ", "format": "mp3", "bitrate": 96}))
            .await,
        StatusCode::OK
    );
    assert_eq!(
        accounts
            .put(serde_json::json!({"client": "SYMFONIUM", "format": "raw"}))
            .await,
        StatusCode::OK
    );

    let rows = accounts.profiles(&accounts.token).await;
    assert_eq!(rows.as_array().unwrap().len(), 2, "{rows}");
    assert_eq!(rows[0]["client"], serde_json::Value::Null);
    assert_eq!(rows[1]["client"], "SYMFONIUM");
    assert_eq!(rows[1]["format"], "raw");
    assert_eq!(rows[1]["bitrate"], serde_json::Value::Null);
}

#[tokio::test]
async fn what_a_row_could_not_mean_is_refused() {
    let accounts = Accounts::new().await;
    let long = "x".repeat(121);
    for body in [
        // Not a format this server produces.
        serde_json::json!({"format": "flac"}),
        // On the default, `raw` says what having no default says.
        serde_json::json!({"format": "raw"}),
        serde_json::json!({"client": "Feishin", "format": "raw", "bitrate": 128}),
        serde_json::json!({"format": "opus", "bitrate": 31}),
        serde_json::json!({"format": "opus", "bitrate": 513}),
        // Blank is not the default, which it would otherwise overwrite.
        serde_json::json!({"client": "  ", "format": "opus"}),
        serde_json::json!({"client": long, "format": "opus"}),
    ] {
        assert_eq!(
            accounts.put(body.clone()).await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}"
        );
    }
    assert_eq!(
        accounts.profiles(&accounts.token).await,
        serde_json::json!([])
    );
}

/// A ceiling that refuses rather than grows, and that does not stop an account
/// at it from changing what it already has.
#[tokio::test]
async fn the_rows_of_an_account_are_bounded() {
    let accounts = Accounts::new().await;
    for index in 0..32 {
        assert_eq!(
            accounts
                .put(serde_json::json!({"client": format!("client-{index}"), "format": "opus"}))
                .await,
            StatusCode::OK,
            "{index}"
        );
    }
    assert_eq!(
        accounts
            .put(serde_json::json!({"client": "one-more", "format": "opus"}))
            .await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        accounts
            .put(serde_json::json!({"client": "client-0", "format": "mp3"}))
            .await,
        StatusCode::OK
    );
}

/// `204` whether or not the row was there, and only the row named.
#[tokio::test]
async fn deleting_removes_one_row_and_repeats_harmlessly() {
    let accounts = Accounts::new().await;
    accounts.put(serde_json::json!({"format": "opus"})).await;
    accounts
        .put(serde_json::json!({"client": "Symfonium", "format": "mp3"}))
        .await;

    for _ in 0..2 {
        let removed = accounts
            .send(&accounts.token, Method::DELETE, "?client=symfonium", None)
            .await;
        assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    }
    let rows = accounts.profiles(&accounts.token).await;
    assert_eq!(rows.as_array().unwrap().len(), 1, "{rows}");
    assert_eq!(rows[0]["client"], serde_json::Value::Null);

    let removed = accounts
        .send(&accounts.token, Method::DELETE, "", None)
        .await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        accounts.profiles(&accounts.token).await,
        serde_json::json!([])
    );
}

/// Every statement names the account: another account neither reads a row nor
/// removes it, and its names stay its own.
#[tokio::test]
async fn a_profile_stays_inside_its_account() {
    let accounts = Accounts::new().await;
    accounts.put(serde_json::json!({"format": "opus"})).await;
    accounts
        .state
        .services
        .stream_profile(accounts.owner, Some("Symfonium"), None)
        .await
        .unwrap();

    let theirs = accounts
        .send(&accounts.stranger_token, Method::GET, "", None)
        .await;
    assert_eq!(
        json_body(theirs).await,
        serde_json::json!({"profiles": [], "seen_clients": []})
    );

    let removed = accounts
        .send(&accounts.stranger_token, Method::DELETE, "", None)
        .await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        accounts
            .profiles(&accounts.token)
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

/// Setting a profile is a write, and reading one is not.
#[tokio::test]
async fn a_read_only_credential_reads_and_does_not_write() {
    let accounts = Accounts::new().await;
    assert_eq!(
        accounts.profiles(&accounts.read_only_token).await,
        serde_json::json!([])
    );
    let refused = accounts
        .send(
            &accounts.read_only_token,
            Method::PUT,
            "",
            Some(serde_json::json!({"format": "opus"})),
        )
        .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let refused = accounts
        .send(&accounts.read_only_token, Method::DELETE, "", None)
        .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
}

/// Deleting the account takes its profile with it.
#[tokio::test]
async fn a_deleted_account_leaves_no_row() {
    let accounts = Accounts::new().await;
    accounts.put(serde_json::json!({"format": "opus"})).await;
    sqlx::query("DELETE FROM account WHERE id=?")
        .bind(accounts.owner.to_string())
        .execute(accounts.state.db.pool())
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transcoding_profile")
        .fetch_one(accounts.state.db.pool())
        .await
        .unwrap();
    assert_eq!(left, 0);
}
