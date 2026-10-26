use super::non_blank;

#[test]
fn a_secret_of_whitespace_is_refused_and_the_variable_is_named() {
    for blank in ["", " ", "\n", "  \t\r\n "] {
        let refused = non_blank("WAVEFLOW_EXAMPLE_SECRET", blank.to_owned()).unwrap_err();
        assert!(
            refused.to_string().contains("WAVEFLOW_EXAMPLE_SECRET"),
            "the message has to name the variable the person must fix"
        );
    }
}

/// Returned as it was found, never trimmed.
///
/// The same path reads account and Subsonic passwords; trimming what it
/// hands back would silently alter a credential that already works.
#[test]
fn a_secret_with_padding_survives_intact() {
    let kept = non_blank("WAVEFLOW_EXAMPLE_SECRET", "  a token  ".to_owned()).unwrap();
    assert_eq!(kept, "  a token  ");
}
