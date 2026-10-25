import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";

import { getMix, type MixSummary, type Song } from "../api";
import { mixesQuery } from "../api/queries";
import { mixTitle } from "../lib/mixes";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";
import { useScopeId } from "../state/library-scope";
import { Artwork } from "./artwork";
import { Icon } from "./icons";

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
  const { data: mixes } = useQuery(mixesQuery(scope));
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
      <h3 id="mix-shelf-title">{t("mixes.title")}</h3>
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
                <Artwork
                  artworkId={mix.artwork_hashes[0] ?? null}
                  title={title}
                />
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
