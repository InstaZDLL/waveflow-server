use super::*;

fn credit(role: Role, position: usize, name: &str, sort: Option<&str>) -> Credit {
    Credit {
        role,
        sub_role: String::new(),
        name: name.to_owned(),
        sort_name: sort.map(str::to_owned),
        position,
    }
}

/// The identity the default spec derives, without a library: enough to
/// tell two names apart the way the catalogue does.
fn identity(name: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, credit_key(name).as_bytes())
}

fn names(credits: &[Credit], role: Role) -> Vec<(usize, &str)> {
    credits
        .iter()
        .filter(|credit| credit.role == role)
        .map(|credit| (credit.position, credit.name.as_str()))
        .collect()
}

fn split(parts: &[&str]) -> CreditDecision {
    CreditDecision::Split(parts.iter().map(|part| (*part).to_owned()).collect())
}

#[test]
fn the_fragments_are_the_desktops_with_parentheses_protected() {
    assert_eq!(
        fragments("Ashreveal, Ashish Swargiary"),
        ["Ashreveal", "Ashish Swargiary"]
    );
    assert_eq!(fragments("蔡依林，周杰伦"), ["蔡依林", "周杰伦"]);
    assert_eq!(fragments("庄心妍、祁隆"), ["庄心妍", "祁隆"]);
    assert_eq!(fragments("A､B, C"), ["A", "B", "C"]);
    assert_eq!(fragments("A, B, A"), ["A", "B"]);
    assert_eq!(
        fragments("Ian Asher, Olly Alexander (Years, Years)"),
        ["Ian Asher", "Olly Alexander (Years, Years)"],
        "a comma inside parentheses qualifies a name"
    );
}

#[test]
fn a_candidate_has_two_fragments_and_none_is_the_whole() {
    assert!(is_candidate("Zedd, Alessia Cara"));
    assert!(is_candidate("Tyler, The Creator"), "only a person can tell");
    assert!(!is_candidate("Solo,"));
    assert!(!is_candidate("Simon & Garfunkel"));
    assert!(!is_candidate("A, a"), "one fragment once deduplicated");
}

#[test]
fn the_key_folds_case_and_typographic_punctuation_only() {
    assert_eq!(credit_key(" A, B "), credit_key("a, b"));
    assert_eq!(credit_key("Guns N’ Roses"), credit_key("guns n' roses"));
    assert_ne!(credit_key("AC/DC"), credit_key("AC DC"));
}

#[test]
fn a_split_replaces_the_credit_in_place_and_renumbers() {
    let credits = [
        credit(Role::Artist, 0, "X", None),
        credit(Role::Artist, 1, "A, B", None),
        credit(Role::Artist, 2, "Y", None),
        credit(Role::AlbumArtist, 0, "A, B", None),
    ];
    let decisions = Decisions::from([(credit_key("A, B"), split(&["A", "B"]))]);
    let resolved = resolve(&credits, &decisions, None, identity);
    assert_eq!(
        names(&resolved, Role::Artist),
        [(0, "X"), (1, "A"), (2, "B"), (3, "Y")]
    );
    assert_eq!(names(&resolved, Role::AlbumArtist), [(0, "A"), (1, "B")]);
}

#[test]
fn no_decision_leaves_every_credit_as_the_file_wrote_it() {
    let credits = [
        credit(Role::Artist, 0, "A, B", Some("B, A")),
        credit(Role::Composer, 0, "C, D", None),
    ];
    let resolved = resolve(&credits, &Decisions::new(), None, identity);
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].sort_name.as_deref(), Some("B, A"));
}

#[test]
fn a_keep_changes_nothing() {
    let credits = [credit(Role::Artist, 0, "Tyler, The Creator", None)];
    let decisions = Decisions::from([(credit_key("Tyler, The Creator"), CreditDecision::Keep)]);
    assert_eq!(
        names(&resolve(&credits, &decisions, None, identity), Role::Artist),
        [(0, "Tyler, The Creator")]
    );
}

