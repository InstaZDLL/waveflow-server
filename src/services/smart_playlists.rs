//! Smart playlists: a playlist defined by rules, evaluated when it is read
//! (RFC-025 decisions 1 to 6).
//!
//! Three layers stand between a rule a client sent and the SQL that answers it,
//! and the RFC explains why each is there:
//!
//! 1. **An allowlist parse.** [`SmartRules::parse`] reads the request as JSON
//!    and accepts only the fields, operators and value types named in this
//!    file. Anything else is refused with the path of the offending value, and
//!    nothing that was refused is stored. The service methods take the parsed
//!    type, never the JSON, so there is no way to reach the database with rules
//!    that skipped the parse.
//! 2. **One static statement.** [`EVALUATE`] is a single literal, assembled by
//!    `concat!` like every projection here. The rules reach it as *one bound
//!    value* — a JSON document SQLite takes apart with `json_each` — and every
//!    field name, operator and logical connective in it is compared against a
//!    string literal inside a `CASE`. No byte of a rule is ever SQL text.
//! 3. **Tenancy in the statement.** It wraps `song_select!`, so the first
//!    bind is an account id and only tracks of that account's libraries can
//!    match. A playlist is evaluated as its owner, then read through the
//!    viewer's own libraries (decision 5).
//!
//! Ampache 8.4.0 shipped an SQL injection in exactly this feature, because the
//! logical operator joining the rules was concatenated into the statement. Here
//! the connective is an enum parsed from `"all"` or `"any"`, re-serialised by
//! this code, bound, and compared with `p.mode = 'all'`.

use super::*;

/// The most tracks one smart playlist answers, and the default when it names
/// no limit.
///
/// A smart playlist is evaluated on every read, and `getPlaylists` reads every
/// one an account can see; a bound on what one evaluation may return is what
/// keeps that listing's cost proportional to the playlists rather than to the
/// catalogue.
pub const MAX_SMART_PLAYLIST_TRACKS: u32 = 1_000;

/// How many conditions one playlist may carry.
pub const MAX_SMART_CONDITIONS: usize = 20;

/// How many smart playlists one account may keep.
const MAX_SMART_PLAYLISTS: i64 = 100;

/// How many virtual-machine instructions one evaluation may spend, and how
/// often it is asked whether to go on (decision 4).
///
/// Two hundred million is several seconds of work on a catalogue of a hundred
/// thousand tracks with play-count rules — enough for any rule set over a
/// real library, and a ceiling on what one request can make the server do.
pub const SMART_EVALUATION_BUDGET_OPS: u64 = 200_000_000;
const BUDGET_STEP_OPS: i32 = 1_000;

/// The same bound in time, whichever is reached first: a slow disk spends
/// time without spending instructions.
const SMART_EVALUATION_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// How many (playlist, reader) answers the listing memory holds before it is
/// emptied. A bound on memory, not on correctness: an entry that is gone is a
/// playlist listed without counts until it is opened again.
const MAX_REMEMBERED_EXTENTS: usize = 10_000;

/// What one reader was last shown of one smart playlist, which is all a
/// listing reports (decision 4).
#[derive(Debug, Clone, Copy)]
pub(super) struct RememberedExtent {
    /// The playlist's `updated_at` when it was evaluated: a rule edit moves
    /// it, and an entry from before the edit is not reported.
    updated_at: i64,
    evaluated_at: i64,
    song_count: i64,
    duration_ms: i64,
}

/// The smart playlist half of the services' runtime state.
#[derive(Clone)]
pub(super) struct SmartRuntime {
    extents: Arc<dashmap::DashMap<(Uuid, Uuid), RememberedExtent>>,
    evaluations: Arc<std::sync::atomic::AtomicU64>,
    budget_ops: Arc<std::sync::atomic::AtomicU64>,
}

impl SmartRuntime {
    pub(super) fn new() -> Self {
        Self {
            extents: Arc::new(dashmap::DashMap::new()),
            evaluations: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            budget_ops: Arc::new(std::sync::atomic::AtomicU64::new(
                SMART_EVALUATION_BUDGET_OPS,
            )),
        }
    }
}

/// The longest text a condition may compare against, in characters.
const MAX_RULE_TEXT_CHARS: usize = 200;

/// The longest comment a smart playlist may carry, in characters.
const MAX_SMART_COMMENT_CHARS: usize = 2_000;

/// One day, in milliseconds: what `in_the_last` counts in.
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// How the conditions combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SmartMatch {
    /// Every condition holds.
    All,
    /// At least one condition holds.
    Any,
}

/// A field a condition may test. The allowlist of decision 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmartField {
    Title,
    Album,
    /// Any artist the track credits in its `artist` role.
    Artist,
    /// Any artist the track's album is credited to.
    AlbumArtist,
    /// Any of the track's genres.
    Genre,
    Year,
    /// The owner's own rating, 0 for none.
    Rating,
    /// Whether the owner starred the track.
    Starred,
    /// The owner's own completed plays.
    PlayCount,
    /// The owner's own last completed play.
    LastPlayed,
    /// When the scanner first saw the file.
    DateAdded,
    /// Seconds.
    Duration,
    /// Kilobits per second.
    Bitrate,
    /// The file's extension: `flac`, `mp3`…
    Format,
    /// A library, by id.
    Library,
}

/// An operator. Which ones a field admits is decided by its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmartOperator {
    Is,
    IsNot,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    GreaterThan,
    LessThan,
    /// Inclusive, `[low, high]`.
    Between,
    /// A number of days back from the moment of evaluation.
    InTheLast,
    NotInTheLast,
    /// A calendar date, `YYYY-MM-DD`, midnight UTC.
    Before,
    After,
}

/// What a rule set may be sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmartSort {
    Title,
    Artist,
    Album,
    Year,
    Rating,
    PlayCount,
    LastPlayed,
    DateAdded,
    Duration,
    Bitrate,
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SmartOrder {
    #[default]
    Asc,
    Desc,
}

/// One condition, as validated. `value` is the value the client sent, kept as
/// it was so the editor reads back what it wrote; its type has been checked
/// against the field and the operator.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct SmartCondition {
    pub field: SmartField,
    pub operator: SmartOperator,
    #[schema(value_type = Object)]
    pub value: serde_json::Value,
}

/// A validated rule set. Only [`SmartRules::parse`] builds one.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct SmartRules {
    #[serde(rename = "match")]
    pub match_kind: SmartMatch,
    pub conditions: Vec<SmartCondition>,
    pub sort: Option<SmartSort>,
    pub order: SmartOrder,
    /// At most [`MAX_SMART_PLAYLIST_TRACKS`]; that many when absent.
    pub limit: Option<u32>,
}

