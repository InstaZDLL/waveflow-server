import type { Song } from "../api";

/** One artist a track can link to. */
export type CreditedArtist = { id: string; name: string };

/**
 * The artists a track can link to, in tag order.
 *
 * The structured list the server sends, where it has one. Otherwise the
 * primary credit, if the catalogue knows it by an id, and nothing at all for
 * an artist it does not know: a name with no page behind it is not a link.
 */
export function creditedArtists(
  song: Pick<Song, "artist" | "artist_id" | "artists">,
): CreditedArtist[] {
  if (song.artists && song.artists.length > 0) return song.artists;
  if (song.artist_id && song.artist) {
    return [{ id: song.artist_id, name: song.artist }];
  }
  return [];
}
