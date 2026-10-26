use super::*;

fn names_of(credits: &[Credit], role: Role) -> Vec<&str> {
    credits
        .iter()
        .filter(|credit| credit.role == role)
        .map(|credit| credit.name.as_str())
        .collect()
}

/// The separator set is padded so a slash inside a name survives it.
#[test]
fn a_slash_inside_a_name_is_not_a_separator() {
    let raw = RawCredits {
        artist: vec!["AC/DC".into()],
        ..RawCredits::default()
    };
    assert_eq!(names_of(&credits(&raw), Role::Artist), vec!["AC/DC"]);

    let joined = RawCredits {
        artist: vec!["Nova Kern / Lior Sand".into()],
        ..RawCredits::default()
    };
    assert_eq!(
        names_of(&credits(&joined), Role::Artist),
        vec!["Nova Kern", "Lior Sand"]
    );
}

/// A separator inside parentheses qualifies one credit; it does not divide
/// two. The real tag that exposed this named two people and produced three
/// entities, each carrying a parenthesis it never opened.
#[test]
fn a_role_separator_inside_parentheses_does_not_cut() {
    let raw = RawCredits {
        artist: vec!["ITZY".into()],
        roles: vec![(
            Role::Composer,
            vec!["Kobee (Melange / INHOUSE), Holy M (Melange / INHOUSE)".into()],
        )],
        ..RawCredits::default()
    };
    assert_eq!(
        names_of(&credits(&raw), Role::Composer),
        vec!["Kobee (Melange / INHOUSE), Holy M (Melange / INHOUSE)"]
    );
}

/// The parenthesis rule must not cost the cut it was added beside: a
/// separator outside one still divides, in the same value.
#[test]
fn a_role_separator_outside_parentheses_still_cuts() {
    let raw = RawCredits {
        artist: vec!["Nobody".into()],
        roles: vec![(
            Role::Composer,
            vec!["Bach (arr. Gounod)/Liszt (after Bach)".into()],
        )],
        ..RawCredits::default()
    };
    assert_eq!(
        names_of(&credits(&raw), Role::Composer),
        vec!["Bach (arr. Gounod)", "Liszt (after Bach)"]
    );
}

/// A tag whose parentheses never balance must not swallow the whole value
/// or hang the scan. The opening one protects only the tail it opened.
#[test]
fn an_unbalanced_parenthesis_protects_only_its_tail() {
    let unclosed = RawCredits {
        artist: vec!["Nobody".into()],
        roles: vec![(
            Role::Composer,
            vec!["Bach/Gounod (Melange / INHOUSE".into()],
        )],
        ..RawCredits::default()
    };
    assert_eq!(
        names_of(&credits(&unclosed), Role::Composer),
        vec!["Bach", "Gounod (Melange / INHOUSE"]
    );

    // A stray closing parenthesis must not make the rest uncuttable.
    let stray = RawCredits {
        artist: vec!["Nobody".into()],
        roles: vec![(Role::Composer, vec!["INHOUSE)/Bach".into()])],
        ..RawCredits::default()
    };
    assert_eq!(
        names_of(&credits(&stray), Role::Composer),
        vec!["INHOUSE)", "Bach"]
    );
}

/// A role tag is cut on the bare separators, where an artist tag is not.
#[test]
fn a_role_tag_is_cut_where_an_artist_tag_would_not_be() {
    let raw = RawCredits {
        artist: vec!["Nobody".into()],
        roles: vec![(Role::Composer, vec!["Bach/Gounod".into()])],
        ..RawCredits::default()
    };
    assert_eq!(
        names_of(&credits(&raw), Role::Composer),
        vec!["Bach", "Gounod"]
    );
}

#[test]
fn the_plural_tag_wins_and_is_never_cut() {
    let raw = RawCredits {
        artist: vec!["Ignored; Entirely".into()],
        artists: vec!["Nova Kern; Lior Sand".into()],
        ..RawCredits::default()
    };
    assert_eq!(
        names_of(&credits(&raw), Role::Artist),
        vec!["Nova Kern; Lior Sand"],
        "a tagger writing the plural tag has already said where the boundaries are"
    );
}