#[test]
fn a_part_already_credited_is_not_credited_twice() {
    let credits = [
        credit(Role::Artist, 0, "A", None),
        credit(Role::Artist, 1, "a, B", None),
    ];
    let decisions = Decisions::from([(credit_key("a, B"), split(&["a", "B"]))]);
    assert_eq!(
        names(&resolve(&credits, &decisions, None, identity), Role::Artist),
        [(0, "A"), (1, "B")],
        "the first occurrence wins, and positions stay contiguous"
    );
}

#[test]
fn two_credits_sharing_a_part_keep_the_files_order_whichever_is_undone() {
    let credits = [
        credit(Role::Artist, 0, "A, B", None),
        credit(Role::Artist, 1, "A, C", None),
    ];
    let both = Decisions::from([
        (credit_key("A, B"), split(&["A", "B"])),
        (credit_key("A, C"), split(&["A", "C"])),
    ]);
    assert_eq!(
        names(&resolve(&credits, &both, None, identity), Role::Artist),
        [(0, "A"), (1, "B"), (2, "C")]
    );
    let first = Decisions::from([(credit_key("A, B"), split(&["A", "B"]))]);
    assert_eq!(
        names(&resolve(&credits, &first, None, identity), Role::Artist),
        [(0, "A"), (1, "B"), (2, "A, C")]
    );
    let second = Decisions::from([(credit_key("A, C"), split(&["A", "C"]))]);
    assert_eq!(
        names(&resolve(&credits, &second, None, identity), Role::Artist),
        [(0, "A, B"), (1, "A"), (2, "C")]
    );
}

#[test]
fn sort_forms_are_paired_before_the_fragments_are_deduplicated() {
    let credits = [credit(Role::Artist, 0, "A, B, B, C", Some("a, b, b, c"))];
    let decisions = Decisions::from([(credit_key("A, B, B, C"), split(&["A", "B", "C"]))]);
    let resolved = resolve(&credits, &decisions, None, identity);
    let sorts: Vec<Option<&str>> = resolved
        .iter()
        .map(|credit| credit.sort_name.as_deref())
        .collect();
    assert_eq!(sorts, [Some("a"), Some("b"), Some("c")]);
}

#[test]
fn a_sort_form_of_another_length_pairs_with_nothing() {
    let credits = [credit(Role::Artist, 0, "A, B", Some("a"))];
    let decisions = Decisions::from([(credit_key("A, B"), split(&["A", "B"]))]);
    assert!(resolve(&credits, &decisions, None, identity)
        .iter()
        .all(|credit| credit.sort_name.is_none()));
}

#[test]
fn a_retagged_spelling_falls_under_the_same_decision() {
    let credits = [credit(Role::Artist, 0, "a, b", None)];
    let decisions = Decisions::from([(credit_key("A, B"), split(&["A", "B"]))]);
    assert_eq!(
        names(&resolve(&credits, &decisions, None, identity), Role::Artist),
        [(0, "A"), (1, "B")]
    );
}

#[test]
fn a_correction_is_final_and_the_other_roles_still_follow_decisions() {
    let credits = [
        credit(Role::Artist, 0, "A, B", None),
        credit(Role::AlbumArtist, 0, "A, B", Some("B, A")),
    ];
    let decisions = Decisions::from([(credit_key("A, B"), split(&["A", "B"]))]);
    let corrected = ["A, B".to_owned()];
    let resolved = resolve(&credits, &decisions, Some(&corrected), identity);
    assert_eq!(names(&resolved, Role::Artist), [(0, "A, B")]);
    assert_eq!(names(&resolved, Role::AlbumArtist), [(0, "A"), (1, "B")]);
    assert_eq!(
        resolved[0].sort_name.as_deref(),
        Some("B, A"),
        "a corrected name takes the album artist's sort form of that name"
    );
}

#[test]
fn performers_number_one_sequence_across_their_instruments() {
    let mut guitar = credit(Role::Performer, 0, "A, B", None);
    guitar.sub_role = "Guitar".into();
    let mut drums = credit(Role::Performer, 1, "C", None);
    drums.sub_role = "Drums".into();
    let decisions = Decisions::from([(credit_key("A, B"), split(&["A", "B"]))]);
    let resolved = resolve(&[guitar, drums], &decisions, None, identity);
    assert_eq!(
        names(&resolved, Role::Performer),
        [(0, "A"), (1, "B"), (2, "C")]
    );
}
