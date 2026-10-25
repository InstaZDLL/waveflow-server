import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";

import {
  ApiError,
  createRadioStation,
  deleteRadioStation,
  type RadioStation,
  replaceRadioStation,
} from "../api";
import { radioStationsQuery } from "../api/queries";
import { Icon } from "../components/icons";
import {
  EmptyState,
  Loading,
  PageHeader,
  useReload,
} from "../components/page-shell";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";

/**
 * The same rule the server applies, applied early enough to say so.
 *
 * Absolute, `http` or `https`. The point is not to protect the server — it
 * validates for itself, and it has to, since the facade accepts the same
 * writes — but to tell somebody typing why the button is refusing them instead
 * of making them send a request to find out.
 */
export function isPlayableAddress(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  try {
    const url = new URL(trimmed);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

/**
 * Whether the browser will refuse this station outright.
 *
 * A clear-text stream on a page served over TLS is mixed content, and every
 * current browser blocks it in the audio element without asking. The server
 * stores such an address quite happily — it never fetches it, so nothing there
 * can tell — and the failure then arrives as a stream that simply does not
 * play. Saying it in advance is the only place this can be said usefully.
 */
export function isBlockedAsMixedContent(
  streamUrl: string,
  pageProtocol: string,
): boolean {
  return pageProtocol === "https:" && streamUrl.trim().startsWith("http:");
}

type Draft = { name: string; stream_url: string; homepage_url: string };

const emptyDraft: Draft = { name: "", stream_url: "", homepage_url: "" };

export function RadioPage() {
  const { t } = useI18n();
  const player = usePlayer();
  const { data: stations, error } = useQuery(radioStationsQuery());
  const reload = useReload(radioStationsQuery().queryKey);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  /** The station the form is replacing, or `null` when it is adding one. */
  const [editing, setEditing] = useState<RadioStation | null>(null);
  const [mutationError, setMutationError] = useState<string | null>(null);
  // The station is the player's, not this page's: it plays in the bar, and
  // carries on when the reader goes elsewhere. What this page shows of it is
  // read from there.
  const live = player.aside?.kind === "live" ? player.aside.station : null;
  const playingId = live && player.asidePlaying ? live.id : null;
  /**
   * True while a write is in flight, which closes the form and the two
   * per-station writes.
   *
   * **Not polish.** A second click on `Add station` before the first answered
   * created a second station — the server has no reason to refuse one — and
   * starting to edit another station mid-save made the save clear a form the
   * reader had just filled, because `editing` and `draft` are read again
   * after the `await`. Closing the form removes both, where comparing the
   * values afterwards would have removed only the second (constats de revue,
   * pris). Listening stays available throughout: it writes nothing, and a
   * reader waiting on a save has no reason to lose the station they are
   * hearing.
   */
  const [busy, setBusy] = useState(false);

  if (!stations) return <Loading error={error} />;

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setMutationError(null);
    // Sent whole, every time: the server replaces the station rather than
    // patching it, so an empty homepage field clears the stored one.
    const payload = {
      name: draft.name,
      stream_url: draft.stream_url,
      homepage_url: draft.homepage_url.trim() ? draft.homepage_url : null,
    };
    try {
      if (editing) {
        await replaceRadioStation(editing.id, payload);
      } else {
        await createRadioStation(payload);
      }
      // Safe to clear because the form was closed for the whole round trip:
      // nothing the reader did in the meantime is being thrown away.
      setDraft(emptyDraft);
      setEditing(null);
      reload();
    } catch (failure) {
      const full = failure instanceof ApiError && failure.status === 409;
      setMutationError(full ? t("radio.full") : t("radio.saveError"));
    } finally {
      setBusy(false);
    }
  }

  async function remove(station: RadioStation) {
    if (busy) return;
    setBusy(true);
    setMutationError(null);
    try {
      await deleteRadioStation(station.id);
      // Asked of the player by id, and not decided here from `live`: the
      // station playing now is not necessarily the one that was playing when
      // this request left, and stopping on that stale value would either leave
      // a deleted station sounding or stop one the reader had just started.
      player.stopAside(station.id);
      if (editing?.id === station.id) {
        setEditing(null);
        setDraft(emptyDraft);
      }
      reload();
    } catch {
      setMutationError(t("radio.deleteError"));
    } finally {
      setBusy(false);
    }
  }

  const addressRefused = draft.stream_url.trim()
    ? !isPlayableAddress(draft.stream_url)
    : false;
  const homepageRefused = draft.homepage_url.trim()
    ? !isPlayableAddress(draft.homepage_url)
    : false;
  const submittable =
    draft.name.trim().length > 0 && !addressRefused && !homepageRefused;

  return (
    <section>
      <PageHeader
        kicker={t("nav.group.listen")}
        title={t("nav.radio")}
        detail={t("radio.detail")}
      />
      <form
        className="stacked-form radio-form"
        onSubmit={(event) => void submit(event)}
      >
        <label>
          {t("radio.name")}
          <input
            value={draft.name}
            onChange={(event) =>
              setDraft({ ...draft, name: event.target.value })
            }
            required
            disabled={busy}
          />
        </label>
        <label>
          {t("radio.address")}
          <input
            type="url"
            value={draft.stream_url}
            onChange={(event) =>
              setDraft({ ...draft, stream_url: event.target.value })
            }
            placeholder="https://radio.example.org/stream"
            required
            disabled={busy}
          />
        </label>
        <label>
          {t("radio.homepage")}
          <input
            type="url"
            value={draft.homepage_url}
            onChange={(event) =>
              setDraft({ ...draft, homepage_url: event.target.value })
            }
            disabled={busy}
          />
        </label>
        {addressRefused || homepageRefused ? (
          <p className="error">{t("radio.addressRefused")}</p>
        ) : null}
        {isBlockedAsMixedContent(draft.stream_url, window.location.protocol) ? (
          <p className="muted">{t("radio.mixedContent")}</p>
        ) : null}
        <div className="actions">
          <button type="submit" disabled={!submittable || busy}>
            {editing ? t("radio.save") : t("radio.add")}
          </button>
          {editing ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setEditing(null);
                setDraft(emptyDraft);
              }}
            >
              {t("common.cancel")}
            </button>
          ) : null}
        </div>
        {editing ? <p className="muted">{t("radio.replacing")}</p> : null}
      </form>
      {mutationError ? <p className="error">{mutationError}</p> : null}
      {live && player.asideError ? (
        <p className="error">{t("radio.playbackError")}</p>
      ) : null}
      {stations.length ? (
        <ul className="resource-list">
          {stations.map((station) => (
            <li key={station.id}>
              <div>
                <strong>{station.name}</strong>
                <span className="muted resource-url">{station.stream_url}</span>
                {isBlockedAsMixedContent(
                  station.stream_url,
                  window.location.protocol,
                ) ? (
                  <span className="muted">{t("radio.mixedContent")}</span>
                ) : null}
              </div>
              {station.homepage_url ? (
                <a
                  className="muted"
                  href={station.homepage_url}
                  target="_blank"
                  rel="noreferrer noopener"
                >
                  {t("radio.visit")}
                </a>
              ) : null}
              <button
                type="button"
                onClick={() =>
                  playingId === station.id
                    ? player.toggleAside()
                    : player.playLive(station)
                }
                aria-label={
                  playingId === station.id ? t("radio.stop") : t("radio.listen")
                }
              >
                <Icon name={playingId === station.id ? "stop" : "play"} />
              </button>
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  setEditing(station);
                  setDraft({
                    name: station.name,
                    stream_url: station.stream_url,
                    homepage_url: station.homepage_url ?? "",
                  });
                }}
              >
                {t("radio.edit")}
              </button>
              <button
                type="button"
                className="danger"
                disabled={busy}
                onClick={() => void remove(station)}
              >
                {t("common.delete")}
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <EmptyState message={t("radio.empty")} />
      )}
    </section>
  );
}
