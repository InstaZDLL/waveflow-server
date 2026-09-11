import { Link } from "@tanstack/react-router";
import { type FormEvent, useState } from "react";

import {
  ApiError,
  correctTrack,
  getTrackCredits,
  getTrackOverrides,
  type SongCredits,
  type TrackOverrides,
} from "./api";
import { type TranslationKey, useI18n } from "./i18n";
import { Loading, PageHeader, useAsync } from "./pages";
import {
  buildCorrectionPatch,
  type CorrectionDraft,
  type CorrectionField,
  draftFrom,
  type FieldDraft,
  isNumericField,
  LIST_FIELDS,
  type ListField,
  SCALAR_FIELDS,
  type ScalarField,
} from "./track-corrections";

const LABELS: Record<CorrectionField, TranslationKey> = {
  title: "correction.fieldTitle",
  sort_title: "correction.fieldSortTitle",
  year: "correction.fieldYear",
  track_number: "correction.fieldTrackNumber",
  disc_number: "correction.fieldDiscNumber",
  musicbrainz_recording_id: "correction.fieldMusicbrainz",
  comment: "correction.fieldComment",
  artists: "correction.fieldArtists",
  genres: "correction.fieldGenres",
};

type Loaded = { song: SongCredits; tracked: TrackOverrides };

function load(trackId: string): Promise<Loaded> {
  return Promise.all([
    getTrackCredits(trackId),
    getTrackOverrides(trackId),
  ]).then(([song, tracked]) => ({ song, tracked }));
}

/**
 * Corrects a track's tags without touching its file.
 *
 * The contract decided on 2026-09-07: the catalogue shows the effective value
 * everywhere, and **provenance appears here and nowhere else**. The page is not
 * gated on the account's role: the server is the authority, and a list of
 * libraries that failed to load would otherwise turn an owner away.
 */
export function TrackEditorPage({ trackId }: { trackId: string }) {
  const { value, error } = useAsync(() => load(trackId), [trackId]);
  if (!value) return <Loading error={error} />;
  return <CorrectionForm key={trackId} initial={value} />;
}

