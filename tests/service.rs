//! Probes, the embedded client, the backup bundle, and how a request is named.
//!
//! Split out of `v2_foundations.rs`; the command-line tests left for `cli.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::storage::database::AccountRole;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

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

/// The files that make the client installable, and the brand's images, are
/// served as themselves, or answered a JSON 404 — never the shell (RFC-022
/// decision 4).
///
/// Which of the two is required depends on what is embedded, and the test
/// asks the binary rather than the source tree: `build.rs` embeds the real
/// client when `webapp/dist` holds one and a placeholder page otherwise, and
/// the shell at `/` says which. With the real client, every install asset must
/// be served with its own type — a build that dropped one would not be
/// installable, and accepting a 404 there would let it pass. With the
/// placeholder, each must be a definitive JSON 404. Neither may ever be
/// `text/html`, which is what the router fallback gives any path it does not
/// know, and which a browser reading a manifest fails on without saying why.
#[tokio::test]
async fn the_install_manifest_and_icons_are_never_answered_with_the_shell() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state);
    let get = |path: &'static str| {
        let router = router.clone();
        async move {
            router
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };
    let shell = body_text(get("/").await).await;
    // The placeholder's own sentence, in `build.rs`.
    let built = !shell.contains("The web client is not built");

    for (path, expected) in [
        ("/manifest.webmanifest", "application/manifest+json"),
        ("/apple-touch-icon.png", "image/png"),
        ("/icons/icon-192.png", "image/png"),
        ("/icons/icon-512.png", "image/png"),
        ("/icons/maskable-512.png", "image/png"),
        // The two favicons `index.html` declares, and the logo the sidebar
        // and the first-load screen draw: all from the logo kit, all images.
        ("/favicon.ico", "image/x-icon"),
        ("/favicon.svg", "image/svg+xml"),
        ("/brand/waveflow-logo-horizontal-color.svg", "image/svg+xml"),
        ("/brand/waveflow-logo-horizontal-dark.svg", "image/svg+xml"),
        ("/brand/waveflow-symbol-mint.svg", "image/svg+xml"),
        // The service worker (RFC-022): a script, never the shell, and never
        // frozen in a cache, or a deploy could not replace it.
        ("/sw.js", "javascript"),
    ] {
        let response = get(path).await;
        let status = response.status();
        let kind = response
            .headers()
            .get("content-type")
            .map(|value| value.to_str().unwrap().to_owned())
            .unwrap_or_default();
        assert!(!kind.starts_with("text/html"), "{path} answered the shell");
        if built {
            assert_eq!(status, StatusCode::OK, "{path} is missing from the build");
            if path == "/sw.js" {
                assert!(kind.contains(expected), "{path}: {kind}");
                assert_eq!(response.headers()["cache-control"], "no-cache");
                continue;
            }
            assert_eq!(kind, expected, "{path}");
            if path == "/manifest.webmanifest" {
                let manifest: serde_json::Value =
                    serde_json::from_str(&body_text(response).await).unwrap();
                assert_eq!(manifest["start_url"], "/");
                assert_eq!(manifest["display"], "standalone");
                assert!(manifest["icons"]
                    .as_array()
                    .is_some_and(|icons| icons.iter().any(|icon| icon["purpose"] == "maskable")));
                // The brand's Wave green, as the logo kit's own manifest sets
                // both.
                assert_eq!(manifest["theme_color"], "#10B981");
                assert_eq!(manifest["background_color"], "#10B981");
            }
        } else {
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert!(kind.starts_with("application/json"), "{path}");
        }
    }

    // A name no build ships is a JSON 404 whatever is embedded: the case that
    // still tells the asset-only rule from the fallback when a real client is
    // embedded and every shipped icon exists.
    for path in ["/icons/not-shipped.png", "/brand/not-shipped.svg"] {
        let missing = get(path).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND, "{path}");
        assert!(
            missing
                .headers()
                .get("content-type")
                .map(|value| value.to_str().unwrap().to_owned())
                .unwrap_or_default()
                .starts_with("application/json"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn embedded_web_client_serves_shell_without_shadowing_the_api() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state);

    let get = |uri: &'static str| {
        let router = router.clone();
        async move {
            router
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };

    // The shell is served at the root.
    let root = get("/").await;
    assert_eq!(root.status(), StatusCode::OK);
    assert!(root.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    // The shell must never be cached: it is what points at the hashed assets.
    assert_eq!(root.headers()["cache-control"], "no-cache");

    // Client-side routes resolve to the same shell rather than 404.
    let deep = get("/albums/some-client-route").await;
    assert_eq!(deep.status(), StatusCode::OK);
    assert!(deep.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));

    // A route parameter carrying an encoded slash still resolves to the shell.
    // Genres are the case that raises it — "Rock/Pop" is one tag, and the
    // client routes it as /genres/Rock%2FPop — so a link to one, shared or
    // reloaded, must not decode into a two-segment path that matches nothing.
    let encoded = get("/genres/Rock%2FPop").await;
    assert_eq!(encoded.status(), StatusCode::OK);
    assert!(encoded.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));

    // An unknown API path stays a JSON 404 instead of silently returning HTML.
    let missing_api = get("/api/v2/does-not-exist").await;
    assert_eq!(missing_api.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        missing_api.headers()["content-type"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap(),
        "application/json"
    );

    // The other server namespaces answer as themselves rather than falling
    // through. /rest keeps its Subsonic contract — HTTP 200 with the failure in
    // the body — and /share is a plain 404. Neither may return the client shell.
    let unknown_method = get("/rest/nope").await;
    assert_eq!(unknown_method.status(), StatusCode::OK);
    assert!(!unknown_method.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    let unknown_method = body_text(unknown_method).await;
    assert!(unknown_method.starts_with("<subsonic-response"));
    assert!(unknown_method.contains("status=\"failed\""));

    let unknown_share = get("/share/nope").await;
    assert_eq!(unknown_share.status(), StatusCode::NOT_FOUND);
    assert!(!unknown_share
        .headers()
        .get("content-type")
        .map(|value| value.to_str().unwrap().to_owned())
        .unwrap_or_default()
        .starts_with("text/html"));

    // An icon request is answered once, or not at all — never with the shell.
    //
    // Before a build shipped a `favicon.ico`, the fallback handed the browser
    // the client page. It cannot decode HTML as an image, learns nothing from
    // a 200, and asks again: sixty-six requests in fifty-one seconds on a real
    // session, forty kilobytes of markup spent on a tab icon. The real client
    // ships the logo kit's icon now, and the placeholder ships none, so the
    // answer is the icon or a definitive 404 — whichever this binary embeds.
    //
    // Stated as what it *is* rather than as what it is not: "not HTML" is also
    // satisfied by a header that never arrived, so it would pin nothing about
    // the answer actually given.
    let built = !body_text(get("/").await)
        .await
        .contains("The web client is not built");
    let icon = get("/favicon.ico").await;
    let kind = icon
        .headers()
        .get("content-type")
        .map(|value| value.to_str().unwrap().to_owned())
        .unwrap_or_default();
    if built {
        assert_eq!(icon.status(), StatusCode::OK);
        assert_eq!(kind, "image/x-icon");
    } else {
        assert_eq!(icon.status(), StatusCode::NOT_FOUND);
        assert!(kind.starts_with("application/json"), "{kind}");
    }

    // A client route that merely starts like a reserved endpoint is not one.
    let lookalike = get("/reference-guide").await;
    assert_eq!(lookalike.status(), StatusCode::OK);
    assert!(lookalike.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));

    // Real routes are untouched by the fallback.
    let health = get("/health").await;
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(json_body(health).await["status"], "ok");
}