/// Why a rule set was refused: where, and what was wrong there.
///
/// `at` is a path into the request — `conditions[2].operator` — so a client can
/// put the message beside the control that produced it. `reason` is one of a
/// fixed set of sentences: nothing the client sent is ever echoed back in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    pub at: String,
    pub reason: &'static str,
}

impl RuleError {
    fn new(at: impl Into<String>, reason: &'static str) -> Self {
        Self {
            at: at.into(),
            reason,
        }
    }
}

impl std::fmt::Display for RuleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.at, self.reason)
    }
}

/// The kind of value a field holds, which decides its operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    /// Several values per track: any one of them may match.
    TextSet,
    Number {
        min: i64,
        max: i64,
    },
    Date,
    Flag,
    Extension,
    Identifier,
}

impl SmartField {
    const ALL: [(&'static str, SmartField); 15] = [
        ("title", Self::Title),
        ("album", Self::Album),
        ("artist", Self::Artist),
        ("album_artist", Self::AlbumArtist),
        ("genre", Self::Genre),
        ("year", Self::Year),
        ("rating", Self::Rating),
        ("starred", Self::Starred),
        ("play_count", Self::PlayCount),
        ("last_played", Self::LastPlayed),
        ("date_added", Self::DateAdded),
        ("duration", Self::Duration),
        ("bitrate", Self::Bitrate),
        ("format", Self::Format),
        ("library", Self::Library),
    ];

    fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, field)| *field == self)
            .map(|(name, _)| *name)
            .expect("every field is named")
    }

    fn kind(self) -> Kind {
        match self {
            Self::Title | Self::Album => Kind::Text,
            Self::Artist | Self::AlbumArtist | Self::Genre => Kind::TextSet,
            Self::Year => Kind::Number { min: 0, max: 9_999 },
            Self::Rating => Kind::Number { min: 0, max: 5 },
            Self::PlayCount => Kind::Number {
                min: 0,
                max: 1_000_000_000,
            },
            Self::Duration => Kind::Number {
                min: 0,
                max: 1_000_000,
            },
            Self::Bitrate => Kind::Number {
                min: 0,
                max: 100_000,
            },
            Self::LastPlayed | Self::DateAdded => Kind::Date,
            Self::Starred => Kind::Flag,
            Self::Format => Kind::Extension,
            Self::Library => Kind::Identifier,
        }
    }
}

impl SmartOperator {
    const ALL: [(&'static str, SmartOperator); 13] = [
        ("is", Self::Is),
        ("is_not", Self::IsNot),
        ("contains", Self::Contains),
        ("not_contains", Self::NotContains),
        ("starts_with", Self::StartsWith),
        ("ends_with", Self::EndsWith),
        ("greater_than", Self::GreaterThan),
        ("less_than", Self::LessThan),
        ("between", Self::Between),
        ("in_the_last", Self::InTheLast),
        ("not_in_the_last", Self::NotInTheLast),
        ("before", Self::Before),
        ("after", Self::After),
    ];

    /// Whether a field of `kind` may be tested this way.
    fn admitted_for(self, kind: Kind) -> bool {
        match kind {
            Kind::Text | Kind::TextSet => matches!(
                self,
                Self::Is
                    | Self::IsNot
                    | Self::Contains
                    | Self::NotContains
                    | Self::StartsWith
                    | Self::EndsWith
            ),
            Kind::Number { .. } => matches!(
                self,
                Self::Is | Self::IsNot | Self::GreaterThan | Self::LessThan | Self::Between
            ),
            Kind::Date => matches!(
                self,
                Self::InTheLast | Self::NotInTheLast | Self::Before | Self::After
            ),
            Kind::Flag => self == Self::Is,
            Kind::Extension | Kind::Identifier => matches!(self, Self::Is | Self::IsNot),
        }
    }
}

impl SmartSort {
    /// The sort keys, in the order their codes are numbered: entry `i` sorts
    /// ascending under code `2i + 1` and descending under `2i + 2`. The codes
    /// are what [`EVALUATE`] compares against, so this order and the `ORDER BY`
    /// list there are one table written twice; `the_sort_codes_match_the_statement`
    /// holds them together.
    const ALL: [(&'static str, SmartSort); 11] = [
        ("title", Self::Title),
        ("artist", Self::Artist),
        ("album", Self::Album),
        ("year", Self::Year),
        ("rating", Self::Rating),
        ("play_count", Self::PlayCount),
        ("last_played", Self::LastPlayed),
        ("date_added", Self::DateAdded),
        ("duration", Self::Duration),
        ("bitrate", Self::Bitrate),
        ("random", Self::Random),
    ];
}

/// The code [`EVALUATE`] reads for "no sort": its fixed tie-break alone.
const SORT_NONE: i64 = 0;
/// The code for a random order, whatever direction was asked.
const SORT_RANDOM: i64 = 99;

fn lookup<T: Copy>(table: &[(&'static str, T)], name: &str) -> Option<T> {
    table
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, value)| *value)
}

impl SmartRules {
    /// Validates a rule set against the allowlist.
    ///
    /// Every key is checked, not only the ones read: an object carrying a key
    /// this server does not know is refused rather than half-understood, so a
    /// client that sends a field a later version adds learns it is not
    /// supported instead of receiving a playlist that silently ignored it.
    pub fn parse(value: &serde_json::Value) -> Result<Self, RuleError> {
        let object = value
            .as_object()
            .ok_or_else(|| RuleError::new("rules", "must be an object"))?;
        for key in object.keys() {
            if !matches!(
                key.as_str(),
                "match" | "conditions" | "sort" | "order" | "limit"
            ) {
                return Err(RuleError::new(
                    "rules",
                    "carries a key that is not supported",
                ));
            }
        }
        let match_kind = match object.get("match") {
            None => SmartMatch::All,
            Some(serde_json::Value::String(kind)) if kind == "all" => SmartMatch::All,
            Some(serde_json::Value::String(kind)) if kind == "any" => SmartMatch::Any,
            Some(_) => return Err(RuleError::new("match", "must be \"all\" or \"any\"")),
        };
        let conditions = match object.get("conditions") {
            None => Vec::new(),
            Some(serde_json::Value::Array(items)) => {
                if items.len() > MAX_SMART_CONDITIONS {
                    return Err(RuleError::new(
                        "conditions",
                        "holds more than 20 conditions",
                    ));
                }
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| parse_condition(index, item))
                    .collect::<Result<Vec<_>, _>>()?
            }
            Some(_) => return Err(RuleError::new("conditions", "must be a list")),
        };
        let sort = match object.get("sort") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(name)) => Some(
                lookup(&SmartSort::ALL, name)
                    .ok_or_else(|| RuleError::new("sort", "is not a field this server sorts by"))?,
            ),
            Some(_) => return Err(RuleError::new("sort", "must be a field name")),
        };
        let order = match object.get("order") {
            None | Some(serde_json::Value::Null) => SmartOrder::Asc,
            Some(serde_json::Value::String(order)) if order == "asc" => SmartOrder::Asc,
            Some(serde_json::Value::String(order)) if order == "desc" => SmartOrder::Desc,
            Some(_) => return Err(RuleError::new("order", "must be \"asc\" or \"desc\"")),
        };
        let limit = match object.get("limit") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => {
                let limit = value
                    .as_u64()
                    .filter(|limit| (1..=u64::from(MAX_SMART_PLAYLIST_TRACKS)).contains(limit))
                    .ok_or_else(|| {
                        RuleError::new("limit", "must be a whole number from 1 to 1000")
                    })?;
                Some(u32::try_from(limit).expect("bounded above"))
            }
        };
        Ok(Self {
            match_kind,
            conditions,
            sort,
            order,
            limit,
        })
    }

    /// The canonical form stored in `rules_json`: this type's own
    /// serialization, never the request as it arrived.
    fn to_stored(&self) -> String {
        serde_json::to_string(self).expect("rule serialization cannot fail")
    }

    /// The code [`EVALUATE`] orders by.
    fn sort_code(&self) -> i64 {
        match self.sort {
            None => SORT_NONE,
            Some(SmartSort::Random) => SORT_RANDOM,
            Some(sort) => {
                let index = SmartSort::ALL
                    .iter()
                    .position(|(_, candidate)| *candidate == sort)
                    .expect("every sort is listed") as i64;
                index * 2
                    + match self.order {
                        SmartOrder::Asc => 1,
                        SmartOrder::Desc => 2,
                    }
            }
        }
    }

    /// The conditions as the statement reads them: one object per condition,
    /// its field and an elementary operator spelled as SQL compares them, and
    /// the operand pre-computed. Built from the validated types alone.
    ///
    /// `now` is the moment of evaluation, read once, so `in_the_last` means the
    /// same thing for every row of one read.
    fn bound_conditions(&self, now: i64) -> String {
        let bound: Vec<serde_json::Value> = self
            .conditions
            .iter()
            .map(|condition| bind_condition(condition, now))
            .collect();
        serde_json::to_string(&bound).expect("condition serialization cannot fail")
    }
}

