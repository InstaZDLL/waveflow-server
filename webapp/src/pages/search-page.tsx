import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { searchQuery } from "../api/queries";
import { AlbumGrid } from "../components/album-grid";
import { Loading, PageHeader } from "../components/page-shell";
import { SongTable } from "../components/song-table";
import { useI18n } from "../state/i18n";
import { useLibraryScope } from "../state/library-scope";

/**
 * How long the typing has to settle before the search is asked.
 *
 * A quarter of a second: long enough that a word typed at speed asks once,
 * short enough that the answer feels like a consequence of the typing rather
 * than of stopping. The submit button stays for keyboard submission, and it
 * does not wait.
 */
const SEARCH_DEBOUNCE_MS = 250;

export function SearchPage() {
  const { t } = useI18n();
  const { libraries, active } = useLibraryScope();
  const scope = active?.id;
  const [query, setQuery] = useState("");
  const [submitted, setSubmitted] = useState("");
  const { data: value, error } = useQuery(searchQuery(submitted, scope));

  /**
   * The question follows the typing, once the typing pauses.
   *
   * The screen used to wait for a click, so a search was a decision rather
   * than a reflex. Asking on every keystroke is the other mistake: one round
   * trip per letter, and the server answering six questions to satisfy one.
   *
   * `submitted` is in the dependencies so a needle that has already been
   * asked — the submit button beat the timer to it — schedules nothing.
   * Clearing the box settles on the empty needle, which `searchQuery` holds
   * disabled rather than asking the server for everything.
   */
  useEffect(() => {
    const needle = query.trim();
    if (needle === submitted) return;
    const timer = setTimeout(() => setSubmitted(needle), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query, submitted]);

  return (
    <section>
      <PageHeader title={t("nav.search")} detail={t("search.detail")} />
      {/* Said only where it is a fact worth saying. This screen used to warn
          that it covered every library, because it did; it is scoped like the
          others now, and an account with one library is not choosing. */}
      {libraries.length > 1 && active ? (
        <p className="muted scope-note">
          {t("search.thisLibrary", { library: active.name })}
        </p>
      ) : null}
      <form
        className="search"
        onSubmit={(event) => {
          event.preventDefault();
          setSubmitted(query.trim());
        }}
      >
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("search.placeholder")}
          aria-label={t("search.query")}
        />
        <button type="submit">{t("nav.search")}</button>
      </form>
      {submitted && !value && <Loading error={error} />}
      {value && (
        <>
          {value.artists.length > 0 && (
            <>
              <h3>{t("nav.artists")}</h3>
              <ul className="list">
                {value.artists.map((artist) => (
                  <li key={artist.id}>
                    <Link
                      to="/artists/$artistId"
                      params={{ artistId: artist.id }}
                    >
                      {artist.name}
                    </Link>
                  </li>
                ))}
              </ul>
            </>
          )}
          {value.albums.length > 0 && (
            <>
              <h3>{t("nav.albums")}</h3>
              <AlbumGrid albums={value.albums} />
            </>
          )}
          {value.songs.length > 0 && (
            <>
              <h3>{t("search.tracks")}</h3>
              <SongTable songs={value.songs} />
            </>
          )}
          {value.artists.length === 0 &&
            value.albums.length === 0 &&
            value.songs.length === 0 && (
              <p className="muted">{t("search.empty")}</p>
            )}
        </>
      )}
    </section>
  );
}
