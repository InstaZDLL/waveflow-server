use super::*;
use axum::http::HeaderValue;

fn nets(list: &[&str]) -> Vec<IpNet> {
    list.iter().map(|net| net.parse().unwrap()).collect()
}

fn forwarded(values: &[&str]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append("x-forwarded-for", HeaderValue::from_str(value).unwrap());
    }
    headers
}

fn ip(value: &str) -> IpAddr {
    value.parse().unwrap()
}

#[test]
fn an_untrusted_peer_is_the_client_whatever_it_forwards() {
    let headers = forwarded(&["203.0.113.9"]);
    assert_eq!(
        client_address(Some(ip("198.51.100.7")), &headers, &nets(&["10.0.0.0/8"])),
        Some(ip("198.51.100.7"))
    );
}

#[test]
fn a_trusted_proxy_is_read_from_the_right() {
    // The client wrote 1.1.1.1; the proxy appended the real 198.51.100.7.
    let headers = forwarded(&["1.1.1.1, 198.51.100.7"]);
    assert_eq!(
        client_address(Some(ip("10.0.0.2")), &headers, &nets(&["10.0.0.0/8"])),
        Some(ip("198.51.100.7"))
    );
    // Two proxies, and several headers joined in order.
    let headers = forwarded(&["1.1.1.1", "198.51.100.7, 10.0.0.3"]);
    assert_eq!(
        client_address(Some(ip("10.0.0.2")), &headers, &nets(&["10.0.0.0/8"])),
        Some(ip("198.51.100.7"))
    );
}

#[test]
fn a_mapped_ipv4_proxy_is_still_the_trusted_one() {
    // A dual-stack listener reports the IPv4 proxy as mapped, and so may a
    // proxy forwarding through another.
    let headers = forwarded(&["1.1.1.1, 198.51.100.7, ::ffff:10.0.0.3"]);
    assert_eq!(
        client_address(
            Some(ip("::ffff:10.0.0.2")),
            &headers,
            &nets(&["10.0.0.0/8"])
        ),
        Some(ip("198.51.100.7"))
    );
    // A list naming the proxy in its mapped form holds it whichever way
    // the socket reports it, once loaded.
    let mapped = vec![trusted_network("::ffff:10.0.0.0/104".parse().unwrap())];
    assert_eq!(mapped, nets(&["10.0.0.0/8"]));
    for peer in ["::ffff:10.0.0.2", "10.0.0.2"] {
        assert_eq!(
            client_address(Some(ip(peer)), &headers, &mapped),
            Some(ip("198.51.100.7"))
        );
    }
    // But an IPv6 block wider than the mapped range trusts no IPv4 peer:
    // were it trusted, the walk would hand back the hop before the
    // malformed entry instead of the peer.
    let wide = vec![trusted_network("::/0".parse().unwrap())];
    let headers = forwarded(&["garbage, 198.51.100.7"]);
    for peer in ["::ffff:10.0.0.2", "10.0.0.2"] {
        assert_eq!(
            client_address(Some(ip(peer)), &headers, &wide),
            Some(ip("10.0.0.2"))
        );
    }
    assert_eq!(
        trusted_network("2001:db8::/96".parse().unwrap()),
        "2001:db8::/96".parse::<IpNet>().unwrap()
    );
}

#[test]
fn a_malformed_or_all_trusted_chain_never_yields_a_chosen_address() {
    let trusted = nets(&["10.0.0.0/8"]);
    let headers = forwarded(&["1.1.1.1, garbage, 10.0.0.3"]);
    assert_eq!(
        client_address(Some(ip("10.0.0.2")), &headers, &trusted),
        Some(ip("10.0.0.3"))
    );
    let headers = forwarded(&["10.0.0.9, 10.0.0.3"]);
    assert_eq!(
        client_address(Some(ip("10.0.0.2")), &headers, &trusted),
        Some(ip("10.0.0.2"))
    );
    assert_eq!(
        client_address(Some(ip("10.0.0.2")), &HeaderMap::new(), &trusted),
        Some(ip("10.0.0.2"))
    );
}

/// A byte that is not text in the part the client wrote stops nothing on
/// the right of it: the entry the proxy appended is still found.
#[test]
fn a_non_text_byte_in_the_client_s_part_does_not_hide_the_proxy_s_entry() {
    let mut headers = HeaderMap::new();
    headers.append(
        "x-forwarded-for",
        HeaderValue::from_bytes(b"1.1.1.1\xff, 198.51.100.7").unwrap(),
    );
    assert_eq!(
        client_address(Some(ip("10.0.0.2")), &headers, &nets(&["10.0.0.0/8"])),
        Some(ip("198.51.100.7"))
    );
    // And on its own, it stops the walk at the last trusted hop.
    let mut headers = HeaderMap::new();
    headers.append(
        "x-forwarded-for",
        HeaderValue::from_bytes(b"\xff, 10.0.0.3").unwrap(),
    );
    assert_eq!(
        client_address(Some(ip("10.0.0.2")), &headers, &nets(&["10.0.0.0/8"])),
        Some(ip("10.0.0.3"))
    );
}