/// RFC-019 decision 2: the web client and the public shares tell a browser
/// how far to trust them, and the Subsonic façade, whose wire is frozen, is
/// left exactly as it was.
#[tokio::test]
async fn the_web_client_and_shares_carry_security_headers_and_rest_does_not() {
    use base64::Engine;
    use sha2::Digest;

    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state);
    let get = |uri: &'static str| {
        let router = router.clone();
        async move {
            router
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };
    let header = |response: &axum::response::Response, name: &str| {
        response
            .headers()
            .get(name)
            .map(|value| value.to_str().unwrap().to_owned())
    };

    // The shell, a client route, and a refusal from the fallback alike.
    for uri in ["/", "/albums/some-client-route", "/icons/not-shipped.png"] {
        let response = get(uri).await;
        assert_eq!(
            header(&response, "x-content-type-options").as_deref(),
            Some("nosniff"),
            "{uri}"
        );
        // Nothing to another host; not `no-referrer`, which adds nothing to
        // that and can null the `Origin` the sign-in checks (RFC-019).
        assert_eq!(
            header(&response, "referrer-policy").as_deref(),
            Some("same-origin"),
            "{uri}"
        );
        assert_eq!(
            header(&response, "x-frame-options").as_deref(),
            Some("SAMEORIGIN"),
            "{uri}"
        );
        let policy = header(&response, "content-security-policy").expect(uri);
        assert!(policy.contains("frame-ancestors 'self'"), "{uri}: {policy}");
        assert!(policy.contains("object-src 'none'"), "{uri}: {policy}");
        assert!(!policy.contains("unsafe-inline"), "{uri}: {policy}");
    }

    // Each inline block of the shell actually served is admitted by its own
    // digest — computed here, apart from the server's parser — and the policy
    // admits no more of them than the shell holds. With the placeholder shell
    // CI embeds there are none; with a real client build there are two.
    let shell = get("/").await;
    let policy = header(&shell, "content-security-policy").unwrap();
    let html = body_text(shell).await;
    let mut inline = 0;
    for tag in ["script", "style"] {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        for (start, _) in html.match_indices(&open) {
            let content = &html[start + open.len()..];
            let content = &content[..content.find(&close).expect("a closed block")];
            let digest = base64::engine::general_purpose::STANDARD
                .encode(sha2::Sha256::digest(content.as_bytes()));
            assert!(
                policy.contains(&format!("'sha256-{digest}'")),
                "the inline {tag} is not admitted: {policy}"
            );
            inline += 1;
        }
    }
    assert_eq!(policy.matches("'sha256-").count(), inline, "{policy}");

    // A share's token is its path, so nothing may carry it away as a referrer.
    // A route that matched and found no share answers with the headers too.
    let share = get("/share/nope").await;
    assert_eq!(share.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        header(&share, "referrer-policy").as_deref(),
        Some("no-referrer")
    );
    assert_eq!(
        header(&share, "x-content-type-options").as_deref(),
        Some("nosniff")
    );
    assert_eq!(header(&share, "x-frame-options").as_deref(), Some("DENY"));
    assert_eq!(
        header(&share, "content-security-policy").as_deref(),
        Some("frame-ancestors 'none'")
    );

    // The façade: none of them.
    let rest = get("/rest/ping").await;
    for name in [
        "content-security-policy",
        "x-content-type-options",
        "referrer-policy",
        "x-frame-options",
    ] {
        assert_eq!(header(&rest, name), None, "/rest carries {name}");
    }
}

