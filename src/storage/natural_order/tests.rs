use super::*;

fn sorted(names: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    names.sort_by(|a, b| compare(a, b));
    names
}

#[test]
fn numbers_sort_by_value() {
    assert_eq!(
        sorted(&[
            "100 Snooze.m4a",
            "2 Paradise.m4a",
            "10 Ballad.m4a",
            "1 Morning.mp3"
        ]),
        [
            "1 Morning.mp3",
            "2 Paradise.m4a",
            "10 Ballad.m4a",
            "100 Snooze.m4a"
        ]
    );
    assert_eq!(
        sorted(&["Disc 10", "Disc 9", "Disc 1"]),
        ["Disc 1", "Disc 9", "Disc 10"]
    );
}

#[test]
fn letters_fold_case_beyond_ascii() {
    assert_eq!(
        sorted(&["b", "Été", "A", "étoile"]),
        ["A", "b", "étoile", "Été"]
    );
    assert_eq!(compare("abc", "ABD"), Ordering::Less);
}

#[test]
fn only_identical_names_compare_equal() {
    assert_eq!(compare("7", "7"), Ordering::Equal);
    assert_ne!(compare("07", "7"), Ordering::Equal);
    assert_ne!(compare("Abc", "abc"), Ordering::Equal);
    assert_eq!(compare("07", "7"), compare("07", "7"));
    assert_eq!(compare("07", "7"), compare("7", "07").reverse());
}

#[test]
fn a_run_longer_than_any_integer_still_sorts() {
    let huge = "9".repeat(40);
    assert_eq!(compare("1", &huge), Ordering::Less);
    assert_eq!(compare(&format!("{huge}0"), &huge), Ordering::Greater);
}
