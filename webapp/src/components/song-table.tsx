import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";

import { formatDuration, type Song, setFavorite, setRating } from "../api";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";
import { mayCorrectTracks, useLibraryScope } from "../state/library-scope";
import { Artwork } from "./artwork";
import { Icon } from "./icons";
import { useTrackMenu } from "./track-menu";

const RATING_STARS = [1, 2, 3, 4, 5];

/**
 * Five stars, where clicking the star already set clears the rating — the
 * server spells that `rating: 0`, and without it a rating could be changed but
 * never taken back.
 *
 * The radio group is the accessible shape for "exactly one of five", and it
 * gives arrow-key selection for free. Each group needs a `name` unique to the
 * row, or every rating on a page would belong to one group.
 */
function StarRating({
  label,
  value,
  name,
  onRate,
}: {
  label: string;
  value: number;
  name: string;
  onRate: (rating: number) => void;
}) {
  const { t } = useI18n();
  return (
    <fieldset className="rating" aria-label={label}>
      {RATING_STARS.map((star) => (
        <label key={star} className={star <= value ? "on" : undefined}>
          <input
            type="radio"
            name={name}
            checked={star === value}
            aria-label={t("rating.stars", { count: star })}
            onChange={() => onRate(star)}
            onClick={() => star === value && onRate(0)}
          />
          <span aria-hidden="true">★</span>
        </label>
      ))}
    </fieldset>
  );
}

/**
 * A track's number belongs to its sleeve, so it is only meaningful where the
 * sleeve is on screen. Everywhere else — favourites, a playlist, a genre, a
 * history, a draw, a search — the number that means something is the row's
 * place in the list being shown, which is why that is the default.
 */
export function SongTable({
  songs,
  numbering = "position",
  variant = "default",
  captions,
}: {
  songs: Song[];
  numbering?: "track" | "position";
  variant?: "default" | "album";
  /**
   * What each row says under its title, in place of the album: the file's
   * name, where the screen is about files.
   */
  captions?: string[];
}) {
  const player = usePlayer();
  const { t } = useI18n();
  // The two maps below are an optimistic overlay, and they live exactly as long
  // as this table does. That was enough while every screen re-read on mount: the
  // server's answer replaced them on the way back. Against a cache it is not —
  // the overlay dies with the component and the held list still carries the old
  // `starred_at`, so a star would un-tick itself on the next visit.
  //
  // Everything is invalidated after a mutation, for the reason a correction is:
  // a song appears in albums, genres, search, history, favourites, playlists and
  // the queue, and a list of the caches that could be wrong is a list with one
  // missing from it.
  const client = useQueryClient();
  /**
   * Everything the mutation could have made wrong, **except the draw**.
   *
   * This table is rendered on the shuffle page too, and that query is the one
   * question here deliberately kept out of the cache: invalidating it would
   * re-ask the server for a *different* sample, so starring a track pulled the
   * list out from under the hand that starred it — and took the track with it.
   * A star says something about one song and nothing about which songs were
   * drawn.
   */
  const reloadAffected = () =>
    client.invalidateQueries({
      predicate: (query) => query.queryKey[0] !== "random-songs",
    });
  const [stars, setStars] = useState<Record<string, boolean>>({});
  const [ratings, setRatings] = useState<Record<string, number>>({});
  const { libraries } = useLibraryScope();
  const { rowProps, menu } = useTrackMenu();
  // Offered only where the server would accept the correction. It refuses
  // anyway; this keeps the table from offering a refusal on every row.
  const mayCorrect = (song: Song) =>
    mayCorrectTracks(
      libraries.find((library) => library.id === song.library_id),
    );

  async function toggleStar(song: Song) {
    const on = !(stars[song.id] ?? song.starred_at !== null);
    setStars((previous) => ({ ...previous, [song.id]: on }));
    try {
      await setFavorite("track", song.id, on);
      await reloadAffected();
    } catch {
      setStars((previous) => ({ ...previous, [song.id]: !on }));
    }
  }

  async function rate(song: Song, rating: number) {
    const previous = ratings[song.id] ?? song.user_rating ?? 0;
    setRatings((current) => ({ ...current, [song.id]: rating }));
    try {
      await setRating("track", song.id, rating);
      await reloadAffected();
    } catch {
      setRatings((current) => ({ ...current, [song.id]: previous }));
    }
  }

  // A playlist can hold the same track twice, so the track id alone does not
  // name a row: each copy is told apart by how many came before it.
  const seen = new Map<string, number>();
  const rowIds = songs.map((song) => {
    const copy = seen.get(song.id) ?? 0;
    seen.set(song.id, copy + 1);
    return `${song.id}#${copy}`;
  });

  return (
    <div className="songs-scroll">
      <table className={`songs${variant === "album" ? " album-songs" : ""}`}>
        {variant === "album" ? (
          <thead>
            <tr>
              <th scope="col" className="index">
                #
              </th>
              <th scope="col">{t("album.title")}</th>
              <th scope="col" className="album-duration-heading">
                {t("album.duration")}
              </th>
              <th scope="col" className="album-actions-heading">
                {t("album.trackActions")}
              </th>
            </tr>
          </thead>
        ) : null}
        <tbody>
          {songs.map((song, position) => {
            const starred = stars[song.id] ?? song.starred_at !== null;
            const active = player.current?.id === song.id;
            return (
              <tr
                key={rowIds[position]}
                className={active ? "active" : undefined}
                {...rowProps(songs, position)}
              >
                <td className="index">
                  {numbering === "track"
                    ? (song.track ?? position + 1)
                    : position + 1}
                </td>
                <td>
                  <button
                    type="button"
                    className="link song-title"
                    onClick={() => player.play(songs, position)}
                  >
                    {variant === "default" ? (
                      <Artwork
                        artworkId={song.artwork_hash}
                        title={song.title}
                        className="song-cover"
                      />
                    ) : null}
                    <span>
                      {song.title}
                      <small>
                        {captions?.[position] ??
                          (variant === "album" ? song.artist : song.album)}
                      </small>
                    </span>
                  </button>
                </td>
                {variant === "default" ? (
                  <td className="muted">{song.artist}</td>
                ) : null}
                <td className="muted song-duration">
                  {formatDuration(song.duration_ms)}
                </td>
                <td className="song-marks">
                  <div>
                    <StarRating
                      label={`${t("rating.label")}: ${song.title}`}
                      name={`rating-${rowIds[position]}`}
                      value={ratings[song.id] ?? song.user_rating ?? 0}
                      onRate={(rating) => void rate(song, rating)}
                    />
                    <button
                      type="button"
                      className="star"
                      onClick={() => void toggleStar(song)}
                      aria-label={`${
                        starred ? t("favourites.remove") : t("favourites.add")
                      }: ${song.title}`}
                      aria-pressed={starred}
                    >
                      <Icon name="heart" size={16} filled={starred} />
                    </button>
                    {mayCorrect(song) ? (
                      <Link
                        to="/tracks/$trackId/edit"
                        params={{ trackId: song.id }}
                        className="row-action"
                        aria-label={t("correction.editTrack", {
                          title: song.title,
                        })}
                      >
                        <Icon name="edit" size={16} />
                      </Link>
                    ) : null}
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {menu}
    </div>
  );
}
