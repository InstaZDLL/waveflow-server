//! Turning tag values into credits: who is on this file, and as what.
//!
//! The catalogue used to ask one question of a file — which artists is it by —
//! and answer it by splitting one string on `;`. OpenSubsonic asks a wider one,
//! and the reference answers it with thirteen named roles, a performer whose
//! credit carries an instrument, and two different rules for cutting a joined
//! value depending on which tag it came from.
//!
//! Three rules from the reference are worth stating because they are easy to
//! get subtly wrong:
//!
//! - **The plural tag wins and is never cut.** A file carrying `ARTISTS` has
//!   already said where the boundaries are; re-cutting its values would only
//!   find boundaries the tagger did not mean.
//! - **A singular tag that already holds several frames is never cut either**,
//!   for the same reason.
//! - **Artist separators are padded, role separators are not.** `" / "` with
//!   its spaces is what lets `AC/DC` through, while a `COMPOSER` reading
//!   `Bach/Gounod` is genuinely two people.

/// What an artist is credited as.
///
/// Held as an enum rather than a free string so a typo cannot invent a role,
/// but stored as its name: the vocabulary is open — OpenSubsonic adds to it —
/// and our migrations are immutable, so a database-level CHECK would mean
/// rebuilding a table to admit a fourteenth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// The track's own credit.
    Artist,
    /// The album's credit, which is not always the track's: a guest appearance
    /// names the guest while the album still belongs under its album artist.
    AlbumArtist,
    Composer,
    Lyricist,
    Conductor,
    Arranger,
    Producer,
    Director,
    Engineer,
    Mixer,
    Remixer,
    DjMixer,
    /// Carries an instrument in its sub-role when the file names one.
    Performer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::AlbumArtist => "albumartist",
            Self::Composer => "composer",
            Self::Lyricist => "lyricist",
            Self::Conductor => "conductor",
            Self::Arranger => "arranger",
            Self::Producer => "producer",
            Self::Director => "director",
            Self::Engineer => "engineer",
            Self::Mixer => "mixer",
            Self::Remixer => "remixer",
            Self::DjMixer => "djmixer",
            Self::Performer => "performer",
        }
    }

    /// The roles that name who the music is *by*, as opposed to who worked on
    /// it. These two are what `artists[]` and `albumArtists[]` render, and
    /// every other role is a contributor.
    pub fn is_main_credit(self) -> bool {
        matches!(self, Self::Artist | Self::AlbumArtist)
    }

    /// Every role a file can be read for, in the order they are emitted.
    pub const ALL: [Role; 13] = [
        Role::Artist,
        Role::AlbumArtist,
        Role::Composer,
        Role::Lyricist,
        Role::Conductor,
        Role::Arranger,
        Role::Producer,
        Role::Director,
        Role::Engineer,
        Role::Mixer,
        Role::Remixer,
        Role::DjMixer,
        Role::Performer,
    ];
}

/// One artist, credited once, in one capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credit {
    pub role: Role,
    /// The instrument a performer is credited on, empty for every other role.
    pub sub_role: String,
    pub name: String,
    /// The tagged sort form of this credit, when the file paired one with it.
    pub sort_name: Option<String>,
    /// Rank within the role, in tag order.
    pub position: usize,
}

/// Separators cutting a single-valued ARTIST or ALBUMARTIST.
///
/// Padded with spaces on purpose: an unpadded `/` would cut `AC/DC` in half.
const ARTIST_SEPARATORS: [&str; 6] = [" / ", " feat. ", " feat ", " ft. ", " ft ", "; "];

/// Separators cutting every other single-valued role tag.
///
/// Unpadded, because a `COMPOSER` reading `Bach/Gounod` is two people and no
/// composer's name contains a slash.
const ROLE_SEPARATORS: [&str; 2] = ["/", ";"];

fn split_on(value: &str, separators: &[&str]) -> Vec<String> {
    let mut parts = vec![value.to_owned()];
    for separator in separators {
        parts = parts
            .into_iter()
            .flat_map(|part| part.split(separator).map(str::to_owned).collect::<Vec<_>>())
            .collect();
    }
    parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

/// Applies the reference's rule for when a value may be cut at all.
///
/// `plural` is what a multi-valued tag supplied and is returned untouched;
/// `singular` is cut only when it holds exactly one value, because two frames
/// already say where the boundary is.
fn resolve_values(plural: &[String], singular: &[String], separators: &[&str]) -> Vec<String> {
    let clean = |values: &[String]| -> Vec<String> {
        values
            .iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect()
    };
    let plural = clean(plural);
    if !plural.is_empty() {
        return plural;
    }
    let singular = clean(singular);
    if singular.len() != 1 {
        return singular;
    }
    split_on(&singular[0], separators)
}

/// A performer credit, split into the instrument and the person.
///
/// Two spellings reach us. ID3 carries a musician-credits frame whose entries
/// are already key and value, so the caller hands them over split. Vorbis
/// writes one string, `Salaam Remi (drums and organ)`, where the first
/// parenthesised group is the instrument and the rest is the name.
pub fn split_performer(value: &str) -> (String, String) {
    let value = value.trim();
    let Some(open) = value.find('(') else {
        return (String::new(), value.to_owned());
    };
    // Nested parentheses belong to the instrument — "drums (drum set)" is one
    // credit, not a credit and a stray fragment.
    let mut depth = 0usize;
    let mut close = None;
    for (at, character) in value[open..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + at);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return (String::new(), value.to_owned());
    };
    let instrument = value[open + 1..close].trim().to_owned();
    let mut name = String::with_capacity(value.len());
    name.push_str(value[..open].trim());
    let tail = value[close + 1..].trim();
    if !tail.is_empty() {
        if !name.is_empty() {
            name.push(' ');
        }
        name.push_str(tail);
    }
    (title_case(&instrument), name)
}

