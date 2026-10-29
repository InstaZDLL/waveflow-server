use super::*;

use axum::{routing::get, Json, Router};

/// A configuration whose only setting that matters here is the port.
fn config_on(address: SocketAddr) -> (tempfile::TempDir, Config) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::for_data_dir(temp.path().join("data"));
    config.laptop = true;
    config.bind_addr = address;
    (temp, config)
}

/// Something holding a port and answering `/health` with `body`.
async fn holder_answering(body: serde_json::Value) -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route(
        "/health",
        get(move || {
            let body = body.clone();
            async move { Json(body) }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, router).await });
    address
}

#[tokio::test]
async fn a_free_port_is_taken() {
    let (_temp, config) = config_on((Ipv4Addr::LOCALHOST, 0).into());
    assert!(matches!(
        bind(&config, None).await.unwrap(),
        Startup::Listening(_)
    ));
}

/// A second double-click finds the first copy, by what it answers.
#[tokio::test]
async fn a_waveflow_on_the_port_is_handed_over_to() {
    let address = holder_answering(serde_json::json!({
        "status": "ok",
        "version": "2.0.0",
        "schema": 2,
    }))
    .await;
    let (_temp, config) = config_on(address);
    match bind(&config, None).await.unwrap() {
        Startup::AlreadyRunning(url) => {
            assert_eq!(url, format!("http://127.0.0.1:{}/", address.port()));
        }
        Startup::Listening(_) => panic!("bound a port another server holds"),
    }
}

/// Another server that also answers 200 on that path is not this one, and
/// the refusal names the port and the line to change in the file it read.
#[tokio::test]
async fn another_program_on_the_port_is_a_refusal_naming_the_line_to_change() {
    let address = holder_answering(serde_json::json!({ "status": "ok" })).await;
    let (temp, config) = config_on(address);
    let file = temp.path().join("laptop.env");
    let error = bind(&config, Some(&file)).await.unwrap_err().to_string();
    let port = address.port();
    assert!(error.contains(&format!("port {port}")), "{error}");
    assert!(error.contains("WAVEFLOW_BIND line"), "{error}");
    assert!(error.contains(&file.display().to_string()), "{error}");
    assert!(
        error.contains(&format!("WAVEFLOW_BIND=127.0.0.1:{}", port + 1)),
        "{error}"
    );
}

#[test]
fn only_a_waveflow_health_body_is_recognised() {
    assert!(is_waveflow_health(&serde_json::json!({
        "status": "ok", "version": "2.0.0", "schema": 2,
    })));
    assert!(!is_waveflow_health(&serde_json::json!({ "status": "ok" })));
    assert!(!is_waveflow_health(&serde_json::json!({
        "status": "ok", "version": "2.0.0", "schema": 1,
    })));
    assert!(!is_waveflow_health(&serde_json::json!("ok")));
}

#[test]
fn the_banner_says_where_to_go_where_the_data_is_and_how_to_stop() {
    let data = std::env::temp_dir().join("WaveFlow Server");
    let banner = banner("0.0.0.0:4533".parse().unwrap(), &data);
    assert!(!banner.contains('\n'));
    assert!(banner.contains("http://127.0.0.1:4533/"), "{banner}");
    assert!(banner.contains(&data.display().to_string()), "{banner}");
    assert!(banner.contains("Close this window to stop the server"));
}

#[test]
fn the_network_hint_opens_the_bind_or_names_the_lan_address() {
    let loopback = describe_network("127.0.0.1:4533".parse().unwrap(), None);
    assert!(
        loopback.contains("WAVEFLOW_BIND=0.0.0.0:4533 in laptop.env"),
        "{loopback}"
    );

    let lan: IpAddr = "192.168.1.20".parse().unwrap();
    let open = describe_network("0.0.0.0:4533".parse().unwrap(), Some(lan));
    assert!(open.contains("http://192.168.1.20:4533/"), "{open}");

    let unknown = describe_network("0.0.0.0:4533".parse().unwrap(), None);
    assert!(unknown.contains("port 4533"), "{unknown}");

    let one_interface = describe_network("192.168.1.20:4600".parse().unwrap(), None);
    assert!(
        one_interface.contains("http://192.168.1.20:4600/"),
        "{one_interface}"
    );
}
