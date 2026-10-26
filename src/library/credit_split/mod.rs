//! Splitting a joined artist credit, by decision (RFC-015).
//!
//! The tag mapper (`crate::library::tags`) cuts an artist value on the separators no
//! band name carries. A comma is not one of them — `Tyler, The Creator` is one
//! artist, `Zedd, Alessia Cara` two — so a credit holding one is left whole,
//! and an owner decides. This is the second step, applied to each name the
//! first one produced:
//!
//! ```text
//! tag value ── tags::credits ──▶ credit names ── decisions ──▶ participants
//! ```
//!
//! Everything here is pure: a name, its sort form and the library's decisions
//! in, names out. That is what lets a decision be applied to the stored
//! catalogue without reading a file, and what makes a scan and an immediate
//! split end in the same rows — they call the same function.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::library::tags::{Credit, Role};

/// The characters a joined credit is cut on: the desktop's set, ASCII and
/// fullwidth commas and the two ideographic ones. Unpadded, as the desktop's
/// cut is, so `蔡依林，周杰伦` splits.
pub const FRAGMENT_SEPARATORS: [char; 4] = [',', '，', '、', '､'];

/// The key a decision is stored under: the name folded the way the artist
/// identity folds it, so one key names exactly the spellings that make one
/// artist row under the default spec — and survives a change of spec, which
/// an artist id would not.
pub fn credit_key(name: &str) -> String {
    crate::library::pid::str_clear(&name.trim().to_lowercase())
}

/// The name cut on [`FRAGMENT_SEPARATORS`], never inside parentheses, trimmed,
/// blanks dropped — and duplicates kept, because a sort form is paired with
/// this list position by position before anything is deduplicated.
///
/// The parenthesis rule is the tag mapper's (`tags::split_on`): a comma inside
/// one qualifies a name rather than dividing two, so `Ian Asher, Olly Alexander
/// (Years & Years)` is two artists and not three fragments.
fn cut(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth: usize = 0;
    let mut current = String::new();
    for character in value.chars() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 && FRAGMENT_SEPARATORS.contains(&character) {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(character);
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

fn canonical(name: &str) -> String {
    waveflow_core::scanner::canonical_name(name)
}

/// What a split of `name` would credit: the cut, deduplicated on the canonical
/// name with the first spelling kept.
pub fn fragments(name: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    cut(name)
        .into_iter()
        .filter(|part| seen.insert(canonical(part)))
        .collect()
}

/// Whether `name` is a credit an owner may be asked about: at least two
/// fragments, none of them the whole credit again. `Solo,` never is.
pub fn is_candidate(name: &str) -> bool {
    let parts = fragments(name);
    let whole = canonical(name);
    parts.len() >= 2 && parts.iter().all(|part| canonical(part) != whole)
}

/// What a library decided about one credit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreditDecision {
    /// Replaced, wherever it is credited, by these names in this order.
    Split(Vec<String>),
    /// Left whole. It changes nothing a scan writes; it is what takes the
    /// credit off the list.
    Keep,
}

/// A library's decisions, by [`credit_key`].
pub type Decisions = HashMap<String, CreditDecision>;

/// The parts a split credit is replaced by on one track, each with the sort
/// form this track's tags paired with it.
///
/// The sort form is cut by the same rule and paired with the track's own
/// fragments **before** they are deduplicated, all or nothing as
/// `tags::paired_sorts` pairs a joined tag. Pairing after the deduplication
/// would compare `A, B, B, C` — three names — with four sort forms, and drop
/// every one of them.
fn expand(credit: &Credit, parts: &[String]) -> Vec<(String, Option<String>)> {
    let names = cut(&credit.name);
    let sorts = credit.sort_name.as_deref().map(cut).unwrap_or_default();
    let paired: Vec<(String, Option<String>)> = if sorts.len() == names.len() {
        names.into_iter().zip(sorts.into_iter().map(Some)).collect()
    } else {
        names.into_iter().map(|name| (name, None)).collect()
    };
    let mut by_name: HashMap<String, Option<String>> = HashMap::new();
    for (name, sort) in paired {
        by_name.entry(canonical(&name)).or_insert(sort);
    }
    parts
        .iter()
        .map(|part| {
            let sort = by_name.get(&canonical(part)).cloned().flatten();
            (part.clone(), sort)
        })
        .collect()
}

/// The participants one track credits: its first-step credits with the
/// library's decisions applied.
///
/// `artist_override` is a correction someone typed (`track_override.artists`).
/// It replaces the `artist` role whole and is final, so no decision touches
/// it; every other role still follows the file and the decisions. A corrected
/// name takes the album-artist sort form of the same name, as it did before
/// this step existed.
///
/// `identity` is the artist id a name derives. A name whose artist already
/// appears earlier in the same role and sub-role is not credited twice — the
/// first occurrence wins — and positions are numbered from zero across each
/// role, which `track_participant`'s `UNIQUE (track_id, role, position)` asks.
pub fn resolve(
    credits: &[Credit],
    decisions: &Decisions,
    artist_override: Option<&[String]>,
    identity: impl Fn(&str) -> Uuid,
) -> Vec<Credit> {
    let mut ordered: Vec<&Credit> = credits.iter().collect();
    // Role order first, then each role in its own order: the relation reads
    // positions per role, and a stable order keeps two runs identical.
    ordered.sort_by_key(|credit| (credit.role, credit.position));

    let mut expanded: Vec<(Role, String, String, Option<String>)> = Vec::new();
    if let Some(names) = artist_override {
        for name in names {
            let sort = credits
                .iter()
                .find(|credit| credit.role == Role::AlbumArtist && credit.name == *name)
                .and_then(|credit| credit.sort_name.clone());
            expanded.push((Role::Artist, String::new(), name.clone(), sort));
        }
    }
    for credit in ordered {
        if credit.role == Role::Artist && artist_override.is_some() {
            continue;
        }
        match decisions.get(&credit_key(&credit.name)) {
            Some(CreditDecision::Split(parts)) => {
                for (name, sort) in expand(credit, parts) {
                    expanded.push((credit.role, credit.sub_role.clone(), name, sort));
                }
            }
            _ => expanded.push((
                credit.role,
                credit.sub_role.clone(),
                credit.name.clone(),
                credit.sort_name.clone(),
            )),
        }
    }

    let mut seen: HashSet<(Role, String, Uuid)> = HashSet::new();
    let mut next: HashMap<Role, usize> = HashMap::new();
    let mut resolved = Vec::with_capacity(expanded.len());
    for (role, sub_role, name, sort_name) in expanded {
        let name = name.trim().to_owned();
        if name.is_empty() || !seen.insert((role, sub_role.clone(), identity(&name))) {
            continue;
        }
        let position = next.entry(role).or_insert(0);
        resolved.push(Credit {
            role,
            sub_role,
            name,
            sort_name,
            position: *position,
        });
        *position += 1;
    }
    resolved
}

#[cfg(test)]
mod tests {
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
}
