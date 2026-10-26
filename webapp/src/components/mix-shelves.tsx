import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";

import { getMix, type MixSummary, type Song } from "../api";
import { mixesQuery } from "../api/queries";
import { mixTitle } from "../lib/mixes";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";
import { useScopeId } from "../state/library-scope";
import { useLocalDay } from "../state/local-day";
import { Artwork } from "./artwork";
import { Icon } from "./icons";

/**
 * A mix's cover: four of its sleeves in a square where it has four, the one
 * it has otherwise, and a "Mix" marker on it either way.
 *
 * The mosaic is what a playlist looks like and what no album does, so a mix is
 * told from a record at a glance; the marker says it in words for the mix
 * with a single cover, which would otherwise be drawn exactly as an album is.
 * The marker is text inside the link, so it is part of what the link is
 * called as well as what it shows.
 */
function MixCover({ hashes, title }: { hashes: string[]; title: string }) {
  const { t } = useI18n();
  const four = hashes.length >= 4 ? hashes.slice(0, 4) : null;
  return (
    <span className="mix-cover">
      {four ? (
        <span className="mix-mosaic">
          {four.map((hash) => (
            <Artwork key={hash} artworkId={hash} title={title} />
          ))}
        </span>
      ) : (
        <Artwork artworkId={hashes[0] ?? null} title={title} />
      )}
      <span className="mix-badge">
        <Icon name="random" size={12} />
        {t("mixes.badge")}
      </span>
    </span>
  );
}

/**
 * The mixes the server drew for this account today (RFC-024 decision 7), as
 * one shelf above the albums.
 *
 * **Nothing at all while there is nothing to offer**, and nothing when the
 * question fails: the shelf is a suggestion on top of a page that has its own
 * content, and an error box there would put a failure the visitor did not ask
 * about above the albums they came for.
 */
export function MixShelves() {
  const { t } = useI18n();
  const scope = useScopeId();
  const player = usePlayer();
  const day = useLocalDay();
  const { data: mixes } = useQuery(mixesQuery(scope, day));
  // A set, for the reason the album grid gives: two cards may be fetching at
  // once, and one slot let the first answer re-enable the other's buttons.
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  // The mix a press could not open, named in the message: a button that
  // does nothing when pressed reads as a button that is broken.
  const [failed, setFailed] = useState<string | null>(null);

  async function withSongs(mix: MixSummary, use: (songs: Song[]) => void) {
    setPending((current) => new Set(current).add(mix.id));
    setFailed(null);
    try {
      const full = await getMix(mix.id, scope);
      if (full.songs.length) use(full.songs);
    } catch {
      setFailed(mixTitle(mix, t));
    } finally {
      setPending((current) => {
        const next = new Set(current);
        next.delete(mix.id);
        return next;
      });
    }
  }

  if (!mixes?.length) return null;
  return (
    <section className="mix-shelf" aria-labelledby="mix-shelf-title">
      {/* Says what these are before anybody has to guess: generated
          playlists, redrawn each day, and not more albums. */}
      <header className="mix-shelf-header">
        <h3 id="mix-shelf-title">{t("mixes.shelfTitle")}</h3>
        <p className="muted mix-shelf-detail">{t("mixes.shelfDetail")}</p>
      </header>
      {failed ? (
        <p className="error" role="alert">
          {t("mixes.failed", { title: failed })}
        </p>
      ) : null}
      <ul>
        {mixes.map((mix) => {
          const title = mixTitle(mix, t);
          return (
            <li key={mix.id}>
              <Link
                className="mix-card"
                to="/mixes/$mixId"
                params={{ mixId: mix.id }}
              >
                <MixCover hashes={mix.artwork_hashes} title={title} />
                <strong className="mix-card-title">{title}</strong>
                <span className="muted mix-card-detail">
                  {t("common.tracks", { count: mix.track_count })}
                </span>
              </Link>
              <div className="card-actions">
                <button
                  type="button"
                  className="card-action"
                  disabled={pending.has(mix.id)}
                  aria-label={`${t("card.play")}: ${title}`}
                  onClick={() =>
                    void withSongs(mix, (songs) => player.play(songs, 0))
                  }
                >
                  <Icon name="play" size={16} />
                </button>
                <button
                  type="button"
                  className="card-action"
                  disabled={pending.has(mix.id)}
                  aria-label={`${t("card.queue")}: ${title}`}
                  onClick={() => void withSongs(mix, player.enqueue)}
                >
                  <Icon name="queue" size={16} />
                </button>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
