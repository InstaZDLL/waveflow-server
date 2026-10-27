import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { formatDuration } from "../../api";
import { Waiting } from "../../components/layout/page-shell";
import {
  forgetOffline,
  type KeptSong,
  keptSongs,
  offlineSource,
  offlineSupported,
  onOfflineChange,
} from "../../lib/offline";
import { useI18n } from "../../state/i18n";

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
  // The playing track by id, and how many times it was chosen: a list that
  // is read again — a track forgotten above it, a copy added — must neither
  // switch what plays nor start it over, and choosing the same track again
  // must start it over.
  const [current, setCurrent] = useState<{ id: string; play: number } | null>(
    null,
  );
  const [source, setSource] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const choose = (id: string) =>
    setCurrent((previous) => ({ id, play: (previous?.play ?? 0) + 1 }));

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

  const playingId = current?.id ?? null;
  const position = songs?.findIndex((song) => song.id === playingId) ?? -1;
  useEffect(() => {
    // The last track's address never plays the next one, even for the
    // moment the next one takes to resolve.
    setSource(null);
    setFailed(false);
    if (playingId === null) return;
    let revoke: () => void = () => {};
    let live = true;
    offlineSource(playingId)
      .then((found) => {
        if (!live) {
          found?.revoke();
          return;
        }
        if (!found) {
          setFailed(true);
          return;
        }
        revoke = found.revoke;
        setSource(found.url);
      })
      .catch(() => {
        if (live) setFailed(true);
      });
    return () => {
      live = false;
      revoke();
    };
  }, [playingId]);

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
              on the track and on how many times it was chosen, so choosing it
              again starts it over and a list read again does not. */}
          <audio
            key={current ? `${current.id}#${current.play}` : "idle"}
            className="share-audio"
            aria-label={t("offline.player")}
            controls
            autoPlay
            src={source ?? undefined}
            onEnded={() => {
              const next = songs[position + 1];
              if (position >= 0 && next) choose(next.id);
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
            {songs.map((song) => (
              <li
                key={song.id}
                aria-current={song.id === playingId ? "true" : undefined}
              >
                <button type="button" onClick={() => choose(song.id)}>
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
                    if (song.id === playingId) setCurrent(null);
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
