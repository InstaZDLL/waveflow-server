use super::query_key;

#[test]
fn a_query_is_keyed_trimmed_and_case_folded() {
    assert_eq!(
        query_key("  Björk "),
        Some(("Björk".to_owned(), "björk".to_owned()))
    );
    assert_eq!(query_key("BJÖRK").map(|(_, key)| key), Some("björk".into()));
}

#[test]
fn an_empty_or_overlong_query_is_refused() {
    assert_eq!(query_key("   "), None);
    assert_eq!(query_key(&"a".repeat(201)), None);
    assert!(query_key(&"a".repeat(200)).is_some());
}
