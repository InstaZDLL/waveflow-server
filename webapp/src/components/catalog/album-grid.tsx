import { Link } from "@tanstack/react-router";
import { useState } from "react";

import { type Album, getAlbum, type Song } from "../api";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";
import { Artwork } from "./artwork";
import { Icon } from "./icons";

export function AlbumGrid({ albums }: { albums: Album[] }) {
  const player = usePlayer();
  const { t } = useI18n();
  // The grid holds album summaries, not their tracks, so both actions have to
  // fetch the sleeve first. A failed fetch leaves the queue alone rather than
  // half-filling it.
  //
  // A set and not one id: two albums can be in flight at once, and a single
  // slot let whichever answered first clear the other one's guard. The second
  // album's buttons came back to life while its own request was still out, so
  // a second press on "add to queue" queued the album twice.
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());

  async function withSongs(album: Album, use: (songs: Song[]) => void) {
    setPending((current) => new Set(current).add(album.id));
    try {
      const detail = await getAlbum(album.id);
      if (detail.songs.length) use(detail.songs);
    } catch {
      // Nothing to report here: the album stays reachable through its link.
    } finally {
      setPending((current) => {
        const next = new Set(current);
        next.delete(album.id);
        return next;
      });
    }
  }

  if (albums.length === 0)
    return <p className="muted">{t("common.noAlbums")}</p>;
  return (
    <ul className="grid">
      {albums.map((album) => (
        <li key={album.id}>
          <Link to="/albums/$albumId" params={{ albumId: album.id }}>
            <Artwork artworkId={album.artwork_hash} title={album.title} />
            <strong>{album.title}</strong>
            <span className="muted">
              {album.artist ?? t("common.variousArtists")}
              {album.year ? ` · ${album.year}` : ""}
            </span>
          </Link>
          <div className="card-actions">
            <button
              type="button"
              className="card-action"
              disabled={pending.has(album.id)}
              aria-label={`${t("card.play")}: ${album.title}`}
              onClick={() =>
                void withSongs(album, (songs) => player.play(songs, 0))
              }
            >
              <Icon name="play" size={16} />
            </button>
            <button
              type="button"
              className="card-action"
              disabled={pending.has(album.id)}
              aria-label={`${t("card.queue")}: ${album.title}`}
              onClick={() => void withSongs(album, player.enqueue)}
            >
              <Icon name="queue" size={16} />
            </button>
          </div>
        </li>
      ))}
    </ul>
  );
}
