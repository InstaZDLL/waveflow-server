//! The identity specs: the defaults, and which of them may name `albumid`.

/// The identity rules an instance runs under unless it says otherwise.
///
/// Taken verbatim from Navidrome, which is the reference these follow: a
/// release identifier when the files carry one, otherwise the album artist,
/// title, version and date together.
pub(super) const DEFAULT_PID_ALBUM: &str =
    "musicbrainz_albumid|albumartistid,album,albumversion,releasedate";
pub(super) const DEFAULT_PID_TRACK: &str =
    "musicbrainz_trackid|albumid,discnumber,tracknumber,title";
pub(super) const DEFAULT_PID_ARTIST: &str = "albumartistid";

/// Whether a spec is allowed to name `albumid`.
///
/// The album's own spec is not, and neither is the artist's: both would be
/// asking the album for an answer that depends on themselves.
pub(super) enum PidSpecKind {
    MayNotReferenceAlbumId,
    MayReferenceAlbumId,
}

impl PidSpecKind {
    fn allows_album_id(&self) -> bool {
        matches!(self, Self::MayReferenceAlbumId)
    }
}

pub(super) fn parse_pid_spec(
    name: &str,
    default: &str,
    kind: PidSpecKind,
) -> anyhow::Result<crate::pid::PidSpec> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
    crate::pid::PidSpec::parse(&raw, kind.allows_album_id())
        .map_err(|error| anyhow::anyhow!("invalid {name}: {error}"))
}

fn default_pid_spec(default: &str, kind: PidSpecKind) -> crate::pid::PidSpec {
    crate::pid::PidSpec::parse(default, kind.allows_album_id()).expect("built-in pid spec default")
}

pub(super) fn default_pid_specs() -> crate::pid::PidSpecs {
    crate::pid::PidSpecs {
        album: default_pid_spec(DEFAULT_PID_ALBUM, PidSpecKind::MayNotReferenceAlbumId),
        track: default_pid_spec(DEFAULT_PID_TRACK, PidSpecKind::MayReferenceAlbumId),
        artist: default_pid_spec(DEFAULT_PID_ARTIST, PidSpecKind::MayNotReferenceAlbumId),
    }
}
