//! The probes, the OpenAPI document and the reference page, and the request id
//! the server mints for every request.

use super::*;

#[tokio::test]
async fn probes_and_openapi_are_available_without_scan_readiness() {
    let (_temp, mut config, state) = test_app().await;
    config.allowed_origins = vec!["http://127.0.0.1:9180".parse().unwrap()];
    let router = waveflow_server::app(&config, state);

    let health = router
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(json_body(health).await["schema"], 2);

    let ready = router
        .clone()
        .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    assert_eq!(json_body(ready).await["database"], "ok");

    let cors = router
        .clone()
        .oneshot(
            Request::get("/health")
                .header("origin", "http://127.0.0.1:9180")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        cors.headers()["access-control-allow-origin"],
        "http://127.0.0.1:9180"
    );
    let rejected = router
        .clone()
        .oneshot(
            Request::get("/health")
                .header("origin", "https://untrusted.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(rejected
        .headers()
        .get("access-control-allow-origin")
        .is_none());

    let openapi = router
        .oneshot(Request::get("/openapi.json").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(openapi.status(), StatusCode::OK);
    let document = json_body(openapi).await;
    assert!(document["paths"]["/api/v2/auth/login"].is_object());
    // RFC-017: both password sign-ins can answer 429, and say when to retry.
    for path in ["/api/v2/auth/login", "/api/v2/web/auth/login"] {
        let throttled = &document["paths"][path]["post"]["responses"]["429"];
        assert!(
            throttled["headers"]["Retry-After"].is_object(),
            "{path}: {throttled}"
        );
    }
    for path in [
        "/api/v2/setup",
        "/api/v2/web/auth/login",
        "/api/v2/libraries",
        "/api/v2/admin/users",
        "/api/v2/sync/snapshot",
        "/api/v2/sync/changes",
        "/api/v2/sync/ack",
        "/api/v2/sync/socket",
        "/api/v2/transcode/status",
        "/api/v2/tracks/{track_id}/lyrics",
        "/api/v2/libraries/{library_id}/uploads",
        "/api/v2/uploads/{session_id}",
        "/api/v2/uploads/{session_id}/chunks/{index}",
        "/api/v2/uploads/{session_id}/commit",
        "/api/v2/canvas/{canvas_hash}",
        "/api/v2/tracks/{track_id}/canvas",
        "/api/v2/tracks/{track_id}/canvas-ticket",
        "/api/v2/canvas-stream/{ticket}",
    ] {
        assert!(document["paths"][path].is_object(), "missing {path}");
    }

    // A track correction has three states per field — absent, `null`, a value —
    // and only the schema can tell a generated client that `null` is one of
    // them. A required field would forbid the absent state; a type without
    // `null` would forbid the removal. Either loses #177 in translation.
    let correction = &document["components"]["schemas"]["TrackMetadataPatch"];
    assert!(
        correction.get("required").is_none(),
        "no field of a partial patch may be required"
    );
    for field in [
        "title",
        "sort_title",
        "year",
        "track_number",
        "disc_number",
        "musicbrainz_recording_id",
        "comment",
        "artists",
        "genres",
    ] {
        assert!(
            correction["properties"][field]["type"]
                .as_array()
                .is_some_and(|types| types.iter().any(|kind| kind == "null")),
            "{field} must admit null"
        );
    }

    // A client generated from this document alone must authenticate correctly
    // and keep replay safety. Neither is inferable from the paths.
    assert_eq!(
        document["components"]["securitySchemes"]["bearer"]["scheme"],
        "bearer"
    );
    assert_eq!(
        document["security"][0]["bearer"].as_array().map(Vec::len),
        Some(0)
    );

    // Endpoints carrying their own credential, or none, must opt out of the
    // global requirement — otherwise the document claims a token is needed to
    // log in, and that the stream ticket is not itself the credential.
    for (path, method) in [
        ("/health", "get"),
        ("/api/v2/auth/login", "post"),
        ("/api/v2/oauth/token", "post"),
        ("/api/v2/stream/{ticket}", "get"),
        ("/api/v2/canvas-stream/{ticket}", "get"),
    ] {
        assert_eq!(
            document["paths"][path][method]["security"]
                .as_array()
                .map(Vec::len),
            Some(0),
            "{method} {path} should be documented as public"
        );
    }

    // The two mutation headers are what the native API offers over the Subsonic
    // facade; a document that omits them yields clients that never replay
    // safely.
    let headers = document["paths"]["/api/v2/scrobbles"]["post"]["parameters"]
        .as_array()
        .map(|parameters| {
            parameters
                .iter()
                .filter_map(|parameter| parameter["name"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        headers.iter().any(|name| name == "x-waveflow-operation-id"),
        "scrobbles should document the operation id header, got {headers:?}"
    );
    assert!(
        headers.iter().any(|name| name == "x-waveflow-device-id"),
        "scrobbles should document the device id header, got {headers:?}"
    );
    // Every user-data write can fail on a replayed operation id, so the 409 is
    // part of their contract — a client that only handles 2xx/4xx generically
    // would retry it forever.
    assert!(document["paths"]["/api/v2/scrobbles"]["post"]["responses"]["409"].is_object());
    // And the sync read has its own 409, with a different code and a different
    // recovery. Documenting one without the other would be worse than neither.
    assert!(document["paths"]["/api/v2/sync/changes"]["get"]["responses"]["409"].is_object());

    // Reads carry no operation id: annotating them would tell a generator to
    // send headers the handler never looks at.
    let read_parameters = document["paths"]["/api/v2/favorites"]["get"]["parameters"]
        .as_array()
        .map(|parameters| {
            parameters
                .iter()
                .filter_map(|parameter| parameter["name"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        !read_parameters
            .iter()
            .any(|name| name.starts_with("x-waveflow-")),
        "a read should not advertise mutation headers, got {read_parameters:?}"
    );
}

#[tokio::test]
async fn a_caller_cannot_name_its_own_request() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let named = |value: Option<&str>| {
        let router = router.clone();
        let value = value.map(str::to_owned);
        async move {
            let mut request = Request::get("/health");
            if let Some(value) = value.as_deref() {
                request = request.header("x-request-id", value);
            }
            // Named rather than `unwrap`ed because every value below has to
            // reach the router for its case to mean anything, and one of them
            // is not ASCII. `http` accepts it: a header value may hold any byte
            // from 0x20 up except 0x7f, so `café` builds — it is `to_str`, on
            // the way back, that would refuse it, which is why the answer is
            // read as bytes further down.
            let request = request
                .body(Body::empty())
                .expect("every proposed id has to reach the router");
            let response = router.oneshot(request).await.unwrap();
            // Read as bytes, not as `&str`. A `HeaderValue` may hold any byte
            // above 0x1f, `to_str` refuses the ones above 0x7f, and the point
            // of the non-ASCII case below is that such a value is not kept — so
            // reading it that way would panic instead of failing the assertion
            // that says so, and only when the drop is removed. A test that
            // crashes rather than fails proves nothing about the reason.
            response
                .headers()
                .get("x-request-id")
                .map(|value| value.as_bytes().to_vec())
        }
    };

    // Every request carries one, minted here, when the caller offered nothing.
    let unprompted = named(None).await.expect("a request is always named");
    assert!(uuid::Uuid::parse_str(&String::from_utf8_lossy(&unprompted)).is_ok());

    for proposed in [
        // The case a shape check would have let through, and the reason there
        // is no shape check: an id belonging to another caller's requests.
        "550e8400-e29b-41d4-a716-446655440000",
        // The one a proxy would plausibly mint, and equally not kept.
        "01JD5X-trace_id.7f",
        // Unbounded length.
        "x".repeat(65).as_str(),
        // A space, which reads as two fields wherever a trace is parsed.
        "two words",
        // Punctuation an identifier has no use for.
        "id;drop",
        // Non-ASCII, which a sink may re-encode.
        "identifiant-café",
        // Nothing at all, which would name a request the empty string.
        "",
    ] {
        let answered = named(Some(proposed))
            .await
            .expect("a request is named even when the caller proposed nothing usable");
        let shown = String::from_utf8_lossy(&answered);
        assert_ne!(
            answered.as_slice(),
            proposed.as_bytes(),
            "the server adopted a name the caller chose: {shown}"
        );
        // And what replaced it is a minted id, not an edited version of what
        // arrived: a sanitiser would still be letting the caller choose most
        // of it.
        assert!(
            uuid::Uuid::parse_str(&shown).is_ok(),
            "the name a request carries is one this server minted, and was: {shown}"
        );
        assert_ne!(
            answered, unprompted,
            "each request is named once, not named the same thing twice"
        );
    }

    // A browser reading a cross-origin response sees only the headers CORS
    // exposes, so the id being on the response is not the same as the client
    // being able to quote it. The API guide says it can; this is what makes
    // that true.
    let mut cors_config = config.clone();
    cors_config.allowed_origins = vec!["http://127.0.0.1:9180".parse().unwrap()];
    let cors_router = waveflow_server::app(&cors_config, state);
    let response = cors_router
        .clone()
        .oneshot(
            Request::get("/health")
                .header("origin", "http://127.0.0.1:9180")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let exposed = response.headers()["access-control-expose-headers"]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        exposed.split(',').any(|name| name.trim() == "x-request-id"),
        "a browser client cannot read an id it is not allowed to see: {exposed}"
    );

    // And a preflight is a response too. `CorsLayer` answers one itself without
    // calling the service beneath it, so for as long as the request-id layers
    // sat inside it this was the one request nobody could name — and an origin
    // this server rejects is precisely what an operator needs to find in a log.
    let response = cors_router
        .oneshot(
            Request::builder()
                .method(axum::http::Method::OPTIONS)
                .uri("/api/v2/albums")
                .header("origin", "http://127.0.0.1:9180")
                .header("access-control-request-method", "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let named = response
        .headers()
        .get("x-request-id")
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
        .expect("a preflight is answered by this server, so it is named by it");
    assert!(
        uuid::Uuid::parse_str(&named).is_ok(),
        "a preflight carries a minted id like any other response, and carried: {named}"
    );

    // Exposed on the way out and not allowed on the way in, which is the pair
    // the guide describes: a browser may read the name this server gave, and a
    // browser that sends one has its preflight refused rather than its value
    // quietly dropped. Adding it to `allow_headers` would make the guide wrong,
    // which is why this reads that list too.
    let allowed = response.headers()["access-control-allow-headers"]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        !allowed.split(',').any(|name| name.trim() == "x-request-id"),
        "a caller has no business sending this one: {allowed}"
    );
}

/// RFC-023 decision 3: `/reference` names no host but this one. The page loads
/// the Scalar bundle the binary carries, at a versioned address, under a policy
/// that admits scripts and connections from this origin only.
#[tokio::test]
async fn the_api_reference_is_served_from_the_binary() {
    use http_body_util::BodyExt;

    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state);
    let get = |uri: &'static str, encoding: Option<&'static str>| {
        let router = router.clone();
        async move {
            let mut request = Request::get(uri);
            if let Some(encoding) = encoding {
                request = request.header("accept-encoding", encoding);
            }
            router
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };

    let page = get("/reference", None).await;
    assert_eq!(page.status(), StatusCode::OK);
    let policy = page.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    for directive in [
        "default-src 'none'",
        "script-src 'self';",
        "connect-src 'self';",
        "frame-ancestors 'self'",
    ] {
        assert!(policy.contains(directive), "{directive} in {policy}");
    }
    assert!(!policy.contains("unsafe-eval"), "{policy}");
    assert_eq!(page.headers()["x-content-type-options"], "nosniff");
    let html = body_text(page).await;
    assert!(
        html.contains(&format!(
            "<script src=\"{}\"></script>",
            waveflow_server::http::reference::SCALAR_SCRIPT_PATH
        )),
        "{html}"
    );
    assert!(!html.contains("jsdelivr"), "no CDN");
    assert_eq!(html.matches("<script src=").count(), 1, "one script, ours");
    assert!(
        html.contains("/api/v2/auth/login"),
        "the document is in the page"
    );

    // Stored compressed and sent as stored to whoever takes gzip…
    let script = get(
        waveflow_server::http::reference::SCALAR_SCRIPT_PATH,
        Some("gzip, br"),
    )
    .await;
    assert_eq!(script.status(), StatusCode::OK);
    assert_eq!(script.headers()["content-encoding"], "gzip");
    assert!(script.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/javascript"));
    assert!(script.headers()["cache-control"]
        .to_str()
        .unwrap()
        .contains("immutable"));
    // …and decompressed for a client that takes none.
    let plain = get(waveflow_server::http::reference::SCALAR_SCRIPT_PATH, None).await;
    assert!(plain.headers().get("content-encoding").is_none());
    let bytes = plain.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        bytes,
        waveflow_server::http::reference::decompressed().unwrap()
    );
}
