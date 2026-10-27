import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type CSSProperties, useState } from "react";

import type { StatsPeriod } from "../../api";
import { statsQuery } from "../../api/queries";
import { Artwork } from "../../components/artwork/artwork";
import {
  EmptyState,
  Loading,
  PageHeader,
} from "../../components/layout/page-shell";
import { barShares, listeningTime } from "../../lib/mixes";
import { usePlayer } from "../../player/player";
import { type TranslationKey, useI18n } from "../../state/i18n";
import { useLocalDay } from "../../state/local-day";

const PERIODS: Array<{ value: StatsPeriod; labelKey: TranslationKey }> = [
  { value: "7d", labelKey: "stats.period7d" },
  { value: "30d", labelKey: "stats.period30d" },
  { value: "365d", labelKey: "stats.period365d" },
  { value: "all", labelKey: "stats.periodAll" },
];

/**
 * This account's listening (RFC-024 decision 6): what it played most, how
 * long for, and when. Everything on it is read from the server's own record
 * of listens; nothing is counted here.
 */
export function StatsPage() {
  const { t, locale } = useI18n();
  const player = usePlayer();
  const [period, setPeriod] = useState<StatsPeriod>("30d");
  const day = useLocalDay();
  const { data: stats, error } = useQuery(statsQuery(period, day));

  const controls = (
    <label className="control">
      <span>{t("stats.period")}</span>
      <select
        value={period}
        aria-label={t("stats.period")}
        onChange={(event) => setPeriod(event.target.value as StatsPeriod)}
      >
        {PERIODS.map((option) => (
          <option key={option.value} value={option.value}>
            {t(option.labelKey)}
          </option>
        ))}
      </select>
    </label>
  );

  if (!stats) {
    return (
      <section>
        <PageHeader kicker={t("nav.group.library")} title={t("nav.stats")}>
          {controls}
        </PageHeader>
        <Loading error={error} />
      </section>
    );
  }

  const when = (at: number) =>
    new Date(at).toLocaleString(locale, {
      dateStyle: "medium",
      timeStyle: "short",
    });
  const shares = barShares(stats.timeline.map((bucket) => bucket.plays));

  return (
    <section className="stats-page">
      <PageHeader
        kicker={t("nav.group.library")}
        title={t("nav.stats")}
        detail={t("stats.detail")}
      >
        {controls}
      </PageHeader>
      {stats.totals.plays === 0 ? (
        <EmptyState message={t("stats.empty")} />
      ) : (
        <>
          <dl className="stats-totals">
            <div className="stats-total">
              <dt>{t("stats.plays")}</dt>
              <dd>{stats.totals.plays}</dd>
            </div>
            <div className="stats-total">
              <dt>{t("stats.listeningTime")}</dt>
              <dd>{listeningTime(stats.totals.listening_ms, t)}</dd>
            </div>
            <div className="stats-total">
              <dt>{t("stats.tracks")}</dt>
              <dd>{stats.totals.tracks}</dd>
            </div>
            <div className="stats-total">
              <dt>{t("stats.artists")}</dt>
              <dd>{stats.totals.artists}</dd>
            </div>
            <div className="stats-total">
              <dt>{t("stats.albums")}</dt>
              <dd>{stats.totals.albums}</dd>
            </div>
          </dl>
          <p className="muted">
            <small>{t("stats.estimate")}</small>
          </p>

          <section aria-labelledby="stats-timeline-title">
            <h3 id="stats-timeline-title">
              {t(
                stats.bucket === "day"
                  ? "stats.timelineDays"
                  : "stats.timelineMonths",
              )}
            </h3>
            <ol className="stats-timeline">
              {stats.timeline.map((bucket, index) => (
                <li
                  key={bucket.label}
                  style={{ "--share": shares[index] } as CSSProperties}
                  aria-label={t("stats.bucket", {
                    label: bucket.label,
                    count: bucket.plays,
                  })}
                  title={t("stats.bucket", {
                    label: bucket.label,
                    count: bucket.plays,
                  })}
                >
                  <span aria-hidden="true" />
                </li>
              ))}
            </ol>
          </section>

          <div className="stats-columns">
            <section aria-labelledby="stats-tracks-title">
              <h3 id="stats-tracks-title">{t("stats.topTracks")}</h3>
              <ol className="stats-list">
                {stats.top_tracks.map((entry, index) => (
                  <li key={entry.song.id}>
                    <button
                      type="button"
                      className="stats-entry"
                      onClick={() =>
                        player.play(
                          stats.top_tracks.map((top) => top.song),
                          index,
                        )
                      }
                    >
                      <Artwork
                        artworkId={entry.song.artwork_hash}
                        title={entry.song.title}
                        className="stats-art"
                      />
                      <span className="stats-text">
                        <strong className="stats-name">
                          {entry.song.title}
                        </strong>
                        <span className="muted stats-sub">
                          {entry.song.artist ?? t("common.unknownArtist")}
                        </span>
                      </span>
                    </button>
                    <span className="stats-count">
                      {t("stats.playCount", { count: entry.plays })}
                    </span>
                  </li>
                ))}
              </ol>
            </section>
            <section aria-labelledby="stats-artists-title">
              <h3 id="stats-artists-title">{t("stats.topArtists")}</h3>
              <ol className="stats-list">
                {stats.top_artists.map((entry) => (
                  <li key={entry.artist.id}>
                    <Link
                      className="stats-entry"
                      to="/artists/$artistId"
                      params={{ artistId: entry.artist.id }}
                    >
                      <Artwork
                        artworkId={entry.artwork_hash}
                        title={entry.artist.name}
                        className="stats-art stats-art-round"
                      />
                      <span className="stats-text">
                        <strong className="stats-name">
                          {entry.artist.name}
                        </strong>
                      </span>
                    </Link>
                    <span className="stats-count">
                      {t("stats.playCount", { count: entry.plays })}
                    </span>
                  </li>
                ))}
              </ol>
            </section>
            <section aria-labelledby="stats-albums-title">
              <h3 id="stats-albums-title">{t("stats.topAlbums")}</h3>
              <ol className="stats-list">
                {stats.top_albums.map((entry) => (
                  <li key={entry.album.id}>
                    <Link
                      className="stats-entry"
                      to="/albums/$albumId"
                      params={{ albumId: entry.album.id }}
                    >
                      <Artwork
                        artworkId={entry.album.artwork_hash}
                        title={entry.album.title}
                        className="stats-art"
                      />
                      <span className="stats-text">
                        <strong className="stats-name">
                          {entry.album.title}
                        </strong>
                        <span className="muted stats-sub">
                          {entry.album.artist ?? t("common.variousArtists")}
                        </span>
                      </span>
                    </Link>
                    <span className="stats-count">
                      {t("stats.playCount", { count: entry.plays })}
                    </span>
                  </li>
                ))}
              </ol>
            </section>
            <section aria-labelledby="stats-genres-title">
              <h3 id="stats-genres-title">{t("stats.topGenres")}</h3>
              <ol className="stats-list">
                {stats.top_genres.map((entry) => (
                  <li key={entry.name}>
                    <Link
                      className="stats-entry"
                      to="/genres/$genre"
                      params={{ genre: entry.name }}
                    >
                      <strong className="stats-name">{entry.name}</strong>
                    </Link>
                    <span className="stats-count">
                      {t("stats.playCount", { count: entry.plays })}
                    </span>
                  </li>
                ))}
              </ol>
            </section>
          </div>

          <dl className="stats-moments">
            {stats.first_play ? (
              <div>
                <dt>{t("stats.firstPlay")}</dt>
                <dd>
                  {t("stats.moment", {
                    title: stats.first_play.song.title,
                    when: when(stats.first_play.played_at),
                  })}
                </dd>
              </div>
            ) : null}
            {stats.last_play ? (
              <div>
                <dt>{t("stats.lastPlay")}</dt>
                <dd>
                  {t("stats.moment", {
                    title: stats.last_play.song.title,
                    when: when(stats.last_play.played_at),
                  })}
                </dd>
              </div>
            ) : null}
          </dl>
        </>
      )}
    </section>
  );
}
