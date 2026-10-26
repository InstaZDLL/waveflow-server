//! The rule vocabulary and its allowlist parse: what a smart playlist may say,
//! how it is stored, and the one bound value the statement reads it from.

use super::*;

/// The longest text a condition may compare against, in characters.
const MAX_RULE_TEXT_CHARS: usize = 200;

/// One day, in milliseconds: what `in_the_last` counts in.
pub(super) const DAY_MS: i64 = 24 * 60 * 60 * 1000;

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
    pub(super) const ALL: [(&'static str, SmartField); 15] = [
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
    /// are what [`EVALUATE`](crate::services::smart_playlists::statement::EVALUATE) compares against, so this order and the `ORDER BY`
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

/// The code [`EVALUATE`](crate::services::smart_playlists::statement::EVALUATE) reads for "no sort": its fixed tie-break alone.
const SORT_NONE: i64 = 0;
/// The code for a random order, whatever direction was asked.
pub(super) const SORT_RANDOM: i64 = 99;

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
    pub(super) fn to_stored(&self) -> String {
        serde_json::to_string(self).expect("rule serialization cannot fail")
    }

    /// The code [`EVALUATE`](crate::services::smart_playlists::statement::EVALUATE) orders by.
    pub(super) fn sort_code(&self) -> i64 {
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
    pub(super) fn bound_conditions(&self, now: i64) -> String {
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
            // Kept in its canonical spelling — hyphenated, lower case — which
            // is how `library.id` is stored. An id sent in capitals or braces
            // parses, and compared as sent it would match no library under
            // `is` and every library under `is_not`.
            let Some(id) = value.as_str().and_then(|id| Uuid::parse_str(id).ok()) else {
                return invalid("must be a library id");
            };
            return Ok(SmartCondition {
                field,
                operator,
                value: serde_json::Value::String(id.to_string()),
            });
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

/// One condition as [`EVALUATE`](crate::services::smart_playlists::statement::EVALUATE) binds it.
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
