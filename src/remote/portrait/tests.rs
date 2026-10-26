use super::*;

/// The protection the suite cannot exercise end to end, since its image
/// host is on loopback: the default refuses loopback and every private
/// range, and admits a public address.
#[test]
fn the_default_policy_refuses_a_loopback_address() {
    let policy = AddressPolicy::default();
    for refused in [
        "127.0.0.1",
        "::1",
        "::ffff:127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "::",
        "fd00::1",
        "fe80::1",
        "::ffff:192.168.1.1",
        "198.18.0.1",
        "240.0.0.1",
        "fec0::1",
        "64:ff9b::a00:1",
        "64:ff9b:1::1",
        "2002:a00:1::1",
        "::10.0.0.1",
        "192.0.0.1",
        "192.88.99.1",
        "2001:0:a00:1::1",
        "2001:db8::1",
        "100::1",
        "::ffff:0:a00:1",
    ] {
        assert!(!policy.admits(refused.parse().unwrap()), "{refused}");
    }
    assert!(policy.admits("93.184.216.34".parse().unwrap()));
    assert!(policy.admits("2606:4700::1111".parse().unwrap()));
    assert!(AddressPolicy::AllowLoopback.admits("127.0.0.1".parse().unwrap()));
    assert!(!AddressPolicy::AllowLoopback.admits("10.0.0.1".parse().unwrap()));
}

#[test]
fn the_bytes_decide_the_format() {
    assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
    assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("png"));
    assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
    assert_eq!(sniff(b"<html>"), None);
}
