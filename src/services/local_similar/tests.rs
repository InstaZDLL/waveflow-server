use super::{era_bonus, Score};

#[test]
fn each_signal_is_capped_and_the_era_alone_qualifies_nothing() {
    let era_only = Score {
        era: 2,
        ..Score::default()
    };
    assert!(!era_only.qualifies());
    let everything = Score {
        credits: 10,
        genres: 10,
        together: 10,
        era: 2,
    };
    assert_eq!(everything.total(), 9 + 6 + 5 + 2);
    assert_eq!(era_bonus(1990.0, 1995.0), 2);
    assert_eq!(era_bonus(1990.0, 2000.0), 1);
    assert_eq!(era_bonus(1990.0, 2001.0), 0);
}
