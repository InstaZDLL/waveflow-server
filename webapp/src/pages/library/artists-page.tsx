import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useMemo, useState } from "react";

import { artistsQuery } from "../api/queries";
import { Artwork } from "../components/artwork";
import { FilterField } from "../components/filter-field";
import { EmptyState, Loading, PageHeader } from "../components/page-shell";
import { matches, normalizeFilter } from "../lib/filter";
import { useI18n } from "../state/i18n";
import { useScopeId } from "../state/library-scope";

export function ArtistsPage() {
  const { t } = useI18n();
  const [filter, setFilter] = useState("");
  const scope = useScopeId();
  const { data: value, error } = useQuery(artistsQuery(scope));
  const needle = normalizeFilter(filter);
  // `/api/v2/artists` takes no `sort`, unlike `/albums`: the one order the
  // server offers is alphabetical, so this page filters and does not sort.
  const shown = useMemo(
    () =>
      !value || !needle
        ? (value ?? [])
        : value.filter((artist) => matches(artist.name, needle)),
    [value, needle],
  );
  if (!value) return <Loading error={error} />;
  if (value.length === 0)
    return <p className="muted">{t("common.noArtists")}</p>;
  return (
    <section>
      <PageHeader
        kicker={t("nav.group.library")}
        title={t("nav.artists")}
        detail={
          needle
            ? t("browse.matches", { count: shown.length, total: value.length })
            : t("artists.detail", { count: value.length })
        }
      >
        <FilterField
          label={t("browse.filterArtists")}
          value={filter}
          onChange={setFilter}
        />
      </PageHeader>
      {needle && shown.length === 0 ? (
        <EmptyState message={t("browse.noMatch")} />
      ) : null}
      <ul className="list artist-list">
        {shown.map((artist) => (
          <li key={artist.id}>
            <Link to="/artists/$artistId" params={{ artistId: artist.id }}>
              <Artwork
                artworkId={artist.artwork_hash}
                title={artist.name}
                className="artist-avatar"
              />
              <strong>{artist.name}</strong>
            </Link>
            <span className="muted">
              {t("common.albums", { count: artist.album_count })}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
