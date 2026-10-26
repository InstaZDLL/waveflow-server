use super::*;

#[test]
fn a_folder_name_is_trimmed_bounded_and_folded() {
    assert_eq!(
        folder_name("  Été / 2024 "),
        Some(("Été / 2024".into(), "été / 2024".into()))
    );
    assert_eq!(
        folder_name(&"é".repeat(120)).map(|(name, _)| name.chars().count()),
        Some(120)
    );
    for refused in ["", "   ", "tab\there", "line\nbreak"] {
        assert!(folder_name(refused).is_none(), "{refused:?}");
    }
    assert!(folder_name(&"é".repeat(121)).is_none());
}