fn parse_condition(index: usize, item: &serde_json::Value) -> Result<SmartCondition, RuleError> {
    let at = |key: &str| format!("conditions[{index}].{key}");
    let object = item
        .as_object()
        .ok_or_else(|| RuleError::new(format!("conditions[{index}]"), "must be an object"))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "field" | "operator" | "value") {
            return Err(RuleError::new(
                format!("conditions[{index}]"),
                "carries a key that is not supported",
            ));
        }
    }
    let field = object
        .get("field")
        .and_then(serde_json::Value::as_str)
        .and_then(|name| lookup(&SmartField::ALL, name))
        .ok_or_else(|| RuleError::new(at("field"), "is not a field this server knows"))?;
    let operator = object
        .get("operator")
        .and_then(serde_json::Value::as_str)
        .and_then(|name| lookup(&SmartOperator::ALL, name))
        .ok_or_else(|| RuleError::new(at("operator"), "is not an operator this server knows"))?;
    let kind = field.kind();
    if !operator.admitted_for(kind) {
        return Err(RuleError::new(
            at("operator"),
            "does not apply to this field",
        ));
    }
    let value = object
        .get("value")
        .ok_or_else(|| RuleError::new(at("value"), "is missing"))?;
    let invalid = |reason| Err(RuleError::new(at("value"), reason));
    match kind {
        Kind::Text | Kind::TextSet => match value.as_str() {
            Some(text)
                if !text.trim().is_empty() && text.chars().count() <= MAX_RULE_TEXT_CHARS => {}
            _ => return invalid("must be text of 1 to 200 characters"),
        },
        Kind::Number { min, max } => {
            let in_range = |value: &serde_json::Value| {
                value.as_i64().filter(|number| (min..=max).contains(number))
            };
            if operator == SmartOperator::Between {
                let pair = value
                    .as_array()
                    .filter(|pair| pair.len() == 2)
                    .and_then(|pair| Some((in_range(&pair[0])?, in_range(&pair[1])?)));
                match pair {
                    Some((low, high)) if low <= high => {}
                    _ => return invalid("must be two whole numbers in range, the lower first"),
                }
            } else if in_range(value).is_none() {
                return invalid("must be a whole number in this field's range");
            }
        }
        Kind::Date => match operator {
            SmartOperator::InTheLast | SmartOperator::NotInTheLast => {
                if value
                    .as_i64()
                    .filter(|days| (1..=36_500).contains(days))
                    .is_none()
                {
                    return invalid("must be a number of days from 1 to 36500");
                }
            }
            _ => {
                if value.as_str().and_then(date_ms).is_none() {
                    return invalid("must be a date written YYYY-MM-DD");
                }
            }
        },
        Kind::Flag => {
            if !value.is_boolean() {
                return invalid("must be true or false");
            }
        }
        Kind::Extension => match value.as_str() {
            Some(extension)
                if (1..=10).contains(&extension.len())
                    && extension
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()) => {}
            _ => return invalid("must be a file extension in lower case, such as flac"),
        },
        Kind::Identifier => {
            if value
                .as_str()
                .and_then(|id| Uuid::parse_str(id).ok())
                .is_none()
            {
                return invalid("must be a library id");
            }
        }
    }
    Ok(SmartCondition {
        field,
        operator,
        value: value.clone(),
    })
}

/// Midnight UTC of a `YYYY-MM-DD` date, in epoch milliseconds.
fn date_ms(value: &str) -> Option<i64> {
    let date = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis())
}