function CorrectionForm({ initial }: { initial: Loaded }) {
  const { t } = useI18n();
  const [loaded, setLoaded] = useState(initial);
  const [draft, setDraft] = useState<CorrectionDraft>(() =>
    draftFrom(initial.tracked, initial.song),
  );
  const [saving, setSaving] = useState(false);
  // After a save the stored corrections have moved. The next patch is computed
  // against them, so a form that could not read them back must not save again:
  // it would diff against the corrections from before and remove the wrong one.
  const [stale, setStale] = useState(false);
  const [message, setMessage] = useState<{
    kind: "notice" | "error";
    text: string;
  } | null>(null);
  const { song, tracked } = loaded;

  function update(field: CorrectionField, change: Partial<FieldDraft>) {
    setMessage(null);
    setDraft((current) => ({
      ...current,
      [field]: { ...current[field], ...change },
    }));
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    const result = buildCorrectionPatch(tracked, song, draft);
    if (!result.ok) {
      setMessage({
        kind: "error",
        text: t("correction.invalid", {
          fields: result.invalid.map((field) => t(LABELS[field])).join(", "),
        }),
      });
      return;
    }
    if (Object.keys(result.patch).length === 0) {
      setMessage({ kind: "notice", text: t("correction.nothing") });
      return;
    }
    setSaving(true);
    setMessage(null);
    try {
      await correctTrack(song.id, result.patch);
    } catch (cause) {
      const status = cause instanceof ApiError ? cause.status : 0;
      setMessage({
        kind: "error",
        text:
          status === 422
            ? t("correction.refused")
            : status === 404
              ? t("correction.notAllowed")
              : t("correction.error"),
      });
      setSaving(false);
      return;
    }
    try {
      const fresh = await load(song.id);
      setLoaded(fresh);
      setDraft(draftFrom(fresh.tracked, fresh.song));
      setMessage({ kind: "notice", text: t("correction.saved") });
    } catch {
      setStale(true);
      setMessage({ kind: "error", text: t("correction.stale") });
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="correction-page">
      <PageHeader title={t("correction.edit")} detail={song.title}>
        {song.album_id ? (
          <Link to="/albums/$albumId" params={{ albumId: song.album_id }}>
            {t("correction.back", { album: song.album ?? song.title })}
          </Link>
        ) : null}
      </PageHeader>
      <p className="muted correction-detail">{t("correction.detail")}</p>
      <form
        className="correction-form"
        onSubmit={(event) => void submit(event)}
      >
        {SCALAR_FIELDS.map((field) => (
          <ScalarInput
            key={field}
            field={field}
            tracked={tracked}
            draft={draft[field]}
            onChange={(change) => update(field, change)}
          />
        ))}
        {LIST_FIELDS.map((field) => (
          <ListInput
            key={field}
            field={field}
            tracked={tracked}
            draft={draft[field]}
            onChange={(change) => update(field, change)}
          />
        ))}
        <div className="correction-actions">
          <button type="submit" disabled={saving || stale}>
            {saving ? t("correction.saving") : t("correction.save")}
          </button>
          {message ? (
            <p
              role={message.kind === "error" ? "alert" : "status"}
              className={message.kind}
            >
              {message.text}
            </p>
          ) : null}
        </div>
      </form>
    </section>
  );
}

function CorrectedBadge() {
  const { t } = useI18n();
  return <span className="correction-badge">{t("correction.corrected")}</span>;
}

function RestoreToggle({
  label,
  restore,
  onToggle,
}: {
  label: string;
  restore: boolean;
  onToggle: () => void;
}) {
  const { t } = useI18n();
  const text = restore ? t("correction.keep") : t("correction.restore");
  return (
    <button
      type="button"
      className="link"
      aria-pressed={restore}
      aria-label={`${text}: ${label}`}
      onClick={onToggle}
    >
      {text}
    </button>
  );
}

function ScalarInput({
  field,
  tracked,
  draft,
  onChange,
}: {
  field: ScalarField;
  tracked: TrackOverrides;
  draft: FieldDraft;
  onChange: (change: Partial<FieldDraft>) => void;
}) {
  const { t } = useI18n();
  const label = t(LABELS[field]);
  const corrected = tracked.overrides[field] !== null;
  const fileValue = tracked.source[field];
  const id = `correction-${field}`;
  const provenanceId = `${id}-provenance`;
  return (
    <div className="correction-field">
      <div className="correction-label">
        <label htmlFor={id}>{label}</label>
        {corrected ? <CorrectedBadge /> : null}
      </div>
      <input
        id={id}
        inputMode={isNumericField(field) ? "numeric" : undefined}
        value={draft.restore ? String(fileValue ?? "") : draft.value}
        disabled={draft.restore}
        aria-describedby={corrected ? provenanceId : undefined}
        onChange={(event) => onChange({ value: event.target.value })}
      />
      {corrected ? (
        <p id={provenanceId} className="muted correction-provenance">
          <span>
            {draft.restore
              ? t("correction.restoring")
              : fileValue === null
                ? t("correction.fileEmpty")
                : t("correction.fromFile", { value: String(fileValue) })}
          </span>
          <RestoreToggle
            label={label}
            restore={draft.restore}
            onToggle={() => onChange({ restore: !draft.restore })}
          />
        </p>
      ) : null}
    </div>
  );
}

function ListInput({
  field,
  tracked,
  draft,
  onChange,
}: {
  field: ListField;
  tracked: TrackOverrides;
  draft: FieldDraft;
  onChange: (change: Partial<FieldDraft>) => void;
}) {
  const { t } = useI18n();
  const label = t(LABELS[field]);
  const corrected = tracked.overrides[field] !== null;
  const id = `correction-${field}`;
  const hintId = `${id}-hint`;
  return (
    <div className="correction-field">
      <div className="correction-label">
        <label htmlFor={id}>{label}</label>
        {corrected ? <CorrectedBadge /> : null}
      </div>
      <textarea
        id={id}
        rows={3}
        value={draft.restore ? "" : draft.value}
        disabled={draft.restore}
        aria-describedby={hintId}
        onChange={(event) => onChange({ value: event.target.value })}
      />
      <p id={hintId} className="muted correction-provenance">
        <span>
          {draft.restore
            ? t("correction.restoringList")
            : t("correction.listHint")}
        </span>
        {corrected ? (
          <RestoreToggle
            label={label}
            restore={draft.restore}
            onToggle={() => onChange({ restore: !draft.restore })}
          />
        ) : null}
      </p>
    </div>
  );
}
