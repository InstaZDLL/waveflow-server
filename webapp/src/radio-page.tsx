import { useQuery } from "@tanstack/react-query";
import {
  type FormEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import {
  ApiError,
  createRadioStation,
  deleteRadioStation,
  type RadioStation,
  replaceRadioStation,
} from "./api";
import { useI18n } from "./i18n";
import { Icon } from "./icons";
import { EmptyState, Loading, PageHeader, useReload } from "./page-shell";
import { usePlayer } from "./player";
import { radioStationsQuery } from "./queries";

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

/**
 * Whether a failed play attempt is still worth telling the reader about.
 *
 * Two ways it is not, both reachable by ordinary use of the list. **A stale
 * attempt**: starting a second station replaces the `src` of the one element
 * every station plays from, which rejects the first station's pending
 * `play()` — and that rejection lands after the second station is already on
 * screen, so reporting it would blame a station that is playing perfectly
 * well. **An interruption**: pressing stop while a station is still loading
 * rejects the same promise with `AbortError`, which is the page doing what it
 * was asked rather than a station that cannot be reached.
 */
export function reportsPlaybackFailure(
  attempt: number,
  current: number,
  failure?: unknown,
): boolean {
  if (attempt !== current) return false;
  return !(failure instanceof DOMException && failure.name === "AbortError");
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
  const [playing, setPlaying] = useState<RadioStation | null>(null);
  const [playbackError, setPlaybackError] = useState(false);
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
  const audio = useRef<HTMLAudioElement | null>(null);
  /**
   * Which play attempt the page is showing.
   *
   * One element serves every station, so starting a second one — or stopping
   * the first — rejects the pending `play()` of the first. That rejection
   * arrives *after* the new state is set, and without this counter it would
   * report a failure about a station nobody is waiting for any more.
   */
  const attempt = useRef(0);
  /**
   * The same station as `playing`, readable after an `await`.
   *
   * `playing` is a render value, so a callback that resumes after a request
   * sees whatever was playing when the request *left*. Deleting a station and
   * then starting one during the round trip is enough to tell the two apart:
   * the state says nothing is playing, and a stream is. Playback is not a
   * mutation and stays available while a write is in flight, which is why
   * this one needs a ref where the form below needs only `busy`.
   */
  const playingRef = useRef<RadioStation | null>(null);

  const silence = useCallback(() => {
    // Invalidated before the pause, because the pause is what rejects the
    // attempt's promise.
    attempt.current += 1;
    audio.current?.pause();
    playingRef.current = null;
    setPlaying(null);
  }, []);

  // A station is not a track: it has no duration to seek in, no id the queue
  // could persist and nothing to scrobble, so it plays from its own element
  // rather than through the queue player. The two must not sound at once,
  // which is what these two effects keep.
  useEffect(() => {
    if (playing && player.playing) silence();
  }, [player.playing, playing, silence]);

  useEffect(
    () => () => {
      // Stopped on the way out, deliberately. This page owns the only stop
      // button there is, so audio that survived it could not be stopped at
      // all. Carrying a station into the player bar instead would mean a
      // transport that has no position, no next and nothing to submit.
      audio.current?.pause();
    },
    [],
  );

  if (!stations) return <Loading error={error} />;

  function startPlaying(station: RadioStation) {
    setPlaybackError(false);
    if (player.playing) player.toggle();
    const element = audio.current ?? new Audio();
    audio.current = element;
    attempt.current += 1;
    const mine = attempt.current;
    const failed = (failure?: unknown) => {
      if (!reportsPlaybackFailure(mine, attempt.current, failure)) return;
      setPlaybackError(true);
      playingRef.current = null;
      setPlaying(null);
    };
    element.onerror = () => failed();
    element.src = station.stream_url;
    playingRef.current = station;
    setPlaying(station);
    void element.play().catch(failed);
  }

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
      // Read from the ref, not from `playing`: the station playing now is not
      // necessarily the one that was playing when this request left, and
      // stopping on the stale value would either leave a deleted station
      // sounding with no button to stop it, or stop a station the reader had
      // just started.
      if (playingRef.current?.id === station.id) silence();
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
      <PageHeader title={t("nav.radio")} detail={t("radio.detail")} />
      <form className="stacked-form" onSubmit={(event) => void submit(event)}>
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
      {playbackError ? (
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
                  playing?.id === station.id ? silence() : startPlaying(station)
                }
                aria-label={
                  playing?.id === station.id
                    ? t("radio.stop")
                    : t("radio.listen")
                }
              >
                <Icon name={playing?.id === station.id ? "pause" : "play"} />
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
