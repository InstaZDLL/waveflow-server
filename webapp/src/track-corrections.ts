import type {
  SongCredits,
  TrackCorrectionPatch,
  TrackOverrides,
  TrackOverrideValues,
} from "./api";

/**
 * The seven tags a correction can carry beside the file's own value. The
 * catalogue merges these with the file's column, so the editor can show both.
 */
export const SCALAR_FIELDS = [
  "title",
  "sort_title",
  "year",
  "track_number",
  "disc_number",
  "musicbrainz_recording_id",
  "comment",
] as const;
export type ScalarField = (typeof SCALAR_FIELDS)[number];

/**
 * The two lists. A correction to either replaces the rows the scan wrote, so
 * the file's value is not in the database: restoring one re-reads the file, and
 * what that gives back cannot be shown beforehand.
 */
export const LIST_FIELDS = ["artists", "genres"] as const;
export type ListField = (typeof LIST_FIELDS)[number];

export type CorrectionField = ScalarField | ListField;

const NUMERIC_FIELDS: ReadonlySet<ScalarField> = new Set([
  "year",
  "track_number",
  "disc_number",
]);

export function isNumericField(field: ScalarField): boolean {
  return NUMERIC_FIELDS.has(field);
}

/**
 * Free text a tagger may break over lines. A text `<input>` strips line breaks
 * from its value, so such a field needs a `<textarea>`: otherwise the editor
 * shows a flattened value and saves it that way at the first keystroke.
 */
export function isMultilineField(field: ScalarField): boolean {
  return field === "comment";
}

/**
 * One field of the form. `restore` is the explicit gesture of handing the field
 * back to the file, which is distinct from typing: it is the only way to remove
 * a list correction, whose file value the editor cannot display.
 */
export type FieldDraft = { value: string; restore: boolean };
export type CorrectionDraft = Record<CorrectionField, FieldDraft>;

export type CorrectionResult =
  | { ok: true; patch: TrackCorrectionPatch }
  | { ok: false; invalid: ScalarField[] };

type Scalar = string | number | null;

function display(value: Scalar): string {
  return value === null ? "" : String(value);
}

/** What a scalar field shows now: the correction if there is one, the file's otherwise. */
export function effectiveScalar(
  field: ScalarField,
  tracked: TrackOverrides,
): Scalar {
  return tracked.overrides[field] ?? tracked.source[field];
}

/**
 * The credits the track answers with now. The correction when there is one;
 * otherwise what the catalogue holds for the track, which is the file's.
 */
export function currentList(
  field: ListField,
  tracked: TrackOverrides,
  song: SongCredits,
): string[] {
  const stored = tracked.overrides[field];
  if (stored) return stored;
  if (field === "artists") return (song.artists ?? []).map(({ name }) => name);
  return song.genres ?? [];
}

/** A form that starts from the values the track answers with. */
export function draftFrom(
  tracked: TrackOverrides,
  song: SongCredits,
): CorrectionDraft {
  const draft = {} as CorrectionDraft;
  for (const field of SCALAR_FIELDS) {
    draft[field] = {
      value: display(effectiveScalar(field, tracked)),
      restore: false,
    };
  }
  for (const field of LIST_FIELDS) {
    draft[field] = {
      value: currentList(field, tracked, song).join("\n"),
      restore: false,
    };
  }
  return draft;
}

/**
 * One name per line. Never `;`: that separator is the guess a tagger's joined
 * string forces, and a correction exists to settle it — "AC;DC" is one band.
 */
export function parseNames(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((name) => name.trim())
    .filter((name) => name.length > 0);
}

/**
 * A typed value, or `undefined` when the server would refuse it. Blank is
 * `null`: the server reads a blank string as a removal, so a blank is never a
 * value worth sending.
 */
function parseScalar(field: ScalarField, raw: string): Scalar | undefined {
  const value = raw.trim();
  if (value === "") return null;
  if (!isNumericField(field)) return value;
  if (!/^\d+$/.test(value)) return undefined;
  const number = Number(value);
  if (field === "year" && (number < 1 || number > 9999)) return undefined;
  return number;
}

function sameList(left: string[], right: string[]): boolean {
  return (
    left.length === right.length &&
    left.every((name, index) => name === right[index])
  );
}

/**
 * The body that says exactly what the form changed, and nothing else.
 *
 * The server's patch has three states — absent leaves a correction alone,
 * `null` removes it, a value sets it — and every rule below chooses between
 * them. The one that matters most is the first: **a field nobody touched is
 * left out**, so a correction another client made to it survives this save.
 *
 * - A scalar typed back to what the file says removes its correction rather than
 *   storing a copy of the file's value, which would pin the track against the
 *   file being retagged later.
 * - An emptied scalar removes its correction; with none, there is nothing to send.
 * - An emptied list is `[]`: the track credits nobody, which is a correction.
 * - `restore` sends `null` only when there is a correction to remove.
 */
export function buildCorrectionPatch(
  tracked: TrackOverrides,
  song: SongCredits,
  draft: CorrectionDraft,
): CorrectionResult {
  const patch: TrackCorrectionPatch = {};
  const invalid: ScalarField[] = [];
  const assign = <Field extends keyof TrackOverrideValues>(
    field: Field,
    value: TrackOverrideValues[Field],
  ) => {
    patch[field] = value;
  };

  for (const field of SCALAR_FIELDS) {
    const stored = tracked.overrides[field];
    const { value, restore } = draft[field];
    if (restore) {
      if (stored !== null) assign(field, null);
      continue;
    }
    const typed = parseScalar(field, value);
    if (typed === undefined) {
      invalid.push(field);
      continue;
    }
    if (typed === effectiveScalar(field, tracked)) continue;
    if (typed === null || typed === tracked.source[field]) {
      if (stored !== null) assign(field, null);
      continue;
    }
    assign(field, typed as TrackOverrideValues[typeof field]);
  }

  for (const field of LIST_FIELDS) {
    const stored = tracked.overrides[field];
    const { value, restore } = draft[field];
    if (restore) {
      if (stored !== null) assign(field, null);
      continue;
    }
    const names = parseNames(value);
    if (sameList(names, currentList(field, tracked, song))) continue;
    assign(field, names);
  }

  return invalid.length > 0 ? { ok: false, invalid } : { ok: true, patch };
}
