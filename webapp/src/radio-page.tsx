import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useEffect, useRef, useState } from "react";

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
  const audio = useRef<HTMLAudioElement | null>(null);

  // A station is not a track: it has no duration to seek in, no id the queue
  // could persist and nothing to scrobble, so it plays from its own element
  // rather than through the queue player. The two must not sound at once,
  // which is what these two effects keep.
  useEffect(() => {
    if (playing && player.playing) {
      audio.current?.pause();
      setPlaying(null);
    }
  }, [player.playing, playing]);

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
    element.onerror = () => {
      setPlaybackError(true);
      setPlaying(null);
    };
    element.src = station.stream_url;
    setPlaying(station);
    void element.play().catch(() => {
      setPlaybackError(true);
      setPlaying(null);
    });
  }

  function stopPlaying() {
    audio.current?.pause();
    setPlaying(null);
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
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
      setDraft(emptyDraft);
      setEditing(null);
      reload();
    } catch (failure) {
      const full = failure instanceof ApiError && failure.status === 409;
      setMutationError(full ? t("radio.full") : t("radio.saveError"));
    }
  }

  async function remove(station: RadioStation) {
    setMutationError(null);
    try {
      await deleteRadioStation(station.id);
      if (playing?.id === station.id) stopPlaying();
      if (editing?.id === station.id) {
        setEditing(null);
        setDraft(emptyDraft);
      }
      reload();
    } catch {
      setMutationError(t("radio.deleteError"));
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
          />
        </label>
        {addressRefused || homepageRefused ? (
          <p className="error">{t("radio.addressRefused")}</p>
        ) : null}
        {isBlockedAsMixedContent(draft.stream_url, window.location.protocol) ? (
          <p className="muted">{t("radio.mixedContent")}</p>
        ) : null}
        <div className="actions">
          <button type="submit" disabled={!submittable}>
            {editing ? t("radio.save") : t("radio.add")}
          </button>
          {editing ? (
            <button
              type="button"
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
                  playing?.id === station.id
                    ? stopPlaying()
                    : startPlaying(station)
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