#[test]
fn a_singular_tag_holding_two_frames_is_not_cut_either() {
    let raw = RawCredits {
        artist: vec!["Nova Kern; A".into(), "Lior Sand".into()],
        ..RawCredits::default()
    };
    assert_eq!(
        names_of(&credits(&raw), Role::Artist),
        vec!["Nova Kern; A", "Lior Sand"]
    );
}

#[test]
fn a_performer_carries_the_instrument_it_is_credited_on() {
    assert_eq!(
        split_performer("Jimmy Page (guitar)"),
        ("Guitar".to_owned(), "Jimmy Page".to_owned())
    );
    assert_eq!(
        split_performer("Salaam Remi (drums (drum set) and organ)"),
        (
            "Drums (Drum Set) And Organ".to_owned(),
            "Salaam Remi".to_owned()
        )
    );
    assert_eq!(
        split_performer("Nobody In Particular"),
        (String::new(), "Nobody In Particular".to_owned())
    );
}

#[test]
fn a_performer_pair_names_every_player_of_its_instrument() {
    let raw = RawCredits {
        artist: vec!["Nobody".into()],
        performer_pairs: vec![("guitar".into(), "Jimmy Page; Nova Kern".into())],
        ..RawCredits::default()
    };
    let credits = credits(&raw);
    let performers: Vec<(&str, &str)> = credits
        .iter()
        .filter(|credit| credit.role == Role::Performer)
        .map(|credit| (credit.sub_role.as_str(), credit.name.as_str()))
        .collect();
    assert_eq!(
        performers,
        vec![("Guitar", "Jimmy Page"), ("Guitar", "Nova Kern")]
    );
}

/// An album has to hang off somebody, and which somebody depends on
/// whether the file calls itself a compilation.
#[test]
fn a_file_naming_no_album_artist_still_has_one() {
    let plain = RawCredits {
        artist: vec!["Nova Kern".into()],
        ..RawCredits::default()
    };
    assert_eq!(
        names_of(&credits(&plain), Role::AlbumArtist),
        vec!["Nova Kern"]
    );

    let compilation = RawCredits {
        artist: vec!["Nova Kern".into()],
        is_compilation: true,
        ..RawCredits::default()
    };
    assert_eq!(
        names_of(&credits(&compilation), Role::AlbumArtist),
        vec![VARIOUS_ARTISTS]
    );
}

#[test]
fn a_sort_tag_is_paired_position_by_position_or_not_at_all() {
    let paired = RawCredits {
        artist: vec!["The Nocturnes; Nova Kern".into()],
        sort_artist: Some("Nocturnes, The; Kern, Nova".into()),
        ..RawCredits::default()
    };
    let paired_credits = credits(&paired);
    let sorts: Vec<Option<&str>> = paired_credits
        .iter()
        .filter(|credit| credit.role == Role::Artist)
        .map(|credit| credit.sort_name.as_deref())
        .collect();
    assert_eq!(sorts, vec![Some("Nocturnes, The"), Some("Kern, Nova")]);

    // Lengths that disagree mean the tagger did not pair them, and filing
    // one artist under another's sort form is worse than filing none.
    let mismatched = RawCredits {
        artist: vec!["The Nocturnes; Nova Kern".into()],
        sort_artist: Some("Nocturnes, The".into()),
        ..RawCredits::default()
    };
    let unpaired = credits(&mismatched);
    assert!(unpaired
        .iter()
        .filter(|credit| credit.role == Role::Artist)
        .all(|credit| credit.sort_name.is_none()));
}

#[test]
fn positions_are_per_role_and_follow_tag_order() {
    let raw = RawCredits {
        artist: vec!["A; B".into()],
        roles: vec![(Role::Producer, vec!["P1; P2".into()])],
        ..RawCredits::default()
    };
    let credits = credits(&raw);
    let at = |role: Role, position: usize| {
        credits
            .iter()
            .find(|credit| credit.role == role && credit.position == position)
            .map(|credit| credit.name.as_str())
    };
    assert_eq!(at(Role::Artist, 0), Some("A"));
    assert_eq!(at(Role::Artist, 1), Some("B"));
    assert_eq!(at(Role::Producer, 0), Some("P1"));
    assert_eq!(at(Role::Producer, 1), Some("P2"));
}
