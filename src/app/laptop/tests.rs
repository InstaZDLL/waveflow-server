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
    holder_answering_after(body, Duration::ZERO).await
}

/// The same, once `delay` has passed: the port is held from the start, as a
/// first copy holds it through its boot, and nobody reads a connection until
/// then.
async fn holder_answering_after(body: serde_json::Value, delay: Duration) -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route(
        "/health",
        get(move || {
            let body = body.clone();
            async move { Json(body) }
        }),
    );
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        axum::serve(listener, router).await
    });
    address
}

fn waveflow_health() -> serde_json::Value {
    serde_json::json!({ "status": "ok", "version": "2.0.0", "schema": 2 })
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
    let address = holder_answering(waveflow_health()).await;
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

/// A second double-click while the first copy is still booting: the port is
/// held and the probe is not read until the first copy starts serving. Past
/// the first timeout, the probe waits for it rather than calling it a
/// stranger.
#[tokio::test]
async fn a_waveflow_still_starting_is_waited_for() {
    let address = holder_answering_after(waveflow_health(), Duration::from_millis(600)).await;
    let (_temp, config) = config_on(address);
    let startup = bind_probing(
        &config,
        None,
        Duration::from_millis(100),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert!(matches!(startup, Startup::AlreadyRunning(_)), "{startup:?}");
}

/// A port that takes the connection and never answers is still refused once
/// the grace has run out, and the refusal says it may be WaveFlow starting.
#[tokio::test]
async fn a_port_that_never_answers_is_refused_as_maybe_still_starting() {
    // Bound and never accepted from: the kernel completes the handshake and
    // nobody reads.
    let holder = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = holder.local_addr().unwrap();
    let (_temp, config) = config_on(address);
    let error = bind_probing(
        &config,
        None,
        Duration::from_millis(100),
        Duration::from_millis(300),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("did not answer"), "{error}");
    assert!(
        error.contains("If it is WaveFlow still starting"),
        "{error}"
    );
    assert!(error.contains("WAVEFLOW_BIND line"), "{error}");
    drop(holder);
}

/// An address this machine does not have — a LAN address kept from another
/// network — is not a port somebody holds, and the refusal does not say it
/// is, nor send the reader to try the next port.
#[tokio::test]
async fn an_address_this_machine_lacks_is_refused_as_such() {
    // TEST-NET-1 (RFC 5737): never assigned to an interface.
    let (temp, config) = config_on("192.0.2.1:4533".parse().unwrap());
    let file = temp.path().join("laptop.env");
    let error = bind(&config, Some(&file)).await.unwrap_err().to_string();
    assert!(error.contains("cannot listen on 192.0.2.1:4533"), "{error}");
    assert!(error.contains("WAVEFLOW_BIND line"), "{error}");
    assert!(error.contains(&file.display().to_string()), "{error}");
    assert!(error.contains("WAVEFLOW_BIND=127.0.0.1:4533"), "{error}");
    assert!(!error.contains("another program"), "{error}");
}

/// A `[::]` bind collides, on Linux, with a copy on `127.0.0.1`: the probe
/// finds it there and hands over its address, not `[::1]`. Asked of the
/// probe directly, since whether the bind collides depends on the platform.
#[tokio::test]
async fn an_ipv6_wildcard_finds_a_copy_on_ipv4_loopback() {
    let held = holder_answering(waveflow_health()).await;
    let wildcard = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), held.port());
    assert_eq!(
        health_probe(wildcard, Duration::from_secs(3)).await,
        Probe::WaveFlow(held)
    );
    assert_eq!(
        browser_url(held),
        format!("http://127.0.0.1:{}/", held.port())
    );
}

