import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useMemo, useState } from "react";

import { genresQuery } from "../api/queries";
import { FilterField } from "../components/filter-field";
import { EmptyState, Loading, PageHeader } from "../components/page-shell";
import { matches, normalizeFilter } from "../lib/filter";
import { useI18n } from "../state/i18n";
import { useScopeId } from "../state/library-scope";

export function GenresPage() {
  const { t } = useI18n();
  const [filter, setFilter] = useState("");
  const scope = useScopeId();
  const { data: value, error } = useQuery(genresQuery(scope));
  const needle = normalizeFilter(filter);
  const shown = useMemo(
    () =>
      !value || !needle
        ? (value ?? [])
        : value.filter((genre) => matches(genre.name, needle)),
    [value, needle],
  );
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        title={t("nav.genres")}
        detail={
          needle
            ? t("browse.matches", { count: shown.length, total: value.length })
            : t("genres.detail", { count: value.length })
        }
      >
        <FilterField
          label={t("browse.filterGenres")}
          value={filter}
          onChange={setFilter}
        />
      </PageHeader>
      {value.length === 0 ? (
        <EmptyState message={t("genres.empty")} />
      ) : shown.length === 0 ? (
        <EmptyState message={t("browse.noMatch")} />
      ) : (
        <ul className="list genre-list">
          {shown.map((genre) => (
            <li key={genre.name}>
              <Link to="/genres/$genre" params={{ genre: genre.name }}>
                <strong>{genre.name}</strong>
              </Link>
              <span className="muted">
                {t("common.tracks", { count: genre.song_count })} ·{" "}
                {t("common.albums", { count: genre.album_count })}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
