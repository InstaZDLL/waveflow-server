/**
 * The smart playlist vocabulary, as the editor offers it (RFC-025).
 *
 * The server holds the allowlist and refuses anything outside it; this file is
 * the same table written for a form, so the editor never builds a rule set the
 * server would refuse. Pure: no React, no requests.
 */

import type {
  SmartCondition,
  SmartConditionValue,
  SmartField,
  SmartOperator,
  SmartRules,
  SmartSort,
} from "../api/smart-playlists";

/** The kind of value a field holds, which decides its operators. */
export type FieldKind =
  | "text"
  | "number"
  | "date"
  | "flag"
  | "extension"
  | "identifier";

/** The field, its kind, and — for a number — the range the server accepts. */
export const SMART_FIELDS: ReadonlyArray<{
  field: SmartField;
  kind: FieldKind;
  min?: number;
  max?: number;
}> = [
  { field: "title", kind: "text" },
  { field: "artist", kind: "text" },
  { field: "album_artist", kind: "text" },
  { field: "album", kind: "text" },
  { field: "genre", kind: "text" },
  { field: "year", kind: "number", min: 0, max: 9999 },
  { field: "rating", kind: "number", min: 0, max: 5 },
  { field: "starred", kind: "flag" },
  { field: "play_count", kind: "number", min: 0, max: 1_000_000_000 },
  { field: "last_played", kind: "date" },
  { field: "date_added", kind: "date" },
  { field: "duration", kind: "number", min: 0, max: 1_000_000 },
  { field: "bitrate", kind: "number", min: 0, max: 100_000 },
  { field: "format", kind: "extension" },
  { field: "library", kind: "identifier" },
];

export const OPERATORS: Record<FieldKind, SmartOperator[]> = {
  text: [
    "contains",
    "is",
    "is_not",
    "not_contains",
    "starts_with",
    "ends_with",
  ],
  number: ["is", "is_not", "greater_than", "less_than", "between"],
  date: ["in_the_last", "not_in_the_last", "before", "after"],
  flag: ["is"],
  extension: ["is", "is_not"],
  identifier: ["is", "is_not"],
};

export const SMART_SORTS: SmartSort[] = [
  "title",
  "artist",
  "album",
  "year",
  "rating",
  "play_count",
  "last_played",
  "date_added",
  "duration",
  "bitrate",
  "random",
];

export const MAX_CONDITIONS = 20;
export const MAX_LIMIT = 1000;

export function kindOf(field: SmartField): FieldKind {
  return SMART_FIELDS.find((entry) => entry.field === field)?.kind ?? "text";
}

/**
 * A condition as the form holds it: every value as text, the second bound of
 * `between` beside the first, so switching an operator never loses what was
 * typed.
 */
export type ConditionDraft = {
  field: SmartField;
  operator: SmartOperator;
  value: string;
  upper: string;
};

export function blankCondition(field: SmartField = "title"): ConditionDraft {
  const kind = kindOf(field);
  return {
    field,
    operator: OPERATORS[kind][0],
    value: kind === "flag" ? "true" : "",
    upper: "",
  };
}

/** A field change keeps the operator when the new field admits it. */
export function withField(
  draft: ConditionDraft,
  field: SmartField,
): ConditionDraft {
  const kind = kindOf(field);
  const operator = OPERATORS[kind].includes(draft.operator)
    ? draft.operator
    : OPERATORS[kind][0];
  return {
    ...draft,
    field,
    operator,
    value:
      kind === "flag"
        ? "true"
        : kind === kindOf(draft.field)
          ? draft.value
          : "",
  };
}

function wholeNumber(text: string, min: number, max: number): number | null {
  if (!/^\d+$/.test(text.trim())) return null;
  const value = Number(text.trim());
  return value >= min && value <= max ? value : null;
}

/**
 * The value the server is sent, or `null` when the form does not yet hold one
 * it would accept.
 */
export function conditionValue(
  draft: ConditionDraft,
): SmartConditionValue | null {
  const kind = kindOf(draft.field);
  const entry = SMART_FIELDS.find(
    (candidate) => candidate.field === draft.field,
  );
  switch (kind) {
    case "text": {
      const text = draft.value.trim();
      return text && text.length <= 200 ? text : null;
    }
    case "number": {
      const min = entry?.min ?? 0;
      const max = entry?.max ?? Number.MAX_SAFE_INTEGER;
      const low = wholeNumber(draft.value, min, max);
      if (low === null) return null;
      if (draft.operator !== "between") return low;
      const high = wholeNumber(draft.upper, min, max);
      return high !== null && low <= high ? [low, high] : null;
    }
    case "date":
      if (
        draft.operator === "in_the_last" ||
        draft.operator === "not_in_the_last"
      ) {
        return wholeNumber(draft.value, 1, 36_500);
      }
      return /^\d{4}-\d{2}-\d{2}$/.test(draft.value) ? draft.value : null;
    case "flag":
      return draft.value === "true";
    case "extension": {
      const extension = draft.value.trim().toLowerCase().replace(/^\./, "");
      return /^[a-z0-9]{1,10}$/.test(extension) ? extension : null;
    }
    case "identifier": {
      // The library's id, in the spelling the server stores it in.
      const id = draft.value.trim().toLowerCase();
      return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(
        id,
      )
        ? id
        : null;
    }
  }
}

export type RulesDraft = {
  match: "all" | "any";
  conditions: ConditionDraft[];
  sort: SmartSort | "";
  order: "asc" | "desc";
  limit: string;
};

export function blankRules(): RulesDraft {
  return {
    match: "all",
    conditions: [blankCondition()],
    sort: "",
    order: "asc",
    limit: "",
  };
}

/** The rule set the form describes, or `null` while any part of it is unfinished. */
export function rulesFromDraft(draft: RulesDraft): SmartRules | null {
  const conditions: SmartCondition[] = [];
  for (const condition of draft.conditions) {
    const value = conditionValue(condition);
    if (value === null) return null;
    conditions.push({
      field: condition.field,
      operator: condition.operator,
      value,
    });
  }
  if (conditions.length > MAX_CONDITIONS) return null;
  let limit: number | null = null;
  if (draft.limit.trim()) {
    limit = wholeNumber(draft.limit, 1, MAX_LIMIT);
    if (limit === null) return null;
  }
  return {
    match: draft.match,
    conditions,
    sort: draft.sort || null,
    order: draft.order,
    limit,
  };
}

/** The form a stored rule set is edited in. */
export function draftFromRules(rules: SmartRules): RulesDraft {
  return {
    match: rules.match,
    conditions: rules.conditions.map((condition) => {
      const value = condition.value;
      if (Array.isArray(value)) {
        return {
          field: condition.field,
          operator: condition.operator,
          value: String(value[0]),
          upper: String(value[1]),
        };
      }
      return {
        field: condition.field,
        operator: condition.operator,
        value: String(value),
        upper: "",
      };
    }),
    sort: rules.sort ?? "",
    order: rules.order,
    limit: rules.limit === null ? "" : String(rules.limit),
  };
}
