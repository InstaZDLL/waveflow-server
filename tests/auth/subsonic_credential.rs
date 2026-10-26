//! The dedicated Subsonic credential an account rotates or switches off.

use super::*;

/// Pings the facade with a key and answers whether it was refused.
///
/// `subsonic_json` asserts success, and half of what this test proves is a key
/// that has **stopped** working — so the refusal needs reading rather than
/// asserting away. Error code 40 inside an HTTP 200 is how the contract says
/// it.
async fn facade_refuses(router: &axum::Router, api_key: &str) -> bool {
    let response = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/ping.view?apiKey={api_key}&v=1.16.1&c=golden&f=json"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await["subsonic-response"]["error"]["code"] == 40
}

/// An account rotates and switches off its own Subsonic credential, and no
/// credential hands on more than it holds doing it.
///
/// The dedicated Subsonic password is the key to the facade — both doors join
/// `subsonic_credential` — and it is typed into third-party clients on phones,
/// which is the ordinary way an application password leaks. Until this, its
/// owner could neither change it nor stop it: both services called
/// `require_admin`, the only routes were administrative, and the CLI is a
/// shell on the host. So an account could not answer "my key has leaked".
#[tokio::test]
async fn an_account_rotates_or_switches_off_its_own_subsonic_credential() {
    let (_temp, config, state) = test_app().await;
    let now = now_ms();
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let listener = state
        .db
        .create_account("facade-listener", &hash, AccountRole::User, now)
        .await
        .unwrap();
    let boss = state
        .db
        .create_account("facade-admin", &hash, AccountRole::Admin, now)
        .await
        .unwrap();

    // A credential the operator granted, which is the only way one comes into
    // being — and the state this whole route set presupposes.
    let first_key = security::generate_token("wfsk_");
    state
        .db
        .set_subsonic_credential(
            boss,
            listener,
            &state
                .secret_box
                .encrypt(b"the password the operator handed over")
                .unwrap(),
            &security::token_hash(&first_key),
            now,
        )
        .await
        .unwrap();

    // One credential narrowed to the scope admitting every mutation, and one
    // unrestricted but carrying a deadline. Each is refused below, for a
    // different currency.
    let scoped = security::generate_token("wfapi_");
    state
        .db
        .create_api_token(
            waveflow_server::storage::database::NewApiToken {
                actor_id: listener,
                user_id: listener,
                name: "a narrowed client",
                token_hash: &security::token_hash(&scoped),
                scopes: &["write".to_owned()],
                expires_at: None,
            },
            now,
        )
        .await
        .unwrap();
    let expiring = security::generate_token("wfapi_");
    state
        .db
        .create_api_token(
            waveflow_server::storage::database::NewApiToken {
                actor_id: listener,
                user_id: listener,
                name: "a client with a deadline",
                token_hash: &security::token_hash(&expiring),
                scopes: &[],
                expires_at: Some(now + 86_400_000),
            },
            now,
        )
        .await
        .unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let session = login_token(&router, "facade-listener", &password).await;
    let bearer = |token: &str| format!("Bearer {token}");

    let read = |token: String| {
        let router = router.clone();
        async move {
            let request = Request::get("/api/v2/subsonic-credential")
                .header("authorization", token)
                .body(Body::empty())
                .unwrap();
            let response = router.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            body_text(response).await
        }
    };

    // 1. **The account can see that it holds one, and the password is not in
    //    the answer.** The query behind it selects two timestamps and never
    //    the columns holding a secret, so there is nothing for a careless edit
    //    of the handler to forward.
    let shown = read(bearer(&session)).await;
    let before: serde_json::Value = serde_json::from_str(&shown).unwrap();
    let created_at = before["created_at"]
        .as_i64()
        .expect("a credential the account holds reports when it was set");
    assert!(
        !shown.contains("password") && !shown.contains("the password the operator handed over"),
        "the status carried something about the password: {shown}"
    );

    // 2. The key works before the rotation, or nothing below means anything.
    assert!(!facade_refuses(&router, &first_key).await);

    // 3. **Rotation replaces the key as well as the password.** A leaked
    //    `wfsk_` key authenticates on its own, so rotating one half would
    //    answer "my key has leaked" with a new password and the same working
    //    key.
    let mut rotate = json_request(
        "/api/v2/subsonic-credential",
        serde_json::json!({ "password": "a replacement long enough" }),
    );
    *rotate.method_mut() = Method::PUT;
    rotate
        .headers_mut()
        .insert("authorization", bearer(&session).parse().unwrap());
    let rotated = router.clone().oneshot(rotate).await.unwrap();
    assert_eq!(rotated.status(), StatusCode::OK);
    let second_key = json_body(rotated).await["api_key"]
        .as_str()
        .expect("a rotation returns the new key once")
        .to_owned();
    assert!(second_key.starts_with("wfsk_"));
    assert_ne!(second_key, first_key);
    assert!(
        facade_refuses(&router, &first_key).await,
        "the leaked key still authenticates after a rotation"
    );
    assert!(!facade_refuses(&router, &second_key).await);
    // The same credential, moved — not a second one beside the first.
    let after: serde_json::Value = serde_json::from_str(&read(bearer(&session)).await).unwrap();
    assert_eq!(after["created_at"].as_i64(), Some(created_at));
    assert!(after["updated_at"].as_i64() >= Some(created_at));

    // 4. **A scoped credential is refused.** A `wfsk_` key carries no scopes,
    //    because the facade reads none, so it cannot be narrowed to match its
    //    caller the way an API token is. Handing one to a token restricted to
    //    `write` would let it walk out unrestricted on the facade.
    for (token, why) in [
        (&scoped, "a scoped credential"),
        // 5. **And so is an expiring one** — the same escalation in the other
        //    currency. `create_own_api_token` answers that with
        //    `min(policy, caller)`; there is no `min` to take on a key that
        //    carries no deadline at all, so a refusal is the only honest
        //    answer.
        (&expiring, "an expiring credential"),
    ] {
        let mut widening = json_request(
            "/api/v2/subsonic-credential",
            serde_json::json!({ "password": "a password it may not set" }),
        );
        *widening.method_mut() = Method::PUT;
        widening
            .headers_mut()
            .insert("authorization", bearer(token).parse().unwrap());
        let refused = router.clone().oneshot(widening).await.unwrap();
        // 403 and not the blanket 404: nothing was looked up, and a 404 would
        // tell the caller its own credential does not exist.
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{why} was allowed");
    }
    // Refused means nothing moved, not merely that the answer was 403.
    assert!(!facade_refuses(&router, &second_key).await);

    // 6. **Switching it off is bounded by nothing more**, because it hands on
    //    no authority — it takes some away. A narrowed credential closing a
    //    door its own account holds is the one act always safe to allow, and
    //    refusing it would leave a compromised account unable to do anything
    //    with the credential it has in hand.
    let revoke = Request::delete("/api/v2/subsonic-credential")
        .header("authorization", bearer(&scoped))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.clone().oneshot(revoke).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert!(facade_refuses(&router, &second_key).await);
    assert!(state
        .services
        .credential_by_username("facade-listener")
        .await
        .unwrap()
        .is_none());
    assert_eq!(read(bearer(&session)).await.trim(), "null");

    // A revocation that removed nothing leaves no trace saying it did. The
    // audit entry used to be written before the result was looked at, so the
    // 404 below came with a `subsonic.credential_revoked` line in the
    // history. That was cosmetic while an administrator was the only caller;
    // this route makes it reachable by any account, in a loop, which turns a
    // wrong line in a history into a way to fill one.
    let revocations = || async {
        sqlx::query(
            "SELECT COUNT(*) AS n FROM audit_event WHERE kind = 'subsonic.credential_revoked'",
        )
        .fetch_one(state.db.pool())
        .await
        .unwrap()
        .get::<i64, _>("n")
    };
    let recorded = revocations().await;
    let again = Request::delete("/api/v2/subsonic-credential")
        .header("authorization", bearer(&session))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.clone().oneshot(again).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        revocations().await,
        recorded,
        "a revocation that removed nothing was recorded as one"
    );

    // 7. **And the rotation cannot bring one back.** This is the whole design
    //    rather than a detail: an account holding no credential cannot reach
    //    the facade at all, so creating one opens a door instead of narrowing
    //    one already open, and whether every account may open that door onto
    //    the server is the operator's decision. The service does it with an
    //    `UPDATE` where its administrative twin upserts, so the refusal lives
    //    in the statement and two callers cannot interleave past it.
    let mut recreate = json_request(
        "/api/v2/subsonic-credential",
        serde_json::json!({ "password": "a password opening a door" }),
    );
    *recreate.method_mut() = Method::PUT;
    recreate
        .headers_mut()
        .insert("authorization", bearer(&session).parse().unwrap());
    let refused = router.clone().oneshot(recreate).await.unwrap();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    // The refusal created nothing: the facade is still shut to this account,
    // which is exactly what a 404 that quietly upserted would not have left
    // behind.
    assert!(state
        .services
        .credential_by_username("facade-listener")
        .await
        .unwrap()
        .is_none());
}