/// The same, through `bind_probing`: the address handed over is the one that
/// answered. Only where a `[::]` bind does collide with `127.0.0.1`, so it
/// checks that first. On Linux it must: `net.ipv6.bindv6only` is 0 by
/// default, and a test that returned there would pass having checked nothing,
/// with libtest hiding why. Elsewhere it stops: Windows makes `[::]`
/// IPv6-only, and a host without IPv6 refuses it outright.
#[tokio::test]
async fn a_wildcard_bind_hands_over_the_address_that_answered() {
    let held = holder_answering(waveflow_health()).await;
    let wildcard = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), held.port());
    match TcpListener::bind(wildcard).await {
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {}
        other if cfg!(target_os = "linux") => panic!(
            "a [::] bind did not collide with 127.0.0.1 ({other:?}): \
             is net.ipv6.bindv6only set, or IPv6 off?"
        ),
        other => {
            eprintln!("no dual-stack collision on this host ({other:?}); nothing to check");
            return;
        }
    }
    let (_temp, config) = config_on(wildcard);
    match bind_probing(
        &config,
        None,
        Duration::from_secs(3),
        Duration::from_secs(3),
    )
    .await
    .unwrap()
    {
        Startup::AlreadyRunning(url) => {
            assert_eq!(url, format!("http://127.0.0.1:{}/", held.port()));
        }
        Startup::Listening(_) => panic!("bound a port another server holds"),
    }
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

/// What `waveflow-server health` asks too. Held here rather than through the
/// binary: on Linux a connection to `0.0.0.0` reaches the loopback anyway, so
/// a probe that forgot to translate the wildcard would still pass there.
#[test]
fn a_wildcard_bind_is_reached_on_the_loopback_of_its_family() {
    let v4: SocketAddr = "0.0.0.0:4533".parse().unwrap();
    let v6: SocketAddr = "[::]:4533".parse().unwrap();
    let lan: SocketAddr = "192.168.1.20:4533".parse().unwrap();
    assert_eq!(reachable(v4), "127.0.0.1:4533".parse().unwrap());
    assert_eq!(reachable(v6), "[::1]:4533".parse().unwrap());
    assert_eq!(reachable(lan), lan);
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

/// No token, no fragment: the address as the banner prints it.
#[test]
fn the_browser_opens_on_the_bare_address_without_a_token() {
    let base = "http://127.0.0.1:4533/";
    assert_eq!(browser::handover_url(base, None), base);
    assert_eq!(browser::handover_url(base, Some("  ")), base);
}

/// A token travels in the fragment and only there, on the sign-in screen: the
/// path and the query, which a browser sends, carry nothing of it.
#[test]
fn a_setup_token_travels_in_the_fragment_only() {
    let base = "http://127.0.0.1:4533/";
    let drawn = crate::auth::security::generate_token("wfsetup_");
    let url = browser::handover_url(base, Some(&drawn));
    assert_eq!(url, format!("{base}login#setup-token={drawn}"));
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.path(), "/login");
    assert_eq!(parsed.query(), None);
    assert_eq!(
        parsed.fragment(),
        Some(format!("setup-token={drawn}").as_str())
    );
}

/// A token an operator chose may hold anything; escaped, it reads back as it
/// was, and leaves nothing in the URL a command line could split on.
#[test]
fn an_operator_token_is_escaped_and_reads_back_unchanged() {
    let token = "my token & 100% sure #1+2=3 é";
    let url = browser::handover_url("http://127.0.0.1:4533/", Some(token));
    let fragment = url.split_once('#').unwrap().1;
    assert!(
        fragment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~%=".contains(c)),
        "{fragment}"
    );
    let value = fragment.strip_prefix("setup-token=").unwrap();
    assert_eq!(
        percent_encoding::percent_decode_str(value)
            .decode_utf8()
            .unwrap(),
        token
    );
}

/// What is handed over is the token the gate accepts: the file's content
/// without its newline, or the operator's value. Nothing once setup is
/// closed, and nothing — rather than a failure — when the file is gone.
#[test]
fn the_token_handed_over_is_the_one_setup_accepts() {
    use crate::auth::setup_token::TokenSource;

    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("setup-token");
    std::fs::write(&file, "wfsetup_drawn\n").unwrap();
    assert_eq!(
        browser::handover_token(Some(&TokenSource::File(file.clone())), None).as_deref(),
        Some("wfsetup_drawn")
    );
    assert_eq!(
        browser::token_left_in(temp.path()).as_deref(),
        Some("wfsetup_drawn")
    );
    assert_eq!(
        browser::handover_token(
            Some(&TokenSource::Operator),
            Some(" chosen by the operator ")
        ),
        Some("chosen by the operator".to_owned())
    );
    assert_eq!(browser::handover_token(None, Some("ignored")), None);

    std::fs::remove_file(&file).unwrap();
    assert_eq!(
        browser::handover_token(Some(&TokenSource::File(file)), None),
        None
    );
    assert_eq!(browser::token_left_in(temp.path()), None);
}