/// One condition as [`EVALUATE`] binds it.
///
/// `o` is the elementary comparison: `eq`, `ne`, `has`, `hasnt`, `sw`, `ew` for
/// text; `eq`, `ne`, `gt`, `lt`, `bt` for numbers; `ge` and `lt` for dates.
/// `v` is the text operand, `k` its canonical form (what the catalogue matches
/// genres and artists on), `n` and `m` the numeric operands.
fn bind_condition(condition: &SmartCondition, now: i64) -> serde_json::Value {
    use SmartOperator as Op;
    let text = condition.value.as_str().unwrap_or_default();
    let (o, v, k, n, m): (
        &str,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
    ) = match condition.field.kind() {
        Kind::Text | Kind::TextSet => {
            let o = match condition.operator {
                Op::Is => "eq",
                Op::IsNot => "ne",
                Op::Contains => "has",
                Op::NotContains => "hasnt",
                Op::StartsWith => "sw",
                _ => "ew",
            };
            (
                o,
                Some(text.to_owned()),
                Some(waveflow_core::scanner::canonical_name(text)),
                None,
                None,
            )
        }
        Kind::Number { .. } => match condition.operator {
            Op::Between => {
                let pair = condition.value.as_array().cloned().unwrap_or_default();
                (
                    "bt",
                    None,
                    None,
                    pair.first().and_then(serde_json::Value::as_i64),
                    pair.get(1).and_then(serde_json::Value::as_i64),
                )
            }
            operator => (
                match operator {
                    Op::Is => "eq",
                    Op::IsNot => "ne",
                    Op::GreaterThan => "gt",
                    _ => "lt",
                },
                None,
                None,
                condition.value.as_i64(),
                None,
            ),
        },
        Kind::Date => match condition.operator {
            Op::InTheLast | Op::NotInTheLast => {
                let days = condition.value.as_i64().unwrap_or(0);
                let since = now.saturating_sub(days.saturating_mul(DAY_MS));
                (
                    if condition.operator == Op::InTheLast {
                        "ge"
                    } else {
                        "lt"
                    },
                    None,
                    None,
                    Some(since),
                    None,
                )
            }
            operator => (
                if operator == Op::After { "ge" } else { "lt" },
                None,
                None,
                date_ms(text).map(|ms| {
                    // `after 2024-01-01` excludes that day: from the next
                    // midnight on.
                    if operator == Op::After {
                        ms + DAY_MS
                    } else {
                        ms
                    }
                }),
                None,
            ),
        },
        Kind::Flag => (
            "eq",
            None,
            None,
            Some(i64::from(condition.value.as_bool().unwrap_or(false))),
            None,
        ),
        Kind::Extension | Kind::Identifier => (
            if condition.operator == Op::Is {
                "eq"
            } else {
                "ne"
            },
            Some(text.to_owned()),
            None,
            None,
            None,
        ),
    };
    serde_json::json!({
        "f": condition.field.name(),
        "o": o,
        "v": v,
        "k": k,
        "n": n,
        "m": m,
    })
}

/// A text comparison on one column, by the bound condition `c`. ASCII case is
/// folded, as `NOCASE` folds it everywhere else in the catalogue.
macro_rules! smart_text_test {
    ($column:literal) => {
        concat!(
            "(CASE c.o \
               WHEN 'eq' THEN lower(COALESCE(",
            $column,
            ", '')) = lower(c.v) \
               WHEN 'ne' THEN lower(COALESCE(",
            $column,
            ", '')) <> lower(c.v) \
               WHEN 'has' THEN instr(lower(COALESCE(",
            $column,
            ", '')), lower(c.v)) > 0 \
               WHEN 'hasnt' THEN instr(lower(COALESCE(",
            $column,
            ", '')), lower(c.v)) = 0 \
               WHEN 'sw' THEN substr(lower(COALESCE(",
            $column,
            ", '')), 1, length(c.v)) = lower(c.v) \
               WHEN 'ew' THEN length(COALESCE(",
            $column,
            ", '')) >= length(c.v) \
                 AND substr(lower(COALESCE(",
            $column,
            ", '')), -length(c.v)) = lower(c.v) \
               ELSE 0 END)"
        )
    };
}

/// A text comparison against a set of names, any one of which may match. The
/// negative operators hold when *none* does: "artist is not X" on a track
/// crediting X and Y is false.
///
/// `$from` is a `FROM … WHERE …` naming the set for the row `s`; `$name` and
/// `$canonical` its display and canonical columns.
macro_rules! smart_set_test {
    ($from:literal, $name:literal, $canonical:literal) => {
        concat!(
            "(CASE WHEN c.o IN ('ne', 'hasnt') THEN NOT EXISTS (SELECT 1 ",
            $from,
            " AND ",
            smart_set_match!($name, $canonical),
            ") ELSE EXISTS (SELECT 1 ",
            $from,
            " AND ",
            smart_set_match!($name, $canonical),
            ") END)"
        )
    };
}

macro_rules! smart_set_match {
    ($name:literal, $canonical:literal) => {
        concat!(
            "(CASE WHEN c.o IN ('eq', 'ne') THEN ",
            $canonical,
            " = c.k \
               WHEN c.o IN ('has', 'hasnt') THEN instr(lower(",
            $name,
            "), lower(c.v)) > 0 \
               WHEN c.o = 'sw' THEN substr(lower(",
            $name,
            "), 1, length(c.v)) = lower(c.v) \
               WHEN c.o = 'ew' THEN length(",
            $name,
            ") >= length(c.v) \
                 AND substr(lower(",
            $name,
            "), -length(c.v)) = lower(c.v) \
               ELSE 0 END)"
        )
    };
}

/// A numeric comparison on one column. A missing value fails every test but
/// `ne`, which is what "year is not 1999" means of an undated track.
macro_rules! smart_number_test {
    ($column:literal) => {
        concat!(
            "(CASE c.o \
               WHEN 'eq' THEN ",
            $column,
            " = c.n \
               WHEN 'ne' THEN ",
            $column,
            " IS NOT c.n \
               WHEN 'gt' THEN ",
            $column,
            " > c.n \
               WHEN 'lt' THEN ",
            $column,
            " < c.n \
               WHEN 'bt' THEN ",
            $column,
            " BETWEEN c.n AND c.m \
               ELSE 0 END)"
        )
    };
}

/// A moment comparison. "Not in the last 30 days" and "before" hold for a track
/// never played, which is what they mean of one.
macro_rules! smart_date_test {
    ($column:literal) => {
        concat!(
            "(CASE c.o \
               WHEN 'ge' THEN ",
            $column,
            " >= c.n \
               WHEN 'lt' THEN COALESCE(",
            $column,
            ", 0) < c.n \
               ELSE 0 END)"
        )
    };
}

