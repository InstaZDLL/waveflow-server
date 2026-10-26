use super::{fts_prefix_query, MAX_SEARCH_QUERY_CHARS, MAX_SEARCH_TERMS};

/// A search query is narrowed to its first terms and characters rather
/// than handed to FTS5 whole (RFC-023 decision 5), and what is kept is
/// still the ordinary expression: the last term kept is the prefix.
#[test]
fn a_search_query_is_bounded_in_terms_and_characters() {
    let words: Vec<String> = (0..40).map(|index| format!("w{index}")).collect();
    let expression = fts_prefix_query(&words.join(" ")).unwrap();
    assert_eq!(expression.matches(" AND ").count(), MAX_SEARCH_TERMS - 1);
    assert!(expression.ends_with(&format!("\"w{}\"*", MAX_SEARCH_TERMS - 1)));

    // One enormous term is cut on a character boundary, accents included.
    let long = "é".repeat(MAX_SEARCH_QUERY_CHARS * 4);
    let expression = fts_prefix_query(&long).unwrap();
    assert_eq!(
        expression.chars().count(),
        MAX_SEARCH_QUERY_CHARS + 3,
        "two quotes and the prefix star around the kept characters"
    );

    // An ordinary search is untouched.
    assert_eq!(
        fts_prefix_query("dark side of the m").unwrap(),
        "\"dark\" AND \"side\" AND \"of\" AND \"the\" AND \"m\"*"
    );
}