/// Title-cases an instrument, as the reference does, so `guitar` and `Guitar`
/// are one sub-role rather than two.
fn title_case(value: &str) -> String {
    value
        .split(' ')
        .map(|word| {
            // The first *letter* leads, not the first character: an instrument
            // written "(drum set)" has to come back "(Drum Set)", which is
            // what Unicode title casing does and what the reference emits.
            let mut lifted = false;
            word.chars()
                .flat_map(|character| {
                    if !lifted && character.is_alphabetic() {
                        lifted = true;
                        character.to_uppercase().collect::<Vec<_>>()
                    } else {
                        character.to_lowercase().collect::<Vec<_>>()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The tag values a file supplied, before any rule is applied to them.
#[derive(Debug, Default, Clone)]
pub struct RawCredits {
    /// `ARTIST`, then `ARTISTS`.
    pub artist: Vec<String>,
    pub artists: Vec<String>,
    /// `ALBUMARTIST`, then `ALBUMARTISTS`.
    pub album_artist: Vec<String>,
    pub album_artists: Vec<String>,
    pub sort_artist: Option<String>,
    pub sort_album_artist: Option<String>,
    pub is_compilation: bool,
    /// Every other role, as the file spelled it.
    pub roles: Vec<(Role, Vec<String>)>,
    /// Performer credits already split into instrument and value by the
    /// reader, for the formats that store them that way.
    pub performer_pairs: Vec<(String, String)>,
}

/// The label an album falls back to when its files name no album artist and
/// say they are a compilation.
pub const VARIOUS_ARTISTS: &str = "Various Artists";

/// The MusicBrainz identifier of that label, which is a real artist there.
pub const VARIOUS_ARTISTS_MBID: &str = "89ad4ac3-39f7-470e-963a-56509c546377";

/// Reads a file's credits.
///
/// The album-artist fallback is the reference's: a file naming none is a
/// compilation's if it says so, and otherwise belongs under its own artists —
/// an album has to hang off somebody.
pub fn credits(raw: &RawCredits) -> Vec<Credit> {
    let mut credits = Vec::new();

    let artists = resolve_values(&raw.artists, &raw.artist, &ARTIST_SEPARATORS);
    let artist_sorts = paired_sorts(&artists, raw.sort_artist.as_deref(), &ARTIST_SEPARATORS);

    let mut album_artists =
        resolve_values(&raw.album_artists, &raw.album_artist, &ARTIST_SEPARATORS);
    if album_artists.is_empty() {
        album_artists = if raw.is_compilation {
            vec![VARIOUS_ARTISTS.to_owned()]
        } else {
            artists.clone()
        };
    }
    let album_sorts = paired_sorts(
        &album_artists,
        raw.sort_album_artist.as_deref(),
        &ARTIST_SEPARATORS,
    );

    push_all(&mut credits, Role::Artist, &artists, &artist_sorts);
    push_all(
        &mut credits,
        Role::AlbumArtist,
        &album_artists,
        &album_sorts,
    );

    // Role groups are merged before positions are handed out. A caller naming
    // one role twice — two `COMPOSER` frames arriving as two entries — would
    // otherwise restart at position 0 and collide with the first group on the
    // relation's `(track, role, position)` key, failing the whole write.
    let mut by_role: Vec<(Role, Vec<String>)> = Vec::new();
    for (role, values) in &raw.roles {
        match by_role.iter_mut().find(|(seen, _)| seen == role) {
            Some((_, merged)) => merged.extend(values.iter().cloned()),
            None => by_role.push((*role, values.clone())),
        }
    }
    for (role, values) in &by_role {
        let names = resolve_values(&[], values, &ROLE_SEPARATORS);
        let none = vec![None; names.len()];
        push_all(&mut credits, *role, &names, &none);
    }

    let mut performers: Vec<(String, String)> = Vec::new();
    for (sub_role, value) in &raw.performer_pairs {
        // A pair the reader already split still has its value cut, because one
        // instrument can name several people.
        for name in split_on(value, &ROLE_SEPARATORS) {
            performers.push((title_case(sub_role), name));
        }
    }
    for (position, (sub_role, name)) in performers.into_iter().enumerate() {
        credits.push(Credit {
            role: Role::Performer,
            sub_role,
            name,
            sort_name: None,
            position,
        });
    }

    credits
}

fn push_all(credits: &mut Vec<Credit>, role: Role, names: &[String], sorts: &[Option<String>]) {
    for (position, name) in names.iter().enumerate() {
        credits.push(Credit {
            role,
            sub_role: String::new(),
            name: name.clone(),
            sort_name: sorts.get(position).cloned().flatten(),
            position,
        });
    }
}

/// Pairs a joined sort tag with its joined name tag, position by position.
///
/// All or nothing: lists of different lengths say the tagger did not pair them,
/// and filing one artist under another's sort form is worse than filing none.
fn paired_sorts(names: &[String], raw: Option<&str>, separators: &[&str]) -> Vec<Option<String>> {
    let Some(raw) = raw else {
        return vec![None; names.len()];
    };
    let sorts = split_on(raw, separators);
    if sorts.len() == names.len() {
        sorts.into_iter().map(Some).collect()
    } else {
        vec![None; names.len()]
    }
}

#[cfg(test)]
mod tests {
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
}
