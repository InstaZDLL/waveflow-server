//! Natural order for names a person reads: runs of digits compare by value,
//! so `2 Song.flac` comes before `10 Song.flac` and `100 Song.flac`.
//!
//! Registered on every pooled connection as the SQLite collation
//! [`COLLATION`], so a listing paged in SQL is ordered in SQL. It is a
//! collation of this binary only: it must never appear in a schema object —
//! an index, a view, a `CHECK` — or the database would no longer open in a
//! tool that lacks it. Queries name it; the schema never does.

use std::cmp::Ordering;
use std::iter::Peekable;
use std::str::Chars;

/// The name queries use: `ORDER BY x COLLATE natural_order`.
/// Not `natural`, which SQLite reads as the keyword of `NATURAL JOIN`.
pub const COLLATION: &str = "natural_order";

/// Digits by value, everything else case-folded, character by character.
///
/// A total order, as SQLite requires of a collation: two names that compare
/// equal by those rules — `07` and `7`, `Abc` and `abc` — are then ordered by
/// their bytes, so the only names that compare equal are identical ones.
pub fn compare(a: &str, b: &str) -> Ordering {
    natural(a, b).then_with(|| a.cmp(b))
}

fn natural(a: &str, b: &str) -> Ordering {
    let (mut left, mut right) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (m, n) = (digits(&mut left), digits(&mut right));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                // Without leading zeros, a longer run is a larger number; runs
                // of one length compare as their digits do. No parsing, so a
                // run longer than any integer type still has its place.
                match m.len().cmp(&n.len()).then_with(|| m.cmp(n)) {
                    Ordering::Equal => {}
                    unequal => return unequal,
                }
            }
            (Some(x), Some(y)) => {
                left.next();
                right.next();
                match x.to_lowercase().cmp(y.to_lowercase()) {
                    Ordering::Equal => {}
                    unequal => return unequal,
                }
            }
        }
    }
}

fn digits(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut run = String::new();
    while let Some(digit) = chars.next_if(char::is_ascii_digit) {
        run.push(digit);
    }
    run
}

#[cfg(test)]
mod tests;