#[test]
fn ipv6_counts_by_its_slash_64_and_mapped_ipv4_as_ipv4() {
    assert_eq!(
        address_bucket(Some(ip("2001:db8:1:2:aaaa::1"))),
        address_bucket(Some(ip("2001:db8:1:2:bbbb::9")))
    );
    assert_ne!(
        address_bucket(Some(ip("2001:db8:1:2::1"))),
        address_bucket(Some(ip("2001:db8:1:3::1")))
    );
    assert_eq!(
        address_bucket(Some(ip("::ffff:198.51.100.7"))),
        "198.51.100.7"
    );
}

#[test]
fn case_variants_of_a_username_share_a_window() {
    let guard = SignInGuard::new(2);
    for name in ["Alice", "ALICE", "alice", "aLiCe"]
        .iter()
        .cycle()
        .take(FAILURES_PER_WINDOW)
    {
        drop(guard.reserve("a", &canonical_account(name)).unwrap());
    }
    assert!(guard.reserve("a", &canonical_account("alice")).is_err());
    assert_eq!(guard.window_count(), 1);
}

#[test]
fn another_address_keeps_its_own_budget() {
    let guard = SignInGuard::new(2);
    for _ in 0..FAILURES_PER_WINDOW {
        drop(guard.reserve("attacker", "owner").unwrap());
    }
    assert_eq!(guard.reserve("attacker", "owner").err(), Some(Throttled));
    assert!(guard.reserve("home", "owner").is_ok());
}

#[test]
fn a_released_slot_is_given_back_and_a_dropped_one_is_kept() {
    let guard = SignInGuard::new(2);
    for _ in 0..FAILURES_PER_WINDOW {
        guard.reserve("a", "owner").unwrap().release();
    }
    assert!(guard.reserve("a", "owner").is_ok());
}

#[test]
fn one_address_never_opens_more_than_its_windows() {
    let guard = SignInGuard::new(2);
    // 31 accounts, one window each.
    for index in 0..WINDOWS_PER_ADDRESS - 1 {
        drop(guard.reserve("a", &format!("invented-{index}")).unwrap());
    }
    assert_eq!(guard.window_count(), WINDOWS_PER_ADDRESS - 1);
    // The 32nd account onwards all count in the one overflow window.
    for index in 0..FAILURES_PER_WINDOW {
        drop(guard.reserve("a", &format!("more-{index}")).unwrap());
    }
    assert_eq!(guard.window_count(), WINDOWS_PER_ADDRESS);
    assert_eq!(guard.reserve("a", "yet-another").err(), Some(Throttled));
    // An account that already has its window keeps it.
    assert!(guard.reserve("a", "invented-0").is_ok());
    assert!(guard.reserve("b", "someone").is_ok());
}

#[test]
fn a_full_table_refuses_new_keys_and_keeps_live_ones() {
    let guard = SignInGuard::new(2);
    {
        let mut table = guard.table.lock().unwrap();
        let now = Instant::now();
        for index in 0..MAX_WINDOWS {
            table.windows.insert(
                window_key(&format!("filler-{index}"), "x"),
                Window {
                    address: format!("filler-{index}"),
                    overflow: false,
                    attempts: VecDeque::from([now]),
                },
            );
        }
    }
    assert_eq!(guard.reserve("new", "owner").err(), Some(Throttled));
    // A window already counting is never dropped to make room.
    assert!(guard.reserve("filler-7", "x").is_ok());
    assert_eq!(guard.window_count(), MAX_WINDOWS);
}

#[tokio::test]
async fn the_gate_bounds_the_queue() {
    let guard = SignInGuard::new(1);
    let held = guard.admit().await.expect("the first permit is free");
    let mut waiters = Vec::new();
    for _ in 0..WAITING_PER_PERMIT {
        let guard = Arc::clone(&guard);
        waiters.push(tokio::spawn(async move { guard.admit().await.is_some() }));
    }
    while guard.waiting() < WAITING_PER_PERMIT {
        tokio::task::yield_now().await;
    }
    // The queue is full: the next one is turned away at once, not after
    // waiting out the five seconds a queued request may wait.
    let turned_away = tokio::time::timeout(Duration::from_millis(500), guard.admit()).await;
    assert!(
        matches!(turned_away, Ok(None)),
        "a request past a full queue waited for a permit"
    );
    drop(held);
    for waiter in waiters {
        assert!(waiter.await.unwrap());
    }
    assert_eq!(guard.waiting(), 0);
}
