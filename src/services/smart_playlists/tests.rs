use super::rules::{DAY_MS, SORT_RANDOM};
use super::statement::EVALUATE;
use super::*;
use serde_json::json;

fn parse(value: serde_json::Value) -> Result<SmartRules, RuleError> {
    SmartRules::parse(&value)
}

#[test]
fn a_rule_set_round_trips_through_its_stored_form() {
    let rules = parse(json!({
        "match": "any",
        "conditions": [
            {"field": "genre", "operator": "is", "value": "Rock"},
            {"field": "year", "operator": "between", "value": [1990, 1999]},
            {"field": "last_played", "operator": "in_the_last", "value": 30},
            {"field": "starred", "operator": "is", "value": true},
            {"field": "format", "operator": "is_not", "value": "mp3"},
            {"field": "date_added", "operator": "after", "value": "2024-01-31"},
        ],
        "sort": "play_count",
        "order": "desc",
        "limit": 50,
    }))
    .unwrap();
    let stored: serde_json::Value = serde_json::from_str(&rules.to_stored()).unwrap();
    assert_eq!(parse(stored).unwrap(), rules);
    assert_eq!(rules.sort_code(), 12);
}

/// The allowlist, on every axis a client controls. Each refusal names where
/// it is, and none repeats what was sent.
#[test]
fn anything_outside_the_allowlist_is_refused_where_it_is() {
    let refused = |value: serde_json::Value, at: &str| {
        let error = parse(value).unwrap_err();
        assert_eq!(error.at, at, "{error}");
    };
    // The connective Ampache concatenated: not a word this parse knows.
    refused(json!({"match": "all) OR 1=1 --"}), "match");
    refused(json!({"match": "AND"}), "match");
    refused(json!({"match": 1}), "match");
    refused(json!({"extra": true}), "rules");
    refused(json!([]), "rules");
    refused(
        json!({"conditions": [{"field": "title; DROP TABLE track", "operator": "is", "value": "x"}]}),
        "conditions[0].field",
    );
    refused(
        json!({"conditions": [{"field": "title", "operator": "=", "value": "x"}]}),
        "conditions[0].operator",
    );
    refused(
        json!({"conditions": [{"field": "year", "operator": "contains", "value": "19"}]}),
        "conditions[0].operator",
    );
    refused(
        json!({"conditions": [{"field": "year", "operator": "is", "value": "1999"}]}),
        "conditions[0].value",
    );
    refused(
        json!({"conditions": [{"field": "year", "operator": "between", "value": [2000, 1990]}]}),
        "conditions[0].value",
    );
    refused(
        json!({"conditions": [{"field": "rating", "operator": "is", "value": 6}]}),
        "conditions[0].value",
    );
    refused(
        json!({"conditions": [{"field": "format", "operator": "is", "value": "%"}]}),
        "conditions[0].value",
    );
    refused(
        json!({"conditions": [{"field": "library", "operator": "is", "value": "not-a-uuid"}]}),
        "conditions[0].value",
    );
    refused(
        json!({"conditions": [{"field": "date_added", "operator": "before", "value": "yesterday"}]}),
        "conditions[0].value",
    );
    refused(
        json!({"conditions": [{"field": "title", "operator": "is", "value": "x", "sql": "1"}]}),
        "conditions[0]",
    );
    refused(
        json!({"conditions": [{"field": "title", "operator": "is", "value": "  "}]}),
        "conditions[0].value",
    );
    refused(json!({"sort": "t.title DESC"}), "sort");
    refused(json!({"order": "desc; --"}), "order");
    refused(json!({"limit": 0}), "limit");
    refused(json!({"limit": 1001}), "limit");
    refused(json!({"limit": "10"}), "limit");
    let many: Vec<_> = (0..=MAX_SMART_CONDITIONS)
        .map(|_| json!({"field": "title", "operator": "contains", "value": "a"}))
        .collect();
    refused(json!({"conditions": many}), "conditions");
}

