use super::catalog::ReleaseGroup;

fn classify(types: &[&str], is_compilation: bool) -> ReleaseGroup {
    let types = types
        .iter()
        .map(|kind| (*kind).to_owned())
        .collect::<Vec<_>>();
    ReleaseGroup::classify(&types, is_compilation)
}

/// One case per row of the table in RFC-022 decision 2, in its order.
#[test]
fn each_rule_of_the_table_files_its_own_group() {
    assert_eq!(classify(&["album"], true), ReleaseGroup::Compilation);
    assert_eq!(
        classify(&["album", "compilation"], false),
        ReleaseGroup::Compilation
    );
    assert_eq!(classify(&["album", "live"], false), ReleaseGroup::Live);
    assert_eq!(classify(&["ep"], false), ReleaseGroup::Ep);
    assert_eq!(classify(&["single"], false), ReleaseGroup::Single);
    assert_eq!(classify(&["album"], false), ReleaseGroup::Album);
}

/// The first rule that matches wins: secondary types before primary ones.
#[test]
fn a_secondary_type_wins_over_the_primary_one() {
    assert_eq!(classify(&["ep", "live"], false), ReleaseGroup::Live);
    assert_eq!(
        classify(&["single", "compilation"], false),
        ReleaseGroup::Compilation
    );
    assert_eq!(classify(&["ep", "live"], true), ReleaseGroup::Compilation);
}

/// What the scanner stores joined with `;`, Picard's ID3v2.3 joins with
/// `/`, and some taggers capitalise. All of them read the same.
#[test]
fn separators_and_case_do_not_change_the_answer() {
    assert_eq!(classify(&["Album/Live"], false), ReleaseGroup::Live);
    assert_eq!(classify(&["album; live"], false), ReleaseGroup::Live);
    assert_eq!(classify(&["EP, Remix"], false), ReleaseGroup::Ep);
    assert_eq!(classify(&[" Single "], false), ReleaseGroup::Single);
    // A type is matched whole, never as a substring of a longer one.
    assert_eq!(classify(&["lives"], false), ReleaseGroup::Album);
    assert_eq!(classify(&["epic"], false), ReleaseGroup::Album);
}

/// Most libraries carry no release type at all, and keep the one section
/// they had; types with no section of their own fall to `album`.
#[test]
fn unknown_or_missing_types_are_albums() {
    assert_eq!(classify(&[], false), ReleaseGroup::Album);
    assert_eq!(classify(&["broadcast"], false), ReleaseGroup::Album);
    assert_eq!(
        classify(&["album", "soundtrack"], false),
        ReleaseGroup::Album
    );
    assert_eq!(classify(&["other"], false), ReleaseGroup::Album);
}