#[tokio::test]
async fn sqlite_and_instance_key_backup_restore_as_one_consistent_bundle() {
    let (_temp, config, state) = test_app().await;
    let password = "backup-web-password";
    let user = state
        .db
        .create_account(
            "backup-admin",
            &security::hash_password(password).unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state.secret_box.encrypt(b"backup-subsonic-secret").unwrap();
    state
        .db
        .set_subsonic_credential(
            user,
            user,
            &encrypted,
            &security::token_hash("wfsk_backup"),
            now_ms(),
        )
        .await
        .unwrap();
    let bundle = config.data_dir.with_file_name("backup-bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    state
        .db
        .backup_to(&bundle.join("waveflow.db"))
        .await
        .unwrap();
    std::fs::copy(&config.instance_key_path, bundle.join("instance.key")).unwrap();
    assert!(
        waveflow_server::storage::database::Database::check_file(&bundle.join("waveflow.db"))
            .await
            .unwrap()
    );
    state.db.pool().close().await;
    drop(state);

    let mismatched_bundle = config.data_dir.with_file_name("mismatched-backup-bundle");
    std::fs::create_dir_all(&mismatched_bundle).unwrap();
    std::fs::copy(
        bundle.join("waveflow.db"),
        mismatched_bundle.join("waveflow.db"),
    )
    .unwrap();
    std::fs::write(mismatched_bundle.join("instance.key"), [9u8; 32]).unwrap();
    let database_before = std::fs::read(&config.database_path).unwrap();
    let key_before = std::fs::read(&config.instance_key_path).unwrap();
    let error = waveflow_server::cli::restore(
        &config,
        waveflow_server::cli::RestoreArgs {
            input: mismatched_bundle,
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("do not match"));
    assert_eq!(
        std::fs::read(&config.database_path).unwrap(),
        database_before
    );
    assert_eq!(
        std::fs::read(&config.instance_key_path).unwrap(),
        key_before
    );

    waveflow_server::cli::restore(
        &config,
        waveflow_server::cli::RestoreArgs {
            input: bundle.clone(),
        },
    )
    .await
    .unwrap();
    let restored = waveflow_server::initialize(&config).await.unwrap();
    let credential = restored
        .services
        .credential_by_username("backup-admin")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        restored
            .services
            .decrypt_subsonic_password(&credential)
            .unwrap(),
        b"backup-subsonic-secret"
    );
    assert!(restored.db.integrity_check().await.unwrap());
}

/// A request is named by this server, whatever the caller proposed.
///
/// The name goes in every `http_request` span, and `CLAUDE.md` allows no header
/// into a trace sink. A well-formed id is no safer than a malformed one for
/// that purpose — reuse somebody else's and the trace record stops telling two
/// callers apart — so the header is dropped on the way in and one is minted
/// here, whatever arrived.
///
/// The round trip below is the only way to see which id the server actually
/// used: `PropagateRequestIdLayer` puts it on the response, which is also what
/// keeps correlation possible — an upstream records the name it was given.
/// A boot that is refused closes the database before it says so.
///
/// Dropping a pool is not closing it: sqlx closes each connection on its own
/// thread after the drop has returned, and the last of them checkpoints the
/// WAL under an exclusive lock. A refusal that only dropped left whoever opens
/// the file next — the operator retrying, or a test right after the refusal —
/// racing that close, and losing it as `database is locked`. A zero
/// `busy_timeout` makes the next open fail rather than wait, so a lock still
/// held is seen however briefly it lasts.
#[tokio::test]
async fn a_refused_boot_gives_the_database_back_before_it_returns() {
    let (_temp, config, state) = test_app().await;
    state.db.pool().close().await;
    drop(state);
    std::fs::write(&config.instance_key_path, [7u8; 32]).unwrap();
    let mut impatient = config.clone();
    impatient.sqlite_busy_timeout = std::time::Duration::ZERO;

    for attempt in 0..30 {
        let refusal = match waveflow_server::initialize(&config).await {
            Ok(_) => panic!("a key that is not the database's must stop the boot"),
            Err(error) => format!("{error:#}"),
        };
        assert!(
            refusal.contains("instance.key does not match"),
            "attempt {attempt}: refused for another reason: {refusal}"
        );
        let next = waveflow_server::storage::database::Database::open(&impatient)
            .await
            .unwrap_or_else(|error| {
                panic!("attempt {attempt}: the refused boot still held the database: {error}")
            });
        next.pool().close().await;
    }
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

/// A browser opening a share link is handed the web client, which plays the
/// share; every other caller keeps the JSON it always had (RFC-020
/// decision 3).
///
/// `/share/{token}` answered JSON to everyone, so the person a link was sent to
/// saw a document of identifiers where a player should have been. And handing
/// out the page costs no visit: a link preview fetching it to draw a card has
/// not listened to anything, and a cap must not be spent on one.
#[tokio::test]
async fn a_browser_opening_a_share_link_is_handed_the_player() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password(&uuid::Uuid::new_v4().to_string()).unwrap();
    let owner = state
        .db
        .create_account("page-owner", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let (_library, tracks) =
        seed_titles(&state, &config, owner, "page-music", 5_000, &["Sent Over"]).await;
    let share = state
        .services
        .create_share(owner, &tracks, Some("For you"), None)
        .await
        .unwrap();
    let url = format!("/share/{}", share.url_token.clone().unwrap());
    let router = waveflow_server::app(&config, state.clone());
    let get = |uri: String, accept: Option<&'static str>| {
        let router = router.clone();
        async move {
            let mut request = Request::get(uri);
            if let Some(accept) = accept {
                request = request.header("accept", accept);
            }
            router
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };
    let content_type = |response: &axum::response::Response| {
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .to_owned()
    };

    // What a browser sends when somebody clicks the link.
    let navigation = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
    let page = get(url.clone(), Some(navigation)).await;
    assert_eq!(page.status(), StatusCode::OK);
    assert!(content_type(&page).starts_with("text/html"));
    assert_eq!(page.headers()["vary"], "accept");
    assert_eq!(page.headers()["cache-control"], "no-store");
    // The share headers of RFC-019, on the page as on everything under
    // `/share/` — and the web client's own policy, without which the shell's
    // inline blocks would not run, with framing refused as on every share.
    assert_eq!(page.headers()["referrer-policy"], "no-referrer");
    assert_eq!(page.headers()["x-content-type-options"], "nosniff");
    assert_eq!(page.headers()["x-frame-options"], "DENY");
    let page_policy = page.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        page_policy.starts_with("default-src 'self'; script-src 'self'"),
        "{page_policy}"
    );
    assert!(
        page_policy.ends_with("frame-ancestors 'none'"),
        "{page_policy}"
    );
    let visits = || async { state.services.shares(owner).await.unwrap()[0].visit_count };
    assert_eq!(visits().await, 0, "handing out the page is not a visit");

    // Nor is a `HEAD`, which is what an uptime monitor or a link previewer
    // sends, and which names no `Accept` at all. It is answered as the page
    // is, status included, and the cap is not spent on it.
    let head = |uri: String| {
        let router = router.clone();
        async move {
            router
                .oneshot(Request::head(uri).body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };
    let probed = head(url.clone()).await;
    assert_eq!(probed.status(), StatusCode::OK);
    assert!(content_type(&probed).starts_with("text/html"));
    assert_eq!(
        head("/share/wfs_nothing-here".into()).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(visits().await, 0, "a HEAD is not a visit");

    // Everybody else, including a caller that says nothing, keeps the JSON.
    for accept in [None, Some("*/*"), Some("application/json")] {
        let document = get(url.clone(), accept).await;
        assert_eq!(document.status(), StatusCode::OK, "{accept:?}");
        assert!(
            content_type(&document).starts_with("application/json"),
            "{accept:?}"
        );
        assert_eq!(document.headers()["vary"], "accept");
        assert_eq!(
            document.headers()["content-security-policy"],
            "frame-ancestors 'none'"
        );
        assert_eq!(document.headers()["referrer-policy"], "no-referrer");
        assert_eq!(document.headers()["x-frame-options"], "DENY");
    }
    assert_eq!(visits().await, 3);

    // A link that opens nothing says so in the page, with the status a
    // missing resource has.
    let dead = get("/share/wfs_nothing-here".into(), Some(navigation)).await;
    assert_eq!(dead.status(), StatusCode::NOT_FOUND);
    assert!(content_type(&dead).starts_with("text/html"));
    // The rest of the namespace stays the server's, whatever the caller accepts.
    let below = get(format!("{url}/elsewhere"), Some(navigation)).await;
    assert_eq!(below.status(), StatusCode::NOT_FOUND);
    assert!(!content_type(&below).starts_with("text/html"));
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
