import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";

import {
  formatDuration,
  openPublicShare,
  type PublicShareTrack,
} from "../../api";
import { Loading, Waiting } from "../../components/layout/page-shell";
import { type CanPlayType, sharePlayer } from "../../lib/share-playback";
import { useI18n } from "../../state/i18n";

/**
 * What a share link opens in a browser (RFC-020 decision 3).
 *
 * Outside the signed-in shell, because the person it was sent to has no
 * account: no rail, no library picker, no session. The server handed this
 * page out without counting anything; the query below is the visit.
 *
 * **Its own `<audio>`, not the app's player.** The player plays through
 * stream tickets, which need a session to mint; a share's tracks carry their
 * own addresses, and those are all this page has.
 */
export function SharePage({ token }: { token: string }) {
  const { locale, t } = useI18n();
  // Asked once for the life of the page. A visit is counted per request, so
  // a refetch on focus or on reconnect would spend a capped share's visits on
  // nothing the visitor did.
  const { data, error, isPending } = useQuery({
    queryKey: ["public-share", token],
    queryFn: () => openPublicShare(token),
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: Number.POSITIVE_INFINITY,
    retry: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
  const [current, setCurrent] = useState<number | null>(null);
  // Counts the rows chosen, and keys the player: each choice is a fresh
  // element that starts its track, even a repeated copy whose address is the
  // one already loaded. Not bumped when the player's own button starts the
  // first row, so that element keeps playing.
  const [choices, setChoices] = useState(0);
  // A cover that would not load gives way to the plain square, rather than
  // a broken image beside the title.
  const [brokenCovers, setBrokenCovers] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const canPlayType = useMemo<CanPlayType>(() => {
    const probe = document.createElement("audio");
    return (mediaType) => probe.canPlayType(mediaType);
  }, []);

  if (isPending) {
    return (
      <main className="centered">
        <Waiting />
      </main>
    );
  }
  if (error) {
    return (
      <main className="centered">
        <Loading error={error} />
      </main>
    );
  }
  if (!data) {
    return (
      <main className="centered">
        <section className="not-found">
          <span className="eyebrow">WaveFlow</span>
          <h1>{t("share.unavailableTitle")}</h1>
          <p className="muted">{t("share.unavailableDetail")}</p>
        </section>
      </main>
    );
  }

  const tracks: PublicShareTrack[] = data.tracks;
  const player = sharePlayer(tracks, current, canPlayType);
  // A share may hold a track twice, so the id alone does not name a row: each
  // copy is told apart by how many came before it, as the song table does.
  const seen = new Map<string, number>();
  const rowIds = tracks.map((track) => {
    const copy = seen.get(track.id) ?? 0;
    seen.set(track.id, copy + 1);
    return `${track.id}#${copy}`;
  });

  function play(position: number) {
    setCurrent(position);
    setChoices((count) => count + 1);
  }

  return (
    <main className="share-page">
      <header className="share-header">
        <span className="page-kicker">{t("share.kicker")}</span>
        <h1>{data.description ?? t("share.defaultTitle")}</h1>
        <p className="muted">
          {t("common.tracks", { count: tracks.length })}
          {data.expiresAt !== null
            ? ` · ${t("share.until", {
                date: new Intl.DateTimeFormat(locale, {
                  dateStyle: "medium",
                }).format(data.expiresAt),
              })}`
            : null}
        </p>
      </header>
      {tracks.length ? (
        <>
          {/* Holds the first track before any row is chosen, so its own play
              button plays it: with no source it answered "Unable to play
              media". `autoPlay` only once a row was chosen, and nothing
              preloaded before, so opening the page plays and fetches
              nothing. Played from its own button, it marks the first row
              as the one playing without a new element. */}
          <audio
            key={choices}
            className="share-audio"
            aria-label={t("share.player")}
            controls
            autoPlay={player.autoPlay}
            preload={player.preload}
            src={player.src}
            onPlay={() => {
              if (current === null) setCurrent(player.position);
            }}
            onEnded={() => {
              if (player.position + 1 < tracks.length) {
                play(player.position + 1);
              }
            }}
          >
            <track kind="captions" />
          </audio>
          <ol className="share-tracks">
            {tracks.map((track, position) => (
              <li
                key={rowIds[position]}
                aria-current={position === current ? "true" : undefined}
              >
                <button type="button" onClick={() => play(position)}>
                  {/* Decorative: the title beside it says what it is. A
                      plain address under the visit's pass, so no session is
                      needed to load it (RFC-020). */}
                  {track.coverUrl && !brokenCovers.has(rowIds[position]) ? (
                    <img
                      className="share-track-cover"
                      src={track.coverUrl}
                      alt=""
                      loading="lazy"
                      referrerPolicy="no-referrer"
                      onError={() =>
                        setBrokenCovers((broken) =>
                          new Set(broken).add(rowIds[position]),
                        )
                      }
                    />
                  ) : (
                    <span className="share-track-cover" aria-hidden="true" />
                  )}
                  <span className="share-track-title">{track.title}</span>
                  <span className="muted">
                    {track.artist ?? t("common.unknownArtist")}
                  </span>
                  <span className="muted share-track-duration">
                    {formatDuration(track.duration_ms)}
                  </span>
                </button>
              </li>
            ))}
          </ol>
        </>
      ) : (
        <p className="muted">{t("share.empty")}</p>
      )}
    </main>
  );
}
