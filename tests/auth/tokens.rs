//! API tokens: long-lived ones, the clients an account manages, and their
//! expiry.

use super::*;

#[tokio::test]
async fn long_lived_native_api_tokens_authenticate_and_honor_revocation() {
    let (_temp, config, state) = test_app().await;
    let now = now_ms();
    let password_hash =
        security::hash_password(&security::generate_token("test-password-")).unwrap();
    let user_id = state
        .db
        .create_account("native-token-user", &password_hash, AccountRole::User, now)
        .await
        .unwrap();
    let token = security::generate_token("wfapi_");
    state
        .db
        .create_api_token(
            waveflow_server::database::NewApiToken {
                // The actor the audit entry records: here the account itself.
                actor_id: user_id,
                user_id,
                name: "integration token",
                token_hash: &security::token_hash(&token),
                scopes: &[],
                expires_at: None,
            },
            now,
        )
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let accepted = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let last_used_at: Option<i64> =
        sqlx::query_scalar("SELECT last_used_at FROM api_token WHERE token_hash = ?")
            .bind(security::token_hash(&token).as_slice())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert!(last_used_at.is_some_and(|last_used| last_used >= now));

    assert!(state
        .db
        .revoke_api_token_by_hash(&security::token_hash(&token), now_ms())
        .await
        .unwrap());
    let revoked = router
        .oneshot(
            Request::get("/api/v2/albums")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
}

/// An account sees and revokes its own clients, and cannot mint a wider one.
///
/// Until now the only listing was `/api/v2/admin/users/{username}/tokens`, so
/// a listener could not find out which devices held a credential on their own
/// library, let alone stop one. The material was there and reachable only by
/// the instance administrator.
///
/// Opening it is safe for a reason worth asserting rather than asserting
/// about: `authenticated` checks the account role separately from the
/// credential scopes, so a scope narrows and never promotes. Both halves are
/// exercised below — a self-minted token is refused on an administrative route
/// however it was scoped, and a narrowed credential cannot hand on more than
/// it holds.
#[tokio::test]
async fn an_account_manages_its_own_clients_and_cannot_widen_one() {
    let (_temp, config, state) = test_app().await;
    let now = now_ms();
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("clients-user", &hash, AccountRole::User, now)
        .await
        .unwrap();
    let stranger = state
        .db
        .create_account("clients-other", &hash, AccountRole::User, now)
        .await
        .unwrap();
    // Another account credential: it must appear in no listing here, and must
    // not be revocable from this side.
    let theirs = state
        .db
        .create_api_token(
            waveflow_server::database::NewApiToken {
                // The actor the audit entry records.
                actor_id: stranger,
                user_id: stranger,
                name: "a phone belonging elsewhere",
                token_hash: &security::token_hash(&security::generate_token("wfapi_")),
                scopes: &[],
                expires_at: None,
            },
            now,
        )
        .await
        .unwrap();
    // A deliberately narrowed credential of this account own, which is what
    // the widening rule is about.
    let narrowed = security::generate_token("wfapi_");
    state
        .db
        .create_api_token(
            waveflow_server::database::NewApiToken {
                // The actor the audit entry records: here the account itself.
                actor_id: owner,
                user_id: owner,
                name: "a narrowed client",
                token_hash: &security::token_hash(&narrowed),
                scopes: &["write".to_owned()],
                expires_at: None,
            },
            now,
        )
        .await
        .unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let session = login_token(&router, "clients-user", &password).await;

    async fn listing(router: &axum::Router, bearer: &str) -> serde_json::Value {
        let response = router
            .clone()
            .oneshot(
                Request::get("/api/v2/api-tokens")
                    .header("authorization", format!("Bearer {bearer}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await
    }

    async fn mint(
        router: &axum::Router,
        bearer: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        let mut request = json_request("/api/v2/api-tokens", body);
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {bearer}").parse().unwrap());
        router.clone().oneshot(request).await.unwrap()
    }

    async fn drop_token(router: &axum::Router, bearer: &str, id: &str) -> axum::response::Response {
        router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/v2/api-tokens/{id}"))
                    .header("authorization", format!("Bearer {bearer}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    // 1. Its own, and **only** its own — from an ordinary account, in an
    //    instance with no administrator at all.
    let listed = listing(&router, &session).await;
    let names: Vec<&str> = listed
        .as_array()
        .expect("a list")
        .iter()
        .map(|token| token["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, ["a narrowed client"]);
    assert!(
        !listed.to_string().contains(&theirs.id.to_string()),
        "another account credential appeared in this account listing"
    );

    // 2. It can register one, and the secret is shown once.
    let created = mint(
        &router,
        &session,
        serde_json::json!({ "name": "my laptop" }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let minted = json_body(created).await;
    let secret = minted["secret"].as_str().expect("a secret").to_owned();
    assert!(secret.starts_with("wfapi_"));
    assert_eq!(
        listing(&router, &session).await.as_array().unwrap().len(),
        2
    );

    // 3. **A self-minted credential is not a promotion.** Unrestricted, from
    //    an ordinary account, it still cannot reach an administrative route:
    //    the role is checked apart from the scopes, which is the whole reason
    //    this surface can be opened at all.
    let administrative = router
        .clone()
        .oneshot(
            Request::get("/api/v2/admin/users/clients-user/tokens")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(administrative.status(), StatusCode::FORBIDDEN);

    // 4. **And a narrowed credential cannot hand on more than it holds.** An
    //    empty scope list means everything this account has, so a token
    //    restricted to `write` asking for one would walk out unrestricted —
    //    an escalation inside a single account.
    let widened = mint(
        &router,
        &narrowed,
        serde_json::json!({ "name": "a wider one" }),
    )
    .await;
    assert_eq!(widened.status(), StatusCode::FORBIDDEN);
    let promoted = mint(
        &router,
        &narrowed,
        serde_json::json!({ "name": "an admin one", "scopes": ["admin"] }),
    )
    .await;
    assert_eq!(promoted.status(), StatusCode::FORBIDDEN);
    // What it holds, it may pass on.
    let sideways = mint(
        &router,
        &narrowed,
        serde_json::json!({ "name": "an equal one", "scopes": ["write"] }),
    )
    .await;
    assert_eq!(sideways.status(), StatusCode::CREATED);

    // 5. Another account credential answers as a missing one. Not 403:
    //    naming it must not confirm that it exists.
    let foreign = drop_token(&router, &session, &theirs.id.to_string()).await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);

    // 6. And revoking its own stops that credential working, which is the
    //    whole point of showing the list.
    let own_id = minted["id"].as_str().expect("an id").to_owned();
    let revoked = drop_token(&router, &session, &own_id).await;
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    let refused = router
        .oneshot(
            Request::get("/api/v2/albums")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
}

/// What a self-minted credential cannot outlive, and what the sibling path
/// may not hand on either.
///
/// Both come from a security review of the branch that opened
/// `/api/v2/api-tokens`, and both are about a door that change created rather
/// than about the door itself.
#[tokio::test]
async fn a_self_minted_credential_expires_and_the_admin_path_is_bounded_too() {
    let (_temp, config, state) = test_app().await;
    let now = now_ms();
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    state
        .db
        .create_account("ttl-user", &hash, AccountRole::User, now)
        .await
        .unwrap();
    let boss = state
        .db
        .create_account("ttl-admin", &hash, AccountRole::Admin, now)
        .await
        .unwrap();
    // An administrator credential narrowed to exactly the scope that admits
    // the administrative routes, and nothing wider.
    let narrow_admin = security::generate_token("wfapi_");
    state
        .db
        .create_api_token(
            waveflow_server::database::NewApiToken {
                // The actor the audit entry records: here the account itself.
                actor_id: boss,
                user_id: boss,
                name: "a narrowed administrator",
                token_hash: &security::token_hash(&narrow_admin),
                scopes: &["admin".to_owned()],
                expires_at: None,
            },
            now,
        )
        .await
        .unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let session = login_token(&router, "ttl-user", &password).await;

    // 1. **A credential an account mints for itself does not outlive the
    //    policy.** `expires_at` has been a column, and honoured by the
    //    lookup, since the table existed, and nothing ever wrote it: before
    //    this, a session taken for the fifteen minutes of its access token
    //    could register a credential that outlived the theft by as long as
    //    nobody noticed.
    let mut request = json_request(
        "/api/v2/api-tokens",
        serde_json::json!({ "name": "a bounded client" }),
    );
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {session}").parse().unwrap(),
    );
    let created = router.clone().oneshot(request).await.unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let minted = json_body(created).await;
    let expires_at = minted["expires_at"]
        .as_i64()
        .expect("a self-minted credential carries an expiry");
    // Read off the configuration rather than written down again, so the
    // assertion follows the policy instead of pinning a year.
    let ttl = config
        .api_token_ttl
        .expect("the fixture bounds a self-minted credential")
        .as_millis() as i64;
    let created_at = minted["created_at"].as_i64().expect("a creation instant");
    assert!(
        (expires_at - created_at - ttl).abs() < 5_000,
        "expiry {expires_at} is not one lifetime after {created_at}"
    );

    // 2. **And the administrative path is bounded by its caller as well.**
    //    Nothing exploits its absence today — reaching that route needs the
    //    `admin` scope, which already admits every access level — but the
    //    asymmetry was the defect: the day a fourth scope exists, `admin`
    //    stops meaning everything and this path would have handed on more
    //    than its caller held, silently, while its twin refused.
    let mut widening = json_request(
        "/api/v2/admin/users/ttl-user/tokens",
        serde_json::json!({ "name": "a wider one" }),
    );
    widening.headers_mut().insert(
        "authorization",
        format!("Bearer {narrow_admin}").parse().unwrap(),
    );
    let refused = router.clone().oneshot(widening).await.unwrap();
    // 403, like its twin: nothing was looked up, so a 404 would say the
    // account just named does not exist.
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    // What it holds, it may still pass on — and the operator path stays
    // unbounded in time, which is its own contract and not an oversight.
    let mut equal = json_request(
        "/api/v2/admin/users/ttl-user/tokens",
        serde_json::json!({ "name": "an equal one", "scopes": ["admin"] }),
    );
    equal.headers_mut().insert(
        "authorization",
        format!("Bearer {narrow_admin}").parse().unwrap(),
    );
    let allowed = router.clone().oneshot(equal).await.unwrap();
    assert_eq!(allowed.status(), StatusCode::CREATED);
    assert!(
        json_body(allowed).await["expires_at"].is_null(),
        "the operator path grew an expiry it was never given"
    );
    // 3. **And it cannot outlive the credential that asked for it.** A token
    //    expiring tomorrow could mint one expiring in a year, and that one
    //    another, so the deadline above was defeatable by the very credential
    //    it bounded — decorative after a single hop.
    let tomorrow = now + 24 * 60 * 60 * 1000;
    let owner_of_ttl_user = state
        .db
        .account_by_username("ttl-user")
        .await
        .unwrap()
        .unwrap()
        .id;
    let short_lived = security::generate_token("wfapi_");
    state
        .db
        .create_api_token(
            waveflow_server::database::NewApiToken {
                // The actor the audit entry records: here the account itself.
                actor_id: owner_of_ttl_user,
                user_id: owner_of_ttl_user,
                name: "a client with a day left",
                token_hash: &security::token_hash(&short_lived),
                scopes: &[],
                expires_at: Some(tomorrow),
            },
            now,
        )
        .await
        .unwrap();
    let mut renewing = json_request(
        "/api/v2/api-tokens",
        serde_json::json!({ "name": "a renewal attempt" }),
    );
    renewing.headers_mut().insert(
        "authorization",
        format!("Bearer {short_lived}").parse().unwrap(),
    );
    let renewed = router.clone().oneshot(renewing).await.unwrap();
    assert_eq!(renewed.status(), StatusCode::CREATED);
    let inherited = json_body(renewed).await["expires_at"]
        .as_i64()
        .expect("a minted credential carries an expiry");
    assert_eq!(
        inherited, tomorrow,
        "the minted credential outlived the one that asked for it"
    );
    // The session, which has no deadline of this kind, still gets the policy:
    // capping by a session would make every client a person registers die in
    // fifteen minutes, which is not what a session is for.
    assert!(
        expires_at > tomorrow,
        "a session should hand on the policy lifetime, not a day"
    );
    // 4. **The bound is authority, not spelling.** An administrator credential
    //    narrowed to `admin` may mint a write-only client for a script: that
    //    is a narrowing, and `admin` admits `write`. Compared on strings it
    //    was refused — which fails closed and is still the wrong answer.
    let mut narrowing = json_request(
        "/api/v2/admin/users/ttl-user/tokens",
        serde_json::json!({ "name": "a write-only script", "scopes": ["write"] }),
    );
    narrowing.headers_mut().insert(
        "authorization",
        format!("Bearer {narrow_admin}").parse().unwrap(),
    );
    let narrower = router.oneshot(narrowing).await.unwrap();
    assert_eq!(narrower.status(), StatusCode::CREATED);

    // 5. **And registering one leaves a trace.** Revoking a credential wrote
    //    an audit entry and registering one wrote none, which is the wrong
    //    half to record: it became an act an account performs for itself with
    //    no administrator anywhere, so it is exactly the event an operator
    //    would go looking for afterwards. In the same transaction as the row
    //    it describes, so a token cannot exist without its entry.
    //
    //    Counted against the tokens themselves rather than against a number
    //    typed here: the claim is that every credential has an entry, and a
    //    literal would have to be recounted every time this test grows a
    //    fixture.
    let entries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_event WHERE kind = ?")
        .bind("api_token.created")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    let tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_token")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(tokens > 0, "no credential was registered at all");
    assert_eq!(entries, tokens, "a credential exists with no audit entry");
}
