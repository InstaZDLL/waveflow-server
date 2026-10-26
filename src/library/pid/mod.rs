//! Persistent identifiers: the rule that decides which row a file belongs to.
//!
//! A catalogue has to answer "is this the same album as that one" from tags
//! alone, and the answer has to survive a rebuild. Navidrome states the rule as
//! a small configurable spec — a list of attribute groups, the first non-empty
//! group winning — and hashes the result. This is that engine.
//!
//! Two properties matter more than the syntax.
//!
//! **The album artist enters the identity as a whole string.** Never as a row
//! id, never split into credits. That is what makes `A; B` and `A; C` two
//! albums without a single special case, and it is why [`str_clear`] must stay
//! a light touch: it folds typographic punctuation to ASCII and nothing else.
//! It is deliberately *not* `waveflow_core::scanner::canonical_name`, which
//! maps every non-alphanumeric character to a space and would erase the
//! boundary between one credit and the next.
//!
//! **The identity is the identifier.** The hash is rendered as a UUID rather
//! than kept beside a random one, so the same files answer the same ids on a
//! fresh install, forever — and a rebuilt database stops re-minting every
//! album id under its clients.

use std::fmt;

use md5::{Digest, Md5};
use uuid::Uuid;

/// Separator written after every part fed to the hash.
///
/// A zero-width space, as in the reference: it cannot occur inside a tag value
/// a tagger would write, so `["ab", "c"]` and `["a", "bc"]` cannot collide.
const PART_SEPARATOR: &str = "\u{200b}";

/// Separator between the attributes of one group, before hashing.
const ATTRIBUTE_JOINER: &str = "\\";

/// One attribute of a spec group.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Attribute {
    /// The album this track belongs to, resolved through the album spec.
    AlbumId,
    /// The directory holding the file.
    Folder,
    /// The album artist's display string, hashed on its own.
    AlbumArtistId,
    Title,
    Album,
    /// Anything else names a tag, read raw.
    Tag(String),
}

impl Attribute {
    fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "albumid" => Self::AlbumId,
            "folder" => Self::Folder,
            "albumartistid" => Self::AlbumArtistId,
            "title" => Self::Title,
            "album" => Self::Album,
            other => Self::Tag(other.to_owned()),
        }
    }
}

/// What a spec reads a value from.
///
/// A trait rather than the catalogue's own input struct, so the engine can be
/// exercised without building one — and so nothing in here depends on the
/// scanner.
pub trait PidSource {
    fn tag(&self, name: &str) -> Option<&str>;
    fn folder(&self) -> &str;
    fn album_artist_display(&self) -> &str;
    fn title(&self) -> &str;
    fn album(&self) -> &str;
}

#[derive(Debug)]
pub struct PidSpecError {
    spec: String,
    reason: &'static str,
}

impl fmt::Display for PidSpecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid identifier spec `{}`: {}",
            self.spec, self.reason
        )
    }
}

impl std::error::Error for PidSpecError {}

/// A parsed spec: `group|group|…`, each group `attribute,attribute,…`.
#[derive(Debug, Clone)]
pub struct PidSpec {
    groups: Vec<Vec<Attribute>>,
    source: String,
}

impl PidSpec {
    /// Parses a spec, rejecting a recursive `albumid` outright.
    ///
    /// The reference logs an error and returns an empty value there, which
    /// silently degrades every album id into the fallback group — a
    /// misconfiguration that re-identifies the whole catalogue without saying
    /// so. Refusing at parse time is both louder and cheaper: the resolver then
    /// needs no depth counter at all.
    pub fn parse(source: &str, allow_album_id: bool) -> Result<Self, PidSpecError> {
        let groups: Vec<Vec<Attribute>> = source
            .split('|')
            .map(|group| {
                group
                    .split(',')
                    .filter(|attribute| !attribute.trim().is_empty())
                    .map(Attribute::parse)
                    .collect()
            })
            .collect();
        if groups.iter().flatten().count() == 0 {
            return Err(PidSpecError {
                spec: source.to_owned(),
                reason: "it names no attribute",
            });
        }
        if !allow_album_id && groups.iter().flatten().any(|a| *a == Attribute::AlbumId) {
            return Err(PidSpecError {
                spec: source.to_owned(),
                // Neither the album's own spec nor the artist's may name it:
                // both would be asking the album for an answer that depends on
                // themselves. The caller names which one it was parsing.
                reason: "`albumid` cannot appear in this spec",
            });
        }
        Ok(Self {
            groups,
            source: source.to_owned(),
        })
    }

