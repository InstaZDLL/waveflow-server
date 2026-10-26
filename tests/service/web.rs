//! The embedded web client: its shell, its manifest and icons, the share page
//! it hands a browser, and the security headers on what it serves.

use super::*;

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