/// Whether the bound condition `c` holds for the row `s`. Every branch is
/// chosen by comparing `c.f` with a literal; an unknown field matches nothing,
/// and cannot be reached anyway, because the parse refused it.
macro_rules! smart_predicate {
    () => {
        concat!(
            "COALESCE(CASE c.f \
               WHEN 'title' THEN ",
            smart_text_test!("s.title"),
            " \
               WHEN 'album' THEN ",
            smart_text_test!("s.album_title"),
            " \
               WHEN 'artist' THEN ",
            smart_set_test!(
                "FROM track_participant tp JOIN artist ar ON ar.id=tp.artist_id \
                 WHERE tp.track_id=s.id AND tp.role='artist'",
                "ar.name",
                "ar.canonical_name"
            ),
            " WHEN 'album_artist' THEN ",
            smart_set_test!(
                "FROM album_participant ap JOIN artist ar ON ar.id=ap.artist_id \
                 WHERE ap.album_id=s.album_id AND ap.role='albumartist'",
                "ar.name",
                "ar.canonical_name"
            ),
            " WHEN 'genre' THEN ",
            smart_set_test!(
                "FROM track_genre tg JOIN genre g ON g.id=tg.genre_id \
                 WHERE tg.track_id=s.id",
                "g.name",
                "g.canonical_name"
            ),
            " WHEN 'year' THEN ",
            smart_number_test!("s.year"),
            " \
               WHEN 'rating' THEN ",
            smart_number_test!("COALESCE(s.user_rating, 0)"),
            " \
               WHEN 'play_count' THEN ",
            smart_number_test!("s.play_count"),
            " \
               WHEN 'duration' THEN ",
            smart_number_test!("(s.duration_ms / 1000)"),
            " \
               WHEN 'bitrate' THEN ",
            smart_number_test!("s.bitrate"),
            " \
               WHEN 'last_played' THEN ",
            smart_date_test!("s.last_played_at"),
            " \
               WHEN 'date_added' THEN ",
            smart_date_test!("s.created_at"),
            " \
               WHEN 'starred' THEN ((s.starred_at IS NOT NULL) = (c.n = 1)) \
               WHEN 'format' THEN (CASE c.o \
                   WHEN 'eq' THEN lower(s.relative_path) LIKE '%.' || c.v \
                   ELSE lower(s.relative_path) NOT LIKE '%.' || c.v END) \
               WHEN 'library' THEN (CASE c.o \
                   WHEN 'eq' THEN s.library_id = c.v ELSE s.library_id <> c.v END) \
               ELSE 0 END, 0)"
        )
    };
}

/// One `ORDER BY` term per sort key and direction, numbered as
/// [`SmartSort::ALL`] numbers them. Only the term whose code is bound sorts;
/// the others are `NULL` for every row and change nothing. `NULLS LAST` on
/// each, so a track missing the value sorts after every track that has it,
/// whichever way the list runs.
macro_rules! smart_order_terms {
    () => {
        concat!(
            "CASE WHEN p.sort_key = 99 THEN random() END, \
             CASE WHEN p.sort_key = 1 THEN s.title END COLLATE NOCASE ASC NULLS LAST, \
             CASE WHEN p.sort_key = 2 THEN s.title END COLLATE NOCASE DESC NULLS LAST, \
             CASE WHEN p.sort_key = 3 THEN s.artist_display END COLLATE NOCASE ASC NULLS LAST, \
             CASE WHEN p.sort_key = 4 THEN s.artist_display END COLLATE NOCASE DESC NULLS LAST, \
             CASE WHEN p.sort_key = 5 THEN s.album_title END COLLATE NOCASE ASC NULLS LAST, \
             CASE WHEN p.sort_key = 6 THEN s.album_title END COLLATE NOCASE DESC NULLS LAST, \
             CASE WHEN p.sort_key = 7 THEN s.year END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 8 THEN s.year END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 9 THEN COALESCE(s.user_rating, 0) END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 10 THEN COALESCE(s.user_rating, 0) END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 11 THEN s.play_count END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 12 THEN s.play_count END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 13 THEN s.last_played_at END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 14 THEN s.last_played_at END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 15 THEN s.created_at END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 16 THEN s.created_at END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 17 THEN s.duration_ms END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 18 THEN s.duration_ms END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 19 THEN s.bitrate END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 20 THEN s.bitrate END DESC NULLS LAST"
        )
    };
}

/// The evaluation, whole. Binds, in order:
///
/// 1. the owner's account id (`p.uid`, read again as a second tenancy check);
/// 2. the connective, `all` or `any`;
/// 3. the bound conditions, one JSON document;
/// 4. how many conditions it holds;
/// 5. the sort code;
/// 6. the owner's account id again, for `song_select!`;
/// 7. the limit.
///
/// No other value reaches it, and none of those is ever SQL text.
const EVALUATE: &str = concat!(
    "WITH p AS (SELECT ? AS uid, ? AS mode, ? AS conditions, ? AS cnt, ? AS sort_key), \
     cond AS (SELECT json_extract(value, '$.f') AS f, json_extract(value, '$.o') AS o, \
                     json_extract(value, '$.v') AS v, json_extract(value, '$.k') AS k, \
                     json_extract(value, '$.n') AS n, json_extract(value, '$.m') AS m \
              FROM json_each((SELECT conditions FROM p))) \
     SELECT s.id FROM p, (",
    song_select!(),
    ") AS s \
     WHERE s.library_id IN (SELECT lm.library_id FROM library_member lm WHERE lm.user_id = p.uid) \
       AND (p.cnt = 0 \
        OR (p.mode = 'all' AND NOT EXISTS (SELECT 1 FROM cond c WHERE NOT ",
    smart_predicate!(),
    ")) \
        OR (p.mode = 'any' AND EXISTS (SELECT 1 FROM cond c WHERE ",
    smart_predicate!(),
    "))) \
     ORDER BY ",
    smart_order_terms!(),
    ", s.title COLLATE NOCASE, s.id LIMIT ?"
);

