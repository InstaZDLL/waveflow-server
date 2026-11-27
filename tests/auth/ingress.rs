//! The browser session inside Home Assistant (RFC-030): the cookies under the
//! ingress prefix, the origin check against Home Assistant's address, and the
//! header read on the ingress listener only.

use super::*;

const PREFIX: &str = "/api/hassio_ingress/tok_42";
const HOME_ASSISTANT: &str = "http://homeassistant.local:8123";

/// An account to sign in with, and a server whose public URL is its own
/// address: the case where an ingress sign-in would be refused if the check
/// read it.
async fn server() -> (tempfile::TempDir, axum::Router, String) {
    let (temp, mut config, mut state) = test_app().await;
    config.public_url = Some("https://music.example.org".to_owned());
    state.public_url = config.public_url.clone();
    let password = Uuid::new_v4().to_string();
    state
        .db
        .create_account(
            "ha-listener",
            &security::hash_password(&password).unwrap(),
            AccountRole::User,
            now_ms(),
        )
        .await
        .unwrap();
    (temp, waveflow_server::app(&config, state), password)
}

/// A request as Home Assistant hands it on: the prefix stripped from the path
/// and named in `X-Ingress-Path`, `Host` and `Origin` as the browser sent them.
fn through_home_assistant(mut request: Request<Body>, proto: &str) -> Request<Body> {
    let headers = request.headers_mut();
    headers.insert("x-ingress-path", PREFIX.parse().unwrap());
    headers.insert("x-forwarded-proto", proto.parse().unwrap());
    headers.insert("origin", HOME_ASSISTANT.parse().unwrap());
    headers.insert("host", "homeassistant.local:8123".parse().unwrap());
    request
}

fn login(password: &str) -> Request<Body> {
    json_request(
        "/api/v2/web/auth/login",
        serde_json::json!({
            "username": "ha-listener",
            "password": password,
            "device_name": "Home Assistant panel"
        }),
    )
}

fn set_cookies(response: &axum::response::Response) -> Vec<String> {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect()
}

fn cookie<'a>(cookies: &'a [String], name: &str) -> &'a str {
    cookies
        .iter()
        .find(|cookie| cookie.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name} cookie in {cookies:?}"))
}

#[tokio::test]
async fn a_session_opened_through_ingress_lives_under_its_prefix() {
    let (_temp, app, password) = server().await;
    let ingress = waveflow_server::ingress_app(app);

    let response = ingress
        .clone()
        .oneshot(through_home_assistant(login(&password), "http"))
        .await
        .unwrap();
    // Accepted although the public URL names another address: the browser is
    // on Home Assistant's, and both `Origin` and `Host` say so.
    assert_eq!(response.status(), StatusCode::OK);
    let cookies = set_cookies(&response);
    let refresh = cookie(&cookies, "waveflow-refresh");
    let csrf = cookie(&cookies, "waveflow-csrf");
    assert!(
        refresh.contains(&format!("Path={PREFIX}/api/v2/web/auth;")),
        "{refresh}"
    );
    assert!(csrf.contains(&format!("Path={PREFIX}/;")), "{csrf}");
    // Home Assistant is on plain http here, whatever the public URL says: a
    // `Secure` cookie would never be stored.
    assert!(!refresh.contains("Secure"), "{refresh}");
    assert!(!csrf.contains("Secure"), "{csrf}");

    // The refresh the browser makes later, with the cookies it was given.
    let refresh_pair = refresh.split(';').next().unwrap();
    let csrf_pair = csrf.split(';').next().unwrap();
    let csrf_value = csrf_pair.split_once('=').unwrap().1;
    let mut renew = Request::post("/api/v2/web/auth/refresh")
        .body(Body::empty())
        .unwrap();
    renew.headers_mut().insert(
        "cookie",
        format!("{refresh_pair}; {csrf_pair}").parse().unwrap(),
    );
    renew
        .headers_mut()
        .insert("x-waveflow-csrf", csrf_value.parse().unwrap());
    let renewed = ingress
        .clone()
        .oneshot(through_home_assistant(renew, "http"))
        .await
        .unwrap();
    assert_eq!(renewed.status(), StatusCode::OK);

    // Signing out clears the cookies where they were set.
    let renewed_cookies = set_cookies(&renewed);
    let refresh_pair = cookie(&renewed_cookies, "waveflow-refresh")
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let csrf_pair = cookie(&renewed_cookies, "waveflow-csrf")
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let mut logout = Request::post("/api/v2/web/auth/logout")
        .body(Body::empty())
        .unwrap();
    logout.headers_mut().insert(
        "cookie",
        format!("{refresh_pair}; {csrf_pair}").parse().unwrap(),
    );
    logout.headers_mut().insert(
        "x-waveflow-csrf",
        csrf_pair.split_once('=').unwrap().1.parse().unwrap(),
    );
    let logged_out = ingress
        .oneshot(through_home_assistant(logout, "http"))
        .await
        .unwrap();
    assert_eq!(logged_out.status(), StatusCode::NO_CONTENT);
    let cleared = set_cookies(&logged_out);
    assert!(
        cookie(&cleared, "waveflow-refresh").contains(&format!("Path={PREFIX}/api/v2/web/auth;")),
        "{cleared:?}"
    );
    assert!(
        cookie(&cleared, "waveflow-csrf").contains(&format!("Path={PREFIX}/;")),
        "{cleared:?}"
    );
}

#[tokio::test]
async fn a_home_assistant_on_https_gets_secure_cookies() {
    let (_temp, app, password) = server().await;
    let response = waveflow_server::ingress_app(app)
        .oneshot(through_home_assistant(login(&password), "https"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookies = set_cookies(&response);
    assert!(cookie(&cookies, "waveflow-refresh").contains("; Secure"));
    assert!(cookie(&cookies, "waveflow-csrf").contains("; Secure"));
}

#[tokio::test]
async fn the_main_listener_never_reads_the_ingress_header() {
    // The same request, sent straight to the port the network reaches: the
    // header changes nothing, so the public URL decides, and Home Assistant's
    // address is not the server's.
    let (_temp, app, password) = server().await;
    let response = app
        .oneshot(through_home_assistant(login(&password), "http"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(response).await["code"], "origin_rejected");
}

#[tokio::test]
async fn the_ingress_listener_still_checks_the_origin() {
    // Another page on another site, posting into the panel's session.
    let (_temp, app, password) = server().await;
    let mut request = through_home_assistant(login(&password), "http");
    request
        .headers_mut()
        .insert("origin", "https://attacker.invalid".parse().unwrap());
    let response = waveflow_server::ingress_app(app)
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(response).await["code"], "origin_rejected");
}

#[tokio::test]
async fn the_ingress_listener_refuses_a_request_without_its_prefix() {
    let (_temp, app, password) = server().await;
    let mut request = through_home_assistant(login(&password), "http");
    request.headers_mut().remove("x-ingress-path");
    let response = waveflow_server::ingress_app(app)
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "ingress_path_invalid");
}