/// Every field and every operator the parse admits is one the statement
/// branches on, and in the same spelling. A field added to the parse but
/// not to the statement would match nothing, silently.
#[test]
fn every_admitted_field_and_operator_is_spelled_in_the_statement() {
    for (name, _) in SmartField::ALL {
        assert!(
            EVALUATE.contains(&format!("WHEN '{name}' THEN")),
            "{name} is admitted and never tested"
        );
    }
    for operator in [
        "eq", "ne", "has", "hasnt", "sw", "ew", "gt", "lt", "bt", "ge",
    ] {
        assert!(
            EVALUATE.contains(&format!("'{operator}'")),
            "{operator} is bound and never compared"
        );
    }
}

/// The sort codes and the `ORDER BY` terms are one table written twice.
#[test]
fn the_sort_codes_match_the_statement() {
    let columns = [
        "s.title",
        "s.artist_display",
        "s.album_title",
        "s.year",
        "COALESCE(s.user_rating, 0)",
        "s.play_count",
        "s.last_played_at",
        "s.created_at",
        "s.duration_ms",
        "s.bitrate",
    ];
    // The direction too: the term after `END`, up to the next comma, says
    // `ASC` for an odd code and `DESC` for the even one after it. Two terms
    // swapped would keep every column in place and reverse every sort.
    let direction = |code: usize, column: &str| {
        let head = format!("p.sort_key = {code} THEN {column} END");
        let at = EVALUATE
            .find(&head)
            .unwrap_or_else(|| panic!("code {code} does not sort by {column}"));
        let tail = &EVALUATE[at + head.len()..];
        let term = &tail[..tail.find(',').unwrap_or(tail.len())];
        if term.contains(" DESC ") {
            "DESC"
        } else if term.contains(" ASC ") {
            "ASC"
        } else {
            panic!("code {code} names no direction: {term:?}")
        }
    };
    for (index, column) in columns.iter().enumerate() {
        let ascending = index * 2 + 1;
        assert_eq!(direction(ascending, column), "ASC", "code {ascending}");
        assert_eq!(
            direction(ascending + 1, column),
            "DESC",
            "code {}",
            ascending + 1
        );
    }
    let random = parse(json!({"sort": "random", "order": "desc"})).unwrap();
    assert_eq!(random.sort_code(), SORT_RANDOM);
    assert!(EVALUATE.contains("p.sort_key = 99 THEN random()"));
}

/// A library id is compared in the spelling `library.id` is stored in,
/// whatever spelling the client sent.
#[test]
fn a_library_id_is_kept_in_its_canonical_spelling() {
    let id = Uuid::new_v4();
    for sent in [
        id.to_string().to_uppercase(),
        format!("{{{id}}}"),
        id.simple().to_string(),
    ] {
        let rules = parse(json!({
            "conditions": [{"field": "library", "operator": "is_not", "value": sent}],
        }))
        .unwrap();
        assert_eq!(rules.conditions[0].value, json!(id.to_string()));
        let bound: serde_json::Value = serde_json::from_str(&rules.bound_conditions(0)).unwrap();
        assert_eq!(bound[0]["v"], json!(id.to_string()));
        assert_eq!(bound[0]["o"], "ne");
    }
}

/// What reaches the statement is built from the validated types: the
/// connective is one of two literals and the bound conditions carry only
/// keys this code wrote.
#[test]
fn the_bound_form_holds_only_what_this_code_wrote() {
    let rules = parse(json!({
        "match": "all",
        "conditions": [{"field": "artist", "operator": "is", "value": "Björk"}],
    }))
    .unwrap();
    let bound: serde_json::Value = serde_json::from_str(&rules.bound_conditions(1_000)).unwrap();
    assert_eq!(
        bound,
        json!([{
            "f": "artist", "o": "eq", "v": "Björk",
            "k": waveflow_core::scanner::canonical_name("Björk"),
            "n": null, "m": null,
        }])
    );
    let within = parse(json!({
        "conditions": [{"field": "last_played", "operator": "in_the_last", "value": 2}],
    }))
    .unwrap();
    let bound: serde_json::Value =
        serde_json::from_str(&within.bound_conditions(10 * DAY_MS)).unwrap();
    assert_eq!(bound[0]["o"], "ge");
    assert_eq!(bound[0]["n"], 8 * DAY_MS);
}