/// A smart playlist, as every surface reads it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SmartPlaylistItem {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub owner_username: String,
    pub name: String,
    pub comment: Option<String>,
    /// Readable by every account on the server, through each one's own
    /// libraries; editable by its owner only.
    pub public: bool,
    pub rules: SmartRules,
    pub created_at: i64,
    pub updated_at: i64,
    /// When the counts below were evaluated for this reader. A smart playlist
    /// has no stored track list. On a read of one playlist it is now; in a
    /// listing it is when this reader last opened it, and `None` when they
    /// have not since the server started — a listing evaluates nothing
    /// (decision 4).
    pub evaluated_at: Option<i64>,
    /// What the reader can play of it, and how long that lasts, as of
    /// `evaluated_at`; `None` with it.
    pub song_count: Option<i64>,
    pub duration_ms: Option<i64>,
    /// The tracks, on a read of one playlist; absent from a listing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub songs: Option<Vec<SongItem>>,
}

/// A rule set evaluated without being stored: what the editor shows.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SmartPlaylistPreview {
    pub song_count: i64,
    pub duration_ms: i64,
    pub songs: Vec<SongItem>,
}

/// Which smart playlists a read reaches, as for ordinary ones (RFC-020
/// decision 2): the reader's own, and other enabled accounts' public ones.
macro_rules! smart_playlist_columns {
    () => {
        "SELECT sp.id, sp.owner_user_id, a.username AS owner_username, sp.name, sp.comment, \
                sp.public, sp.rules_json, sp.created_at, sp.updated_at \
         FROM smart_playlist sp JOIN account a ON a.id=sp.owner_user_id "
    };
}

impl DomainServices {
    /// This account's smart playlists, newest first, and — when asked —
    /// every other enabled account's public ones after them.
    pub async fn smart_playlists(
        &self,
        user_id: Uuid,
        include_public: bool,
    ) -> Result<Vec<SmartPlaylistItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        let rows = sqlx::query(concat!(
            smart_playlist_columns!(),
            "WHERE sp.owner_user_id=? OR (? = 1 AND sp.public=1 AND a.disabled=0) \
             ORDER BY sp.owner_user_id<>?, sp.updated_at DESC, sp.id"
        ))
        .bind(user_id.to_string())
        .bind(i64::from(include_public))
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut playlists = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(mut playlist) = smart_playlist_from_row(&row)? else {
                continue;
            };
            // **No evaluation here** (decision 4): what this reader was last
            // shown of it, if the rules have not changed since.
            if let Some(known) = self
                .smart
                .extents
                .get(&(playlist.id, user_id))
                .filter(|known| known.updated_at == playlist.updated_at)
            {
                playlist.evaluated_at = Some(known.evaluated_at);
                playlist.song_count = Some(known.song_count);
                playlist.duration_ms = Some(known.duration_ms);
            }
            playlists.push(playlist);
        }
        Ok(playlists)
    }

    /// One smart playlist this account may read, evaluated now, its tracks
    /// read through this account's libraries. Another account's private one
    /// answers like one that does not exist.
    pub async fn smart_playlist(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<SmartPlaylistItem, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        let row = sqlx::query(concat!(
            smart_playlist_columns!(),
            "WHERE sp.id=? AND (sp.owner_user_id=? OR (sp.public=1 AND a.disabled=0))"
        ))
        .bind(id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(ServiceError::NotFound)?;
        let mut playlist = smart_playlist_from_row(&row)?.ok_or(ServiceError::NotFound)?;
        let ids = self.evaluate(playlist.owner_id, &playlist.rules).await?;
        let songs = self
            .songs_by_ids_lenient_on(&mut connection, user_id, &ids)
            .await?;
        playlist.evaluated_at = Some(now_ms());
        playlist.song_count = Some(songs.len() as i64);
        playlist.duration_ms = Some(songs.iter().map(|song| song.duration_ms).sum());
        playlist.songs = Some(songs);
        self.remember_extent(&playlist, user_id);
        Ok(playlist)
    }

    /// What a rule set would answer for this account, without storing it.
    pub async fn preview_smart_playlist(
        &self,
        user_id: Uuid,
        rules: &SmartRules,
    ) -> Result<SmartPlaylistPreview, ServiceError> {
        let ids = self.evaluate(user_id, rules).await?;
        let mut connection = self.db.pool().acquire().await?;
        let songs = self
            .songs_by_ids_lenient_on(&mut connection, user_id, &ids)
            .await?;
        Ok(SmartPlaylistPreview {
            song_count: songs.len() as i64,
            duration_ms: songs.iter().map(|song| song.duration_ms).sum(),
            songs,
        })
    }

    pub async fn create_smart_playlist(
        &self,
        user_id: Uuid,
        name: &str,
        comment: Option<&str>,
        public: bool,
        rules: &SmartRules,
    ) -> Result<SmartPlaylistItem, ServiceError> {
        let name = smart_name(name)?;
        let comment = smart_comment(comment)?;
        let id = Uuid::new_v4();
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        // Counted inside `BEGIN IMMEDIATE`, under the gate: two creations
        // cannot both read 99, as for radio stations.
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM smart_playlist WHERE owner_user_id=?")
                .bind(user_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        if count >= MAX_SMART_PLAYLISTS {
            tx.rollback().await?;
            return Err(ServiceError::Conflict);
        }
        sqlx::query(
            "INSERT INTO smart_playlist \
               (id, owner_user_id, name, comment, public, rules_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(&name)
        .bind(comment.as_deref())
        .bind(i64::from(public))
        .bind(rules.to_stored())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        drop(_writer);
        self.smart_playlist(user_id, id).await
    }

    /// Changes what the owner names. `rules`, when given, replaces the whole
    /// rule set: rules are not patched one condition at a time.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_smart_playlist(
        &self,
        user_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        comment: Option<&str>,
        clear_comment: bool,
        public: Option<bool>,
        rules: Option<&SmartRules>,
    ) -> Result<SmartPlaylistItem, ServiceError> {
        let name = name.map(smart_name).transpose()?;
        let comment = smart_comment(comment)?;
        let rules = rules.map(SmartRules::to_stored);
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut connection = self.db.pool().acquire().await?;
        // The owner's own only, in SQL: another account's public playlist
        // answers this exactly as a missing one does.
        let updated = sqlx::query(
            "UPDATE smart_playlist SET \
               name=COALESCE(?, name), \
               comment=CASE WHEN ? = 1 THEN NULL ELSE COALESCE(?, comment) END, \
               public=COALESCE(?, public), \
               rules_json=COALESCE(?, rules_json), \
               updated_at=? \
             WHERE owner_user_id=? AND id=?",
        )
        .bind(name.as_deref())
        .bind(i64::from(clear_comment))
        .bind(comment.as_deref())
        .bind(public.map(i64::from))
        .bind(rules.as_deref())
        .bind(now)
        .bind(user_id.to_string())
        .bind(id.to_string())
        .execute(&mut *connection)
        .await?;
        drop(connection);
        drop(_writer);
        if updated.rows_affected() == 0 {
            return Err(ServiceError::NotFound);
        }
        self.smart_playlist(user_id, id).await
    }

    pub async fn delete_smart_playlist(&self, user_id: Uuid, id: Uuid) -> Result<(), ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut connection = self.db.pool().acquire().await?;
        let deleted = sqlx::query("DELETE FROM smart_playlist WHERE owner_user_id=? AND id=?")
            .bind(user_id.to_string())
            .bind(id.to_string())
            .execute(&mut *connection)
            .await?;
        if deleted.rows_affected() == 0 {
            return Err(ServiceError::NotFound);
        }
        Ok(())
    }

    /// The track ids a rule set answers for `owner`, in its order, at most its
    /// limit.
    ///
    /// **Under a budget** (decision 4). The statement runs on a connection of
    /// its own with a SQLite progress handler installed: every
    /// [`BUDGET_STEP_OPS`] virtual-machine instructions it is asked whether to
    /// go on, and it stops the statement once the budget of instructions, or
    /// [`SMART_EVALUATION_DEADLINE`], is spent. A stopped evaluation answers
    /// `Unavailable` and returns nothing partial.
    ///
    /// **Spawned, so it always finishes what it started.** The handler is a
    /// property of the connection, and the connection goes back to the pool:
    /// were the request dropped between installing the handler and removing
    /// it, the next query on that connection would inherit an exhausted
    /// budget. A spawned task is not cancelled with the request, so the
    /// removal always runs.
    async fn evaluate(&self, owner: Uuid, rules: &SmartRules) -> Result<Vec<Uuid>, ServiceError> {
        self.smart
            .evaluations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let limit = rules.limit.unwrap_or(MAX_SMART_PLAYLIST_TRACKS);
        let mode = match rules.match_kind {
            SmartMatch::All => "all",
            SmartMatch::Any => "any",
        };
        let conditions = rules.bound_conditions(now_ms());
        let count = rules.conditions.len() as i64;
        let sort = rules.sort_code();
        let budget = self
            .smart
            .budget_ops
            .load(std::sync::atomic::Ordering::Relaxed);
        let pool = self.db.pool().clone();
        let exhausted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&exhausted);
        let ids = tokio::spawn(async move {
            let mut connection = pool.acquire().await?;
            let started = std::time::Instant::now();
            let mut spent: u64 = 0;
            connection
                .lock_handle()
                .await?
                .set_progress_handler(BUDGET_STEP_OPS, move || {
                    spent = spent.saturating_add(BUDGET_STEP_OPS as u64);
                    let go_on = spent <= budget && started.elapsed() < SMART_EVALUATION_DEADLINE;
                    if !go_on {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    go_on
                });
            let ids: Result<Vec<String>, sqlx::Error> = sqlx::query_scalar(EVALUATE)
                .bind(owner.to_string())
                .bind(mode)
                .bind(conditions)
                .bind(count)
                .bind(sort)
                .bind(owner.to_string())
                .bind(i64::from(limit))
                .fetch_all(&mut *connection)
                .await;
            connection.lock_handle().await?.remove_progress_handler();
            ids
        })
        .await
        .map_err(|_| ServiceError::Unavailable)?;
        if exhausted.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::warn!("a smart playlist evaluation passed its budget and was stopped");
            return Err(ServiceError::Unavailable);
        }
        ids?.into_iter()
            .map(|id| parse_uuid(id).map_err(ServiceError::from))
            .collect()
    }

    /// How many rule sets this process has evaluated. Observability, and what
    /// the suite counts to prove a listing evaluates none.
    #[doc(hidden)]
    pub fn smart_playlist_evaluations(&self) -> u64 {
        self.smart
            .evaluations
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Lowers the evaluation budget, in virtual-machine instructions. For the
    /// suite, which cannot build a library large enough to reach the real one.
    #[doc(hidden)]
    pub fn limit_smart_playlist_evaluation(&self, ops: u64) {
        self.smart
            .budget_ops
            .store(ops, std::sync::atomic::Ordering::Relaxed);
    }

    /// Records what one viewer was last shown of one playlist, for listings.
    fn remember_extent(&self, playlist: &SmartPlaylistItem, viewer: Uuid) {
        if self.smart.extents.len() >= MAX_REMEMBERED_EXTENTS {
            self.smart.extents.clear();
        }
        self.smart.extents.insert(
            (playlist.id, viewer),
            RememberedExtent {
                updated_at: playlist.updated_at,
                evaluated_at: playlist.evaluated_at.unwrap_or_else(now_ms),
                song_count: playlist.song_count.unwrap_or(0),
                duration_ms: playlist.duration_ms.unwrap_or(0),
            },
        );
    }
}

