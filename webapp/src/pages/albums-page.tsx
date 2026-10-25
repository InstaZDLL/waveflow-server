import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";

import type { AlbumSort } from "../api";
import { albumsQuery } from "../api/queries";
import { AlbumGrid } from "../components/album-grid";
import { FilterField } from "../components/filter-field";
import { MixShelves } from "../components/mix-shelves";
import { EmptyState, Loading, PageHeader } from "../components/page-shell";
import { matches, normalizeFilter } from "../lib/filter";
import { type TranslationKey, useI18n } from "../state/i18n";
import { useScopeId } from "../state/library-scope";

/**
 * The orders offered on the albums page, in menu order. Four of them narrow the
 * catalogue as well as reordering it — the server answers `frequent`, `recent`
 * and `starred` only for albums that have a play count, a last play or a star,
 * and `byYear` only for those carrying a year — which is why the count in the
 * header is read off the response.
 */
const ALBUM_SORTS: Array<{ value: AlbumSort; labelKey: TranslationKey }> = [
  { value: "alphabeticalByName", labelKey: "browse.sortAlphabeticalByName" },
  {
    value: "alphabeticalByArtist",
    labelKey: "browse.sortAlphabeticalByArtist",
  },
  { value: "newest", labelKey: "browse.sortNewest" },
  { value: "recent", labelKey: "browse.sortRecent" },
  { value: "frequent", labelKey: "browse.sortFrequent" },
  { value: "starred", labelKey: "browse.sortStarred" },
  { value: "byYear", labelKey: "browse.sortByYear" },
];

/** Render the album catalogue for the active library. */
export function AlbumsPage() {
  const { t } = useI18n();
  const [sort, setSort] = useState<AlbumSort>("alphabeticalByName");
  const [filter, setFilter] = useState("");
  const scope = useScopeId();
  const { data: value, error } = useQuery(albumsQuery(sort, scope));
  const needle = normalizeFilter(filter);
  const shown = useMemo(
    () =>
      !value || !needle
        ? (value ?? [])
        : value.filter(
            (album) =>
              matches(album.title, needle) || matches(album.artist, needle),
          ),
    [value, needle],
  );

  const controls = (
    <>
      <label className="control">
        <span>{t("browse.sort")}</span>
        <select
          value={sort}
          aria-label={t("browse.sort")}
          onChange={(event) => setSort(event.target.value as AlbumSort)}
        >
          {ALBUM_SORTS.map((option) => (
            <option key={option.value} value={option.value}>
              {t(option.labelKey)}
            </option>
          ))}
        </select>
      </label>
      <FilterField
        label={t("browse.filterAlbums")}
        value={filter}
        onChange={setFilter}
      />
    </>
  );

  if (!value) {
    return (
      <section>
        {/* No detail line: the count it would carry is exactly what is still
            missing, and `Loading` already says whether this is a wait or a
            failure. */}
        <PageHeader kicker={t("nav.group.library")} title={t("nav.albums")}>
          {controls}
        </PageHeader>
        <Loading error={error} />
      </section>
    );
  }
  return (
    <section>
      <PageHeader
        kicker={t("nav.group.library")}
        title={t("nav.albums")}
        detail={
          needle
            ? t("browse.matches", { count: shown.length, total: value.length })
            : t("albums.detail", { count: value.length })
        }
      >
        {controls}
      </PageHeader>
      {/* Above the albums and not inside their filter: the shelf is what the
          server suggests today, and narrowing the grid does not narrow it. */}
      {needle ? null : <MixShelves />}
      {needle && shown.length === 0 ? (
        <EmptyState message={t("browse.noMatch")} />
      ) : (
        <AlbumGrid albums={shown} />
      )}
    </section>
  );
}
