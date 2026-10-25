import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { formatDuration } from "../api";
import { Waiting } from "../components/page-shell";
import {
  forgetOffline,
  type KeptSong,
  keptSongs,
  offlineSource,
  offlineSupported,
  onOfflineChange,
} from "../lib/offline";
import { useI18n } from "../state/i18n";

/**
 * The tracks kept for offline listening (RFC-022), and a player for them.
 *
 * Outside the signed-in shell, as the share page is, because it must open
 * with no network: no session to refresh, no catalogue to ask. Everything it
 * shows was written into this browser when the track was kept, and it plays
 * from there. **Its own `<audio>`**, for the reason the share page has one:
 * the app's player plays through stream tickets, which need the server.
 */
export function OfflinePage() {
  const { t, locale } = useI18n();
  const [songs, setSongs] = useState<KeptSong[] | null>(null);
  const [current, setCurrent] = useState<number | null>(null);
  const [source, setSource] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let live = true;
    const read = () =>
      void keptSongs()
        .then((kept) => live && setSongs(kept))
        .catch(() => live && setSongs([]));
    read();
    const stop = onOfflineChange(read);
    return () => {
      live = false;
      stop();
    };
  }, []);

  const playing = current === null ? null : (songs?.[current] ?? null);
  useEffect(() => {
    if (!playing) {
      setSource(null);
      return;
    }
    let revoke: () => void = () => {};
    let live = true;
    void offlineSource(playing.id).then((found) => {
      if (!live) {
        found?.revoke();
        return;
      }
      if (!found) {
        setFailed(true);
        return;
      }
      revoke = found.revoke;
      setFailed(false);
      setSource(found.url);
    });
    return () => {
      live = false;
      revoke();
    };
  }, [playing]);

  const size = new Intl.NumberFormat(locale, {
    style: "unit",
    unit: "megabyte",
    maximumFractionDigits: 1,
  });

  return (
    <main className="share-page offline-page">
      <header className="share-header">
        <span className="page-kicker">{t("offline.kicker")}</span>
        <h1>{t("offline.title")}</h1>
        <p className="muted">
          {offlineSupported() ? t("offline.detail") : t("offline.unsupported")}
        </p>
        <p>
          <Link to="/">{t("offline.back")}</Link>
        </p>
      </header>
      {songs === null ? (
        <Waiting />
      ) : songs.length === 0 ? (
        <p className="muted">{t("offline.empty")}</p>
      ) : (
        <>
          {/* `autoPlay` only ever acts on a source a click put there. Keyed
              on the position, so the same track chosen again starts over. */}
          <audio
            key={current ?? "idle"}
            className="share-audio"
            aria-label={t("offline.player")}
            controls
            autoPlay
            src={source ?? undefined}
            onEnded={() => {
              if (current !== null && current + 1 < songs.length) {
                setCurrent(current + 1);
              }
            }}
          >
            <track kind="captions" />
          </audio>
          {failed ? (
            <p className="error" role="alert">
              {t("offline.gone")}
            </p>
          ) : null}
          <ol className="share-tracks offline-tracks">
            {songs.map((song, position) => (
              <li
                key={song.id}
                aria-current={position === current ? "true" : undefined}
              >
                <button type="button" onClick={() => setCurrent(position)}>
                  <span className="share-track-cover" aria-hidden="true" />
                  <span className="share-track-title">{song.title}</span>
                  <span className="muted">
                    {song.artist ?? t("common.unknownArtist")}
                  </span>
                  <span className="muted share-track-duration">
                    {formatDuration(song.duration_ms)}
                  </span>
                </button>
                <button
                  type="button"
                  className="row-action offline-forget"
                  onClick={() => {
                    if (position === current) setCurrent(null);
                    void forgetOffline(song.id);
                  }}
                  aria-label={t("offline.forget", { title: song.title })}
                  title={t("offline.forgetDetail", {
                    size: size.format(song.bytes / 1_000_000),
                  })}
                >
                  {t("offline.forgetShort")}
                </button>
              </li>
            ))}
          </ol>
        </>
      )}
    </main>
  );
}
