use super::*;

struct Fixture {
    tags: Vec<(&'static str, &'static str)>,
    folder: &'static str,
    album_artist: &'static str,
    title: &'static str,
    album: &'static str,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            tags: Vec::new(),
            folder: "/music/album",
            album_artist: "Album Artist",
            title: "Track Title",
            album: "Album Name",
        }
    }
}

impl PidSource for Fixture {
    fn tag(&self, name: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    }
    fn folder(&self) -> &str {
        self.folder
    }
    fn album_artist_display(&self) -> &str {
        self.album_artist
    }
    fn title(&self) -> &str {
        self.title
    }
    fn album(&self) -> &str {
        self.album
    }
}

fn default_specs() -> PidSpecs {
    PidSpecs {
        album: PidSpec::parse(
            "musicbrainz_albumid|albumartistid,album,albumversion,releasedate",
            false,
        )
        .expect("album spec"),
        track: PidSpec::parse(
            "musicbrainz_trackid|albumid,discnumber,tracknumber,title",
            true,
        )
        .expect("track spec"),
        artist: PidSpec::parse("albumartistid", false).expect("artist spec"),
    }
}

const LIBRARY: Uuid = Uuid::from_u128(1);

#[test]
fn the_first_group_holding_a_value_wins() {
    let specs = default_specs();
    let untagged = Fixture::default();
    let tagged = Fixture {
        tags: vec![("musicbrainz_albumid", "1234")],
        ..Fixture::default()
    };
    assert_ne!(
        specs.album_id(LIBRARY, &untagged),
        specs.album_id(LIBRARY, &tagged),
        "a release id answers instead of the fallback group"
    );
    // And a tag no group names moves nothing.
    let unrelated = Fixture {
        tags: vec![("comment", "anything")],
        ..Fixture::default()
    };
    assert_eq!(
        specs.album_id(LIBRARY, &untagged),
        specs.album_id(LIBRARY, &unrelated)
    );
}

/// The defect this engine exists to make impossible: a joined credit used
/// to collapse onto the row id of its first artist.
#[test]
fn a_joined_credit_identifies_the_album_by_the_whole_string() {
    let specs = default_specs();
    let of = |credit: &'static str| {
        specs.album_id(
            LIBRARY,
            &Fixture {
                album_artist: credit,
                ..Fixture::default()
            },
        )
    };
    assert_ne!(of("Nova Kern; Lior Sand"), of("Nova Kern; Ada Vale"));
    assert_ne!(of("Nova Kern"), of("Nova Kern; Lior Sand"));
    // The boundary between credits is part of the identity, which the old
    // canonical form erased.
    assert_ne!(of("A; B C"), of("A; B; C"));
}

#[test]
fn punctuation_and_accents_survive_normalisation() {
    assert_eq!(str_clear("Sgt. Pepper\u{2019}s"), "Sgt. Pepper's");
    assert_eq!(str_clear("Beyonc\u{e9}"), "Beyonc\u{e9}");
    assert_eq!(str_clear("AC/DC"), "AC/DC");
    assert_eq!(str_clear("Post\u{2014}Rock"), "Post-Rock");
    assert_ne!(str_clear("AC/DC"), str_clear("AC DC"));
}

#[test]
fn the_library_separates_two_catalogues_holding_the_same_file() {
    let specs = default_specs();
    let other = Uuid::from_u128(2);
    assert_ne!(
        specs.album_id(LIBRARY, &Fixture::default()),
        specs.album_id(other, &Fixture::default())
    );
    assert_ne!(
        specs.artist_id(LIBRARY, "Nova Kern"),
        specs.artist_id(other, "Nova Kern")
    );
}

#[test]
fn a_track_follows_its_album() {
    let specs = default_specs();
    let here = Fixture::default();
    let elsewhere = Fixture {
        album: "Another Album",
        ..Fixture::default()
    };
    assert_ne!(
        specs.track_id(LIBRARY, &here),
        specs.track_id(LIBRARY, &elsewhere),
        "`albumid` reaches the album spec"
    );
}

#[test]
fn a_recursive_album_spec_is_refused() {
    assert!(PidSpec::parse("albumid,album", false).is_err());
    assert!(PidSpec::parse("albumid,title", true).is_ok());
    assert!(PidSpec::parse("", false).is_err());
}

#[test]
fn every_identifier_is_a_version_eight_uuid() {
    let specs = default_specs();
    assert_eq!(
        specs
            .album_id(LIBRARY, &Fixture::default())
            .get_version_num(),
        8
    );
    assert_eq!(specs.artist_id(LIBRARY, "Nova Kern").get_version_num(), 8);
}
