import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";

import {
  formatDuration,
  openPublicShare,
  type PublicShareTrack,
} from "../../api";
import { Loading, Waiting } from "../../components/layout/page-shell";
import { type CanPlayType, shareSource } from "../../lib/share-playback";
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

  const tracks = data.tracks;
  const playing: PublicShareTrack | null =
    current === null ? null : (tracks[current] ?? null);
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
          {/* `autoPlay` only ever acts on a source a click put there: the
              element has no source until a track is chosen. Keyed on the
              position, so the next copy of a repeated track starts again
              rather than finding its address unchanged and staying silent. */}
          <audio
            key={current ?? "idle"}
            className="share-audio"
            aria-label={t("share.player")}
            controls
            autoPlay
            src={
              playing
                ? shareSource(playing.streamUrl, playing.suffix, canPlayType)
                : undefined
            }
            onEnded={() => {
              if (current !== null && current + 1 < tracks.length) {
                play(current + 1);
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
