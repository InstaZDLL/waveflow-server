use super::MutationIntent;

#[test]
fn mutation_intents_are_canonical_and_scope_action_target_and_payload() {
    let first = MutationIntent::new(
        "update",
        "playlist:one",
        &serde_json::json!({ "name": "Road", "tracks": ["a", "b"] }),
    );
    let reordered = MutationIntent::new(
        "update",
        "playlist:one",
        &serde_json::json!({ "tracks": ["a", "b"], "name": "Road" }),
    );
    assert_eq!(first.as_bytes(), reordered.as_bytes());
    assert_ne!(
        first.as_bytes(),
        MutationIntent::new(
            "delete",
            "playlist:one",
            &serde_json::json!({ "name": "Road", "tracks": ["a", "b"] }),
        )
        .as_bytes()
    );
    assert_ne!(
        first.as_bytes(),
        MutationIntent::new(
            "update",
            "playlist:two",
            &serde_json::json!({ "name": "Road", "tracks": ["a", "b"] }),
        )
        .as_bytes()
    );
    assert_ne!(
        first.as_bytes(),
        MutationIntent::new(
            "update",
            "playlist:one",
            &serde_json::json!({ "name": "Night", "tracks": ["a", "b"] }),
        )
        .as_bytes()
    );
}
