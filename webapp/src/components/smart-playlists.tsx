/**
 * Smart playlists beside the ordinary ones (RFC-025): the list, and the editor
 * that writes their rules.
 *
 * Its own module, mounted by the playlists page with one line: the ordinary
 * playlists keep their own code, and a smart playlist shares none of their
 * verbs — it has no track list to append to or export, only rules and what
 * they answer now.
 */

import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";

import {
  ApiError,
  createSmartPlaylist,
  currentUser,
  deleteSmartPlaylist,
  formatDuration,
  getSmartPlaylist,
  previewSmartPlaylist,
  type SmartField,
  type SmartOperator,
  type SmartPlaylist,
  type SmartPreview,
  type SmartSort,
  updateSmartPlaylist,
} from "../api";
import { smartPlaylistsQuery } from "../api/queries";
import {
  blankCondition,
  blankRules,
  type ConditionDraft,
  draftFromRules,
  kindOf,
  MAX_CONDITIONS,
  OPERATORS,
  type RulesDraft,
  rulesFromDraft,
  SMART_FIELDS,
  SMART_SORTS,
  withField,
} from "../lib/smart-rules";
import { usePlayer } from "../player/player";
import { type TranslationKey, useI18n } from "../state/i18n";
import { useReload } from "./page-shell";

type Editing = {
  /** `null` while creating. */
  id: string | null;
  name: string;
  isPublic: boolean;
  rules: RulesDraft;
};

const fieldKey = (field: SmartField) =>
  `smart.field.${field}` as TranslationKey;
const operatorKey = (operator: SmartOperator) =>
  `smart.op.${operator}` as TranslationKey;
const sortKey = (sort: SmartSort) => `smart.sort.${sort}` as TranslationKey;

export function SmartPlaylistsSection() {
  const { t } = useI18n();
  const player = usePlayer();
  const reload = useReload(smartPlaylistsQuery().queryKey);
  const { data: playlists, error } = useQuery(smartPlaylistsQuery());
  const [editing, setEditing] = useState<Editing | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const me = currentUser()?.id;

  async function play(playlist: SmartPlaylist) {
    setMessage(null);
    try {
      // Evaluated now, by the server: the listing carries counts only.
      const full = await getSmartPlaylist(playlist.id);
      if (full.songs?.length) player.play(full.songs, 0);
    } catch {
      setMessage(t("smart.loadError"));
    }
  }

  async function publish(playlist: SmartPlaylist, isPublic: boolean) {
    setMessage(null);
    try {
      await updateSmartPlaylist(playlist.id, { public: isPublic });
      reload();
    } catch {
      setMessage(t("smart.saveError"));
    }
  }

  async function remove(playlist: SmartPlaylist) {
    if (busy) return;
    setBusy(true);
    setMessage(null);
    try {
      await deleteSmartPlaylist(playlist.id);
      if (editing?.id === playlist.id) setEditing(null);
      reload();
    } catch {
      setMessage(t("smart.deleteError"));
    } finally {
      setBusy(false);
    }
  }

  const own = (playlists ?? []).filter((playlist) => playlist.owner_id === me);
  const others = (playlists ?? []).filter(
    (playlist) => playlist.owner_id !== me,
  );

  const card = (playlist: SmartPlaylist, mine: boolean) => (
    <article className="collection smart-playlist" key={playlist.id}>
      <header className="collection-header">
        <div>
          <h3>
            {playlist.name}{" "}
            <span className="smart-badge">{t("smart.badge")}</span>
          </h3>
          <span className="muted">
            {t("common.tracks", { count: playlist.song_count })}
            {playlist.duration_ms
              ? ` · ${formatDuration(playlist.duration_ms)}`
              : ""}
            {mine
              ? null
              : ` · ${t("playlists.by", { owner: playlist.owner_username })}`}
          </span>
        </div>
        <div className="actions">
          <button
            type="button"
            onClick={() => void play(playlist)}
            disabled={!playlist.song_count}
          >
            {t("playlists.play")}
          </button>
          {mine ? (
            <>
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  setMessage(null);
                  setEditing({
                    id: playlist.id,
                    name: playlist.name,
                    isPublic: playlist.public,
                    rules: draftFromRules(playlist.rules),
                  });
                }}
              >
                {t("smart.edit")}
              </button>
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={playlist.public}
                  onChange={(event) =>
                    void publish(playlist, event.target.checked)
                  }
                />
                {t("playlists.public")}
              </label>
              <button
                type="button"
                className="danger"
                disabled={busy}
                onClick={() => void remove(playlist)}
              >
                {t("common.delete")}
              </button>
            </>
          ) : null}
        </div>
      </header>
    </article>
  );

  return (
    <section className="stack smart-playlists" aria-labelledby="smart-title">
      <header className="collection-header">
        <div>
          <h3 id="smart-title">{t("smart.title")}</h3>
          <p className="muted">{t("smart.detail")}</p>
        </div>
        <button
          type="button"
          disabled={editing !== null}
          onClick={() => {
            setMessage(null);
            setEditing({
              id: null,
              name: "",
              isPublic: false,
              rules: blankRules(),
            });
          }}
        >
          {t("smart.new")}
        </button>
      </header>
      {editing ? (
        <SmartPlaylistEditor
          editing={editing}
          onChange={setEditing}
          onDone={() => {
            setEditing(null);
            reload();
          }}
        />
      ) : null}
      {message ? <p className="error">{message}</p> : null}
      {error ? <p className="error">{t("smart.loadError")}</p> : null}
      {own.length ? (
        <div className="stack">
          {own.map((playlist) => card(playlist, true))}
        </div>
      ) : playlists ? (
        <p className="muted">{t("smart.empty")}</p>
      ) : null}
      {others.length ? (
        <div className="stack">
          {others.map((playlist) => card(playlist, false))}
        </div>
      ) : null}
    </section>
  );
}