    /// The spec as written, which is what gets persisted and compared.
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// The three specs an instance runs under.
#[derive(Debug, Clone)]
pub struct PidSpecs {
    pub album: PidSpec,
    pub track: PidSpec,
    pub artist: PidSpec,
}

impl PidSpecs {
    pub fn album_id(&self, library: Uuid, source: &impl PidSource) -> Uuid {
        hash_parts(&[&library.to_string(), &self.evaluate(&self.album, source)])
    }

    pub fn track_id(&self, library: Uuid, source: &impl PidSource) -> Uuid {
        hash_parts(&[&library.to_string(), &self.evaluate(&self.track, source)])
    }

    /// The identity of one artist name.
    ///
    /// The library is folded in, unlike the reference: our `artist` rows are
    /// per library, so hashing the name alone would make the same artist in two
    /// libraries collide on the primary key.
    pub fn artist_id(&self, library: Uuid, name: &str) -> Uuid {
        let source = ArtistNameSource { name };
        hash_parts(&[&library.to_string(), &self.evaluate(&self.artist, &source)])
    }

    /// The first group holding at least one non-empty attribute wins, and all
    /// of that group's values — the empty ones included — are joined, so a
    /// missing middle attribute keeps its position instead of shifting the rest.
    fn evaluate(&self, spec: &PidSpec, source: &impl PidSource) -> String {
        for group in &spec.groups {
            let values: Vec<String> = group
                .iter()
                .map(|attribute| self.resolve(attribute, source))
                .collect();
            if values.iter().any(|value| !value.is_empty()) {
                return values.join(ATTRIBUTE_JOINER);
            }
        }
        String::new()
    }

    fn resolve(&self, attribute: &Attribute, source: &impl PidSource) -> String {
        match attribute {
            // Unprefixed by the library on purpose: the track spec already
            // carries it, and prefixing twice would make a track's identity
            // depend on the library string in two places at once.
            Attribute::AlbumId => hash_parts(&[&self.evaluate(&self.album, source)]).to_string(),
            Attribute::Folder => source.folder().to_owned(),
            Attribute::AlbumArtistId => {
                hash_parts(&[&str_clear(&source.album_artist_display().to_lowercase())]).to_string()
            }
            Attribute::Title => source.title().to_owned(),
            Attribute::Album => str_clear(&source.album().to_lowercase()),
            Attribute::Tag(name) => source.tag(name).unwrap_or_default().to_owned(),
        }
    }
}

/// A bare artist name, for the artist spec.
struct ArtistNameSource<'a> {
    name: &'a str,
}

impl PidSource for ArtistNameSource<'_> {
    fn tag(&self, _name: &str) -> Option<&str> {
        None
    }
    fn folder(&self) -> &str {
        ""
    }
    fn album_artist_display(&self) -> &str {
        self.name
    }
    fn title(&self) -> &str {
        ""
    }
    fn album(&self) -> &str {
        ""
    }
}

/// Folds typographic punctuation onto its ASCII form, and nothing else.
///
/// Deliberately far lighter than `canonical_name`: spaces, punctuation and
/// accents all survive, because this feeds an identity rather than a match.
/// Two names differing by a real character are two artists — which is why
/// "AC/DC" and "AC DC" are no longer folded together, matching the reference.
pub fn str_clear(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\u{2018}' | '\u{2019}' | '\u{201b}' | '\u{2032}' => '\u{27}',
            '\u{ff02}' | '\u{3003}' | '\u{02ee}' | '\u{05f4}' | '\u{2033}' | '\u{2036}'
            | '\u{02f6}' | '\u{02ba}' | '\u{201c}' | '\u{201d}' | '\u{02dd}' | '\u{201f}' => {
                '\u{22}'
            }
            '\u{2010}' | '\u{2013}' | '\u{2014}' | '\u{2212}' | '\u{2015}' => '-',
            other => other,
        })
        .collect()
}

/// Hashes the parts into a UUID.
///
/// MD5, and it is not a security choice: it is a deduplication digest that
/// yields the sixteen bytes a UUID needs without truncation, which is what lets
/// the reference's own vectors be checked against this implementation. The
/// result is stamped as a version 8 UUID (RFC 9562 §5.8, custom) so it is
/// self-describing beside the version 4 ids the rest of the schema mints.
fn hash_parts(parts: &[&str]) -> Uuid {
    let mut hasher = Md5::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(PART_SEPARATOR.as_bytes());
    }
    Uuid::new_v8(hasher.finalize().into())
}

#[cfg(test)]
mod tests;