/// A row, or `None` when its stored rules no longer parse — a hand edit, or a
/// rule a later version removed. Such a playlist is skipped rather than
/// evaluated on rules this version cannot vouch for.
fn smart_playlist_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<Option<SmartPlaylistItem>, ServiceError> {
    let stored: String = row.try_get("rules_json")?;
    let Some(rules) = serde_json::from_str(&stored)
        .ok()
        .and_then(|value| SmartRules::parse(&value).ok())
    else {
        tracing::warn!("a smart playlist carries rules this server cannot read; it is skipped");
        return Ok(None);
    };
    Ok(Some(SmartPlaylistItem {
        id: parse_uuid(row.try_get("id")?)?,
        owner_id: parse_uuid(row.try_get("owner_user_id")?)?,
        owner_username: row.try_get("owner_username")?,
        name: row.try_get("name")?,
        comment: row.try_get("comment")?,
        public: row.try_get::<i64, _>("public")? != 0,
        rules,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        evaluated_at: None,
        song_count: None,
        duration_ms: None,
        songs: None,
    }))
}

fn smart_name(name: &str) -> Result<String, ServiceError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(ServiceError::Invalid);
    }
    Ok(name.to_owned())
}

/// A blank comment is no comment.
fn smart_comment(comment: Option<&str>) -> Result<Option<String>, ServiceError> {
    match comment.map(str::trim) {
        None | Some("") => Ok(None),
        Some(comment) if comment.chars().count() > MAX_SMART_COMMENT_CHARS => {
            Err(ServiceError::Invalid)
        }
        Some(comment) => Ok(Some(comment.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: serde_json::Value) -> Result<SmartRules, RuleError> {
        SmartRules::parse(&value)
    }

    #[test]
    fn a_rule_set_round_trips_through_its_stored_form() {
        let rules = parse(json!({
            "match": "any",
            "conditions": [
                {"field": "genre", "operator": "is", "value": "Rock"},
                {"field": "year", "operator": "between", "value": [1990, 1999]},
                {"field": "last_played", "operator": "in_the_last", "value": 30},
                {"field": "starred", "operator": "is", "value": true},
                {"field": "format", "operator": "is_not", "value": "mp3"},
                {"field": "date_added", "operator": "after", "value": "2024-01-31"},
            ],
            "sort": "play_count",
            "order": "desc",
            "limit": 50,
        }))
        .unwrap();
        let stored: serde_json::Value = serde_json::from_str(&rules.to_stored()).unwrap();
        assert_eq!(parse(stored).unwrap(), rules);
        assert_eq!(rules.sort_code(), 12);
    }

    /// The allowlist, on every axis a client controls. Each refusal names where
    /// it is, and none repeats what was sent.
    #[test]
    fn anything_outside_the_allowlist_is_refused_where_it_is() {
        let refused = |value: serde_json::Value, at: &str| {
            let error = parse(value).unwrap_err();
            assert_eq!(error.at, at, "{error}");
        };
        // The connective Ampache concatenated: not a word this parse knows.
        refused(json!({"match": "all) OR 1=1 --"}), "match");
        refused(json!({"match": "AND"}), "match");
        refused(json!({"match": 1}), "match");
        refused(json!({"extra": true}), "rules");
        refused(json!([]), "rules");
        refused(
            json!({"conditions": [{"field": "title; DROP TABLE track", "operator": "is", "value": "x"}]}),
            "conditions[0].field",
        );
        refused(
            json!({"conditions": [{"field": "title", "operator": "=", "value": "x"}]}),
            "conditions[0].operator",
        );
        refused(
            json!({"conditions": [{"field": "year", "operator": "contains", "value": "19"}]}),
            "conditions[0].operator",
        );
        refused(
            json!({"conditions": [{"field": "year", "operator": "is", "value": "1999"}]}),
            "conditions[0].value",
        );
        refused(
            json!({"conditions": [{"field": "year", "operator": "between", "value": [2000, 1990]}]}),
            "conditions[0].value",
        );
        refused(
            json!({"conditions": [{"field": "rating", "operator": "is", "value": 6}]}),
            "conditions[0].value",
        );
        refused(
            json!({"conditions": [{"field": "format", "operator": "is", "value": "%"}]}),
            "conditions[0].value",
        );
        refused(
            json!({"conditions": [{"field": "library", "operator": "is", "value": "not-a-uuid"}]}),
            "conditions[0].value",
        );
        refused(
            json!({"conditions": [{"field": "date_added", "operator": "before", "value": "yesterday"}]}),
            "conditions[0].value",
        );
        refused(
            json!({"conditions": [{"field": "title", "operator": "is", "value": "x", "sql": "1"}]}),
            "conditions[0]",
        );
        refused(
            json!({"conditions": [{"field": "title", "operator": "is", "value": "  "}]}),
            "conditions[0].value",
        );
        refused(json!({"sort": "t.title DESC"}), "sort");
        refused(json!({"order": "desc; --"}), "order");
        refused(json!({"limit": 0}), "limit");
        refused(json!({"limit": 1001}), "limit");
        refused(json!({"limit": "10"}), "limit");
        let many: Vec<_> = (0..=MAX_SMART_CONDITIONS)
            .map(|_| json!({"field": "title", "operator": "contains", "value": "a"}))
            .collect();
        refused(json!({"conditions": many}), "conditions");
    }

    /// Every field and every operator the parse admits is one the statement
    /// branches on, and in the same spelling. A field added to the parse but
    /// not to the statement would match nothing, silently.
    #[test]
    fn every_admitted_field_and_operator_is_spelled_in_the_statement() {
        for (name, _) in SmartField::ALL {
            assert!(
                EVALUATE.contains(&format!("WHEN '{name}' THEN")),
                "{name} is admitted and never tested"
            );
        }
        for operator in [
            "eq", "ne", "has", "hasnt", "sw", "ew", "gt", "lt", "bt", "ge",
        ] {
            assert!(
                EVALUATE.contains(&format!("'{operator}'")),
                "{operator} is bound and never compared"
            );
        }
    }

    /// The sort codes and the `ORDER BY` terms are one table written twice.
    #[test]
    fn the_sort_codes_match_the_statement() {
        let columns = [
            "s.title",
            "s.artist_display",
            "s.album_title",
            "s.year",
            "COALESCE(s.user_rating, 0)",
            "s.play_count",
            "s.last_played_at",
            "s.created_at",
            "s.duration_ms",
            "s.bitrate",
        ];
        for (index, column) in columns.iter().enumerate() {
            let ascending = index * 2 + 1;
            assert!(
                EVALUATE.contains(&format!("p.sort_key = {ascending} THEN {column} END")),
                "code {ascending} does not sort by {column}"
            );
            assert!(EVALUATE.contains(&format!("p.sort_key = {} THEN {column} END", ascending + 1)));
        }
        let random = parse(json!({"sort": "random", "order": "desc"})).unwrap();
        assert_eq!(random.sort_code(), SORT_RANDOM);
        assert!(EVALUATE.contains("p.sort_key = 99 THEN random()"));
    }

    /// What reaches the statement is built from the validated types: the
    /// connective is one of two literals and the bound conditions carry only
    /// keys this code wrote.
    #[test]
    fn the_bound_form_holds_only_what_this_code_wrote() {
        let rules = parse(json!({
            "match": "all",
            "conditions": [{"field": "artist", "operator": "is", "value": "Björk"}],
        }))
        .unwrap();
        let bound: serde_json::Value =
            serde_json::from_str(&rules.bound_conditions(1_000)).unwrap();
        assert_eq!(
            bound,
            json!([{
                "f": "artist", "o": "eq", "v": "Björk",
                "k": waveflow_core::scanner::canonical_name("Björk"),
                "n": null, "m": null,
            }])
        );
        let within = parse(json!({
            "conditions": [{"field": "last_played", "operator": "in_the_last", "value": 2}],
        }))
        .unwrap();
        let bound: serde_json::Value =
            serde_json::from_str(&within.bound_conditions(10 * DAY_MS)).unwrap();
        assert_eq!(bound[0]["o"], "ge");
        assert_eq!(bound[0]["n"], 8 * DAY_MS);
    }
}