function SmartPlaylistEditor({
  editing,
  onChange,
  onDone,
}: {
  editing: Editing;
  onChange: (editing: Editing | null) => void;
  onDone: () => void;
}) {
  const { t } = useI18n();
  const [preview, setPreview] = useState<SmartPreview | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const rules = rulesFromDraft(editing.rules);

  const setRules = (next: Partial<RulesDraft>) =>
    onChange({ ...editing, rules: { ...editing.rules, ...next } });
  const setCondition = (index: number, next: ConditionDraft) =>
    setRules({
      conditions: editing.rules.conditions.map((condition, at) =>
        at === index ? next : condition,
      ),
    });

  async function showPreview() {
    if (!rules || busy) return;
    setBusy(true);
    setFailure(null);
    try {
      setPreview(await previewSmartPlaylist(rules));
    } catch {
      setFailure(t("smart.refused"));
    } finally {
      setBusy(false);
    }
  }

  async function save(event: FormEvent) {
    event.preventDefault();
    if (!rules || busy || !editing.name.trim()) return;
    setBusy(true);
    setFailure(null);
    try {
      if (editing.id) {
        await updateSmartPlaylist(editing.id, {
          name: editing.name,
          public: editing.isPublic,
          rules,
        });
      } else {
        await createSmartPlaylist({
          name: editing.name,
          public: editing.isPublic,
          rules,
        });
      }
      onDone();
    } catch (error) {
      setFailure(
        error instanceof ApiError && error.status === 409
          ? t("smart.full")
          : error instanceof ApiError && error.status === 422
            ? t("smart.refused")
            : t("smart.saveError"),
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      className="collection stacked-form smart-editor"
      onSubmit={(event) => void save(event)}
      aria-label={editing.id ? t("smart.edit") : t("smart.new")}
    >
      <label>
        {t("smart.name")}
        <input
          value={editing.name}
          onChange={(event) =>
            onChange({ ...editing, name: event.target.value })
          }
          required
          disabled={busy}
        />
      </label>
      <label>
        {t("smart.match")}
        <select
          value={editing.rules.match}
          onChange={(event) =>
            setRules({ match: event.target.value as "all" | "any" })
          }
          disabled={busy}
        >
          <option value="all">{t("smart.match.all")}</option>
          <option value="any">{t("smart.match.any")}</option>
        </select>
      </label>
      <ol className="smart-conditions">
        {editing.rules.conditions.map((condition, index) => {
          const kind = kindOf(condition.field);
          const position = index + 1;
          return (
            // The position is the only identity a condition has.
            // biome-ignore lint/suspicious/noArrayIndexKey: conditions carry no id
            <li className="smart-condition" key={index}>
              <select
                aria-label={t("smart.fieldOf", { position })}
                value={condition.field}
                onChange={(event) =>
                  setCondition(
                    index,
                    withField(condition, event.target.value as SmartField),
                  )
                }
                disabled={busy}
              >
                {SMART_FIELDS.map(({ field }) => (
                  <option key={field} value={field}>
                    {t(fieldKey(field))}
                  </option>
                ))}
              </select>
              <select
                aria-label={t("smart.operatorOf", { position })}
                value={condition.operator}
                onChange={(event) =>
                  setCondition(index, {
                    ...condition,
                    operator: event.target.value as SmartOperator,
                  })
                }
                disabled={busy}
              >
                {OPERATORS[kind].map((operator) => (
                  <option key={operator} value={operator}>
                    {t(operatorKey(operator))}
                  </option>
                ))}
              </select>
              {kind === "flag" ? (
                <select
                  aria-label={t("smart.valueOf", { position })}
                  value={condition.value}
                  onChange={(event) =>
                    setCondition(index, {
                      ...condition,
                      value: event.target.value,
                    })
                  }
                  disabled={busy}
                >
                  <option value="true">{t("smart.yes")}</option>
                  <option value="false">{t("smart.no")}</option>
                </select>
              ) : (
                <input
                  aria-label={t("smart.valueOf", { position })}
                  type={
                    kind === "date" &&
                    (condition.operator === "before" ||
                      condition.operator === "after")
                      ? "date"
                      : kind === "number" || kind === "date"
                        ? "number"
                        : "text"
                  }
                  value={condition.value}
                  placeholder={
                    kind === "date" &&
                    (condition.operator === "in_the_last" ||
                      condition.operator === "not_in_the_last")
                      ? t("smart.days")
                      : undefined
                  }
                  onChange={(event) =>
                    setCondition(index, {
                      ...condition,
                      value: event.target.value,
                    })
                  }
                  disabled={busy}
                />
              )}
              {condition.operator === "between" ? (
                <input
                  aria-label={t("smart.upperOf", { position })}
                  type="number"
                  value={condition.upper}
                  onChange={(event) =>
                    setCondition(index, {
                      ...condition,
                      upper: event.target.value,
                    })
                  }
                  disabled={busy}
                />
              ) : null}
              <button
                type="button"
                className="danger"
                aria-label={t("smart.removeCondition", { position })}
                onClick={() =>
                  setRules({
                    conditions: editing.rules.conditions.filter(
                      (_, at) => at !== index,
                    ),
                  })
                }
                disabled={busy}
              >
                ×
              </button>
            </li>
          );
        })}
      </ol>
      <div className="actions">
        <button
          type="button"
          onClick={() =>
            setRules({
              conditions: [...editing.rules.conditions, blankCondition()],
            })
          }
          disabled={busy || editing.rules.conditions.length >= MAX_CONDITIONS}
        >
          {t("smart.addCondition")}
        </button>
      </div>
      <div className="actions">
        <label>
          {t("smart.sort")}
          <select
            value={editing.rules.sort}
            onChange={(event) =>
              setRules({ sort: event.target.value as SmartSort | "" })
            }
            disabled={busy}
          >
            <option value="">{t("smart.sort.none")}</option>
            {SMART_SORTS.map((sort) => (
              <option key={sort} value={sort}>
                {t(sortKey(sort))}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t("smart.order")}
          <select
            value={editing.rules.order}
            onChange={(event) =>
              setRules({ order: event.target.value as "asc" | "desc" })
            }
            disabled={
              busy || !editing.rules.sort || editing.rules.sort === "random"
            }
          >
            <option value="asc">{t("smart.order.asc")}</option>
            <option value="desc">{t("smart.order.desc")}</option>
          </select>
        </label>
        <label>
          {t("smart.limit")}
          <input
            type="number"
            min={1}
            max={1000}
            value={editing.rules.limit}
            onChange={(event) => setRules({ limit: event.target.value })}
            disabled={busy}
          />
        </label>
        <label className="checkbox-label">
          <input
            type="checkbox"
            checked={editing.isPublic}
            onChange={(event) =>
              onChange({ ...editing, isPublic: event.target.checked })
            }
            disabled={busy}
          />
          {t("playlists.public")}
        </label>
      </div>
      {rules ? null : <p className="muted">{t("smart.unfinished")}</p>}
      {failure ? <p className="error">{failure}</p> : null}
      {preview ? (
        <div className="smart-preview" role="status">
          <strong>
            {t("smart.previewCount", { count: preview.song_count })}
          </strong>
          {preview.songs.length ? (
            <ol>
              {preview.songs.slice(0, 10).map((song) => (
                <li key={song.id}>
                  {song.title}
                  {song.artist ? (
                    <span className="muted"> · {song.artist}</span>
                  ) : null}
                </li>
              ))}
            </ol>
          ) : null}
        </div>
      ) : null}
      <div className="actions">
        <button
          type="button"
          onClick={() => void showPreview()}
          disabled={!rules || busy}
        >
          {t("smart.preview")}
        </button>
        <button type="submit" disabled={!rules || busy || !editing.name.trim()}>
          {t("smart.save")}
        </button>
        <button type="button" onClick={() => onChange(null)} disabled={busy}>
          {t("common.cancel")}
        </button>
      </div>
    </form>
  );
}
