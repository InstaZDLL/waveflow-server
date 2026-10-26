//! Corrections to a track's tags, and the editor's view of them.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Corrections to a track's tags, field by field.
///
/// A real partial patch, in three states: **a field left out leaves that
/// correction as it is**, `null` removes it, and a value sets it. A blank string
/// reads as `null`, because a track with no title is not a correction anyone
/// wants to store. The two lists follow the same three states, with one
/// difference: `[]` is a value, since saying a track credits nobody is a
/// correction rather than the absence of one.
///
/// It was wholesale until #177 — the body was every correction the track should
/// carry afterwards — and that silently dropped whatever a client did not
/// mention, including corrections another client had made. The only client that
/// sent this patch always spelled every field out, `null` included, so the
/// change leaves its requests meaning what they meant.
///
/// `Option<Option<T>>` is what makes the three states expressible. Under plain
/// serde an absent field and a `null` one both deserialise to `None`, and the
/// difference between them is the whole contract: the outer `None` is absent,
/// `Some(None)` is `null`, `Some(Some(value))` is a value.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct TrackMetadataPatch {
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub title: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub sort_title: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    pub year: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    pub track_number: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    pub disc_number: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub musicbrainz_recording_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub comment: Option<Option<String>>,
    /// The track's artists, in order, and its genres. Explicit lists rather
    /// than the `;`-joined string a file carries: that form exists because a
    /// tagger writes names however it likes and the mapper has to guess where
    /// one ends, which is not a guess worth reintroducing on a list someone
    /// typed on purpose.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Vec<String>>, nullable)]
    pub artists: Option<Option<Vec<String>>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Vec<String>>, nullable)]
    pub genres: Option<Option<Vec<String>>>,
}

/// Tells a field that is in the body from one that is not.
///
/// Serde calls this only for a field present in the body — `#[serde(default)]`
/// supplies the outer `None` when it is absent — so wrapping whatever the field
/// holds in `Some` is exactly the distinction: `Some(None)` for `null`,
/// `Some(Some(value))` for a value.
pub(crate) fn present<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// What a track's tags say, and what a correction says instead.
///
/// The editor's view of [`TrackMetadataPatch`]: every field it can write, with
/// the file's own value beside the correction standing over it. The catalogue
/// never shows this — it answers `effective`, which is `override ?? source` —
/// and nothing else needs it. It exists because an editor cannot offer to
/// restore a value it cannot read, nor show which fields are corrected. It no
/// longer has to guard a write: [`TrackMetadataPatch`] leaves alone what it
/// does not mention.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TrackOverrides {
    /// What the file said, for the fields where the database still knows.
    pub source: TrackSourceTags,
    /// The correction, field by field. `null` means no correction on that
    /// field, which is what the row's nullable columns mean.
    pub overrides: TrackOverrideValues,
}

/// The file's own values, as the last scan read them.
///
/// **Artists and genres are absent, and cannot be added here.** They are more
/// than columns: a correction to either is *materialised* into
/// `track_participant`, `track_genre` and the `*_display` strings, because
/// those rows feed every projection and the search index. Applying it
/// overwrites what the scan read, so once a correction exists the file's own
/// credits are no longer in the database — only in the file. The seven fields
/// below survive because the projection merges them with `COALESCE` instead,
/// leaving the scanned column untouched.
///
/// Restoring a corrected list therefore re-reads the file, which
/// `set_track_metadata` already does. What cannot be done cheaply is *showing*
/// it beforehand, so an editor offers the reset without previewing its result.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TrackSourceTags {
    pub title: String,
    pub sort_title: Option<String>,
    pub year: Option<i64>,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub musicbrainz_recording_id: Option<String>,
    pub comment: Option<String>,
}

/// The `track_override` row as it stands, not a projection of it.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct TrackOverrideValues {
    pub title: Option<String>,
    pub sort_title: Option<String>,
    pub year: Option<i64>,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub musicbrainz_recording_id: Option<String>,
    pub comment: Option<String>,
    pub artists: Option<Vec<String>>,
    pub genres: Option<Vec<String>>,
}
