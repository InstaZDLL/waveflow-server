import { useInfiniteQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";

import type { SongSort } from "../api";
import { songsQuery } from "../api/queries";
import { Icon } from "../components/icons";
import { Loading, PageHeader } from "../components/page-shell";
import { SongTable } from "../components/song-table";
import { usePlayer } from "../player/player";
import { type TranslationKey, useI18n } from "../state/i18n";
import { useScopeId } from "../state/library-scope";

/**
 * The orderings this menu offers, in the album menu's order plus one.
 *
 * `alphabeticalByAlbum` has no album equivalent: it is sleeve order read
 * across the whole catalogue, album then disc then track.
 */
const SONG_SORTS: Array<{ value: SongSort; labelKey: TranslationKey }> = [
  { value: "alphabeticalByName", labelKey: "browse.sortAlphabeticalByName" },
  {
    value: "alphabeticalByArtist",
    labelKey: "browse.sortAlphabeticalByArtist",
  },
  { value: "alphabeticalByAlbum", labelKey: "browse.sortAlphabeticalByAlbum" },
  { value: "newest", labelKey: "browse.sortNewest" },
  { value: "recent", labelKey: "browse.sortRecent" },
  { value: "frequent", labelKey: "browse.sortFrequent" },
  { value: "starred", labelKey: "browse.sortStarred" },
  { value: "byYear", labelKey: "browse.sortByYear" },
];

/**
 * Every track in the catalogue, sorted, a page at a time.
 *
 * The listing the client never had: albums, artists, genres, favourites,
 * history and random were all here, and "my tracks, sorted" was not.
 *
 * **There is no filter box, and that is the decision rather than the
 * omission.** Every other browse page filters what it holds, because it holds
 * everything. This one holds the pages it has asked for, so the same box would
 * search two hundred rows out of twenty thousand and call the rest absent —
 * a worse answer than none, because nothing on screen would say so. Searching
 * the catalogue is what `/search` does, against the index.
 */
export function SongsPage() {
  const player = usePlayer();
  const { t } = useI18n();
  const [sort, setSort] = useState<SongSort>("alphabeticalByName");
  const scope = useScopeId();
  const {
    data,
    error,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
    isFetchNextPageError,
  } = useInfiniteQuery(songsQuery(sort, scope));
  const songs = useMemo(() => data?.pages.flat() ?? [], [data]);

  const controls = (
    <>
      <label className="control">
        <span>{t("browse.sort")}</span>
        <select
          value={sort}
          aria-label={t("browse.sort")}
          onChange={(event) => setSort(event.target.value as SongSort)}
        >
          {SONG_SORTS.map((option) => (
            <option key={option.value} value={option.value}>
              {t(option.labelKey)}
            </option>
          ))}
        </select>
      </label>
      {songs.length ? (
        <button
          className="action-primary"
          type="button"
          onClick={() => player.play(songs, 0)}
        >
          <Icon name="play" size={17} />
          {hasNextPage
            ? t("songs.playLoaded", { count: songs.length })
            : t("songs.playAll", { count: songs.length })}
        </button>
      ) : null}
    </>
  );

  if (!data) {
    return (
      <section>
        <PageHeader kicker={t("nav.group.library")} title={t("nav.songs")}>
          {controls}
        </PageHeader>
        <Loading error={error} />
      </section>
    );
  }

  return (
    <section>
      {/* The count is what has been loaded, not what exists, and it says so:
          the route sends no total, and inventing one would mean a second
          `COUNT(*)` over the same filter on every request. */}
      <PageHeader
        kicker={t("nav.group.library")}
        title={t("nav.songs")}
        detail={
          hasNextPage
            ? t("songs.loadedSoFar", { count: songs.length })
            : t("common.tracks", { count: songs.length })
        }
      >
        {controls}
      </PageHeader>
      {songs.length ? (
        <SongTable songs={songs} />
      ) : (
        <p className="muted">{t("songs.empty")}</p>
      )}
      {hasNextPage && (
        <div className="actions">
          <button
            type="button"
            onClick={() => fetchNextPage()}
            disabled={isFetchingNextPage}
          >
            {isFetchingNextPage ? t("songs.loading") : t("songs.loadMore")}
          </button>
          {/* A page that failed to load has to say so. `Loading` is no use
              here — the pages already loaded are on screen and must stay
              there, so this is a line beside the button that asks again, not
              a state replacing the listing. Without it the button simply came
              back to life and the catalogue silently stopped growing. */}
          {isFetchNextPageError ? (
            <p className="error" role="alert">
              {t("songs.loadMoreError")}
            </p>
          ) : null}
        </div>
      )}
    </section>
  );
}
