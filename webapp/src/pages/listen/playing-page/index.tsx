// The playing screen: the view in the address, its switcher, the bookmarks.

import { useQuery } from "@tanstack/react-query";
import { Link, useSearch } from "@tanstack/react-router";
import { useState } from "react";

import {
  deleteBookmark,
  formatDuration,
  type Song,
  setBookmark,
} from "../../../api";
import { bookmarksQuery } from "../../../api/queries";
import { Artwork } from "../../../components/artwork/artwork";
import {
  EmptyState,
  PageHeader,
  useReload,
} from "../../../components/layout/page-shell";
import { usePlayer, usePlayerProgress } from "../../../player/player";
import { type TranslationKey, useI18n } from "../../../state/i18n";
import { DetailView } from "./detail-view";
import { ImmersiveView } from "./immersive";
import { LyricsView } from "./lyrics";

export { detailSplit } from "./detail-view";
export { currentLyricLine, lyricSheetState } from "./lyrics";

/**
 * What the playing screen is showing.
 *
 * Three ways to look at one track, and `detail` is the one that was there
 * before: the cover beside the names, with the words as a strip under them.
 * `lyrics` makes those words the subject instead of a column inside something
 * else, and `immersive` gives the artwork — and the canvas, where the track
 * carries one — the whole viewport.
 */
export type PlayingView = "detail" | "lyrics" | "immersive";

/**
 * The view a location asks for, or `detail` for anything it does not.
 *
 * **In the URL rather than in a `useState`**, and the reason is the Back
 * button. On a phone, Back is what everybody presses to leave the thing they
 * are in; held in component state, the immersive view would swallow that press
 * and Back would leave the playing screen altogether. A location also makes
 * "read along to this" a link somebody can keep.
 *
 * Only the two modes that are not the default are spelled, so an ordinary
 * visit stays `/playing` and nobody reads `?view=detail`. Anything else — a
 * mode this build does not know, a number, a hostile string — reads as
 * `detail`, because an unknown mode is not a mode and the screen still has to
 * show something.
 */
export function playingView(search: unknown): PlayingView {
  const asked =
    typeof search === "object" && search !== null
      ? (search as { view?: unknown }).view
      : undefined;
  return asked === "lyrics" || asked === "immersive" ? asked : "detail";
}

/** The three ways to look at the track, as links, because the view is a place. */
function ViewSwitcher() {
  const { t } = useI18n();
  const entries: Array<{
    view: PlayingView;
    search: Record<string, string>;
    label: TranslationKey;
  }> = [
    { view: "detail", search: {}, label: "playing.viewDetail" },
    { view: "lyrics", search: { view: "lyrics" }, label: "playing.viewLyrics" },
    {
      view: "immersive",
      search: { view: "immersive" },
      label: "playing.viewImmersive",
    },
  ];
  return (
    <nav className="view-switcher" aria-label={t("playing.viewLabel")}>
      {entries.map((entry) => (
        <Link
          key={entry.view}
          to="/playing"
          search={entry.search}
          // **`exact`, and the default would be wrong here.** All three
          // links point at one path, and the router counts a link whose
          // search is `{}` as matching every search there is — so `Detail`
          // was marked current on `?view=lyrics` as well and the switcher
          // claimed to be in two views at once. Measured rather than
          // reasoned about: the reading was `['Detail', 'Lyrics']`, and it
          // is `['Lyrics']` with this.
          //
          // The router marks it and this component does not, although it
          // knows which view is showing. Two things setting one attribute is
          // how they start disagreeing, and the one that survives is whichever
          // the library applies last.
          activeOptions={{ exact: true, includeSearch: true }}
        >
          {t(entry.label)}
        </Link>
      ))}
    </nav>
  );
}

export function PlayingPage() {
  const player = usePlayer();
  const progress = usePlayerProgress();
  const { t } = useI18n();
  const reload = useReload(bookmarksQuery().queryKey);
  const [busy, setBusy] = useState(false);
  const { data: bookmarks } = useQuery(bookmarksQuery());
  const current = player.current;
  const view = playingView(useSearch({ strict: false }));

  /**
   * One bookmark per track, replaced rather than added — so this button reads
   * "save" whether or not the track already has one, and saving again simply
   * moves it to where the head is now.
   */
  async function saveHere(song: Song) {
    setBusy(true);
    try {
      await setBookmark(song.id, progress.position * 1000);
      reload();
    } catch {
      // Nothing is lost: the head has not moved and the old bookmark stands.
    } finally {
      setBusy(false);
    }
  }

  async function forget(trackId: string) {
    try {
      await deleteBookmark(trackId);
      reload();
    } catch {
      // Same: the list simply does not change.
    }
  }

  return (
    <section>
      <PageHeader
        kicker={t("nav.group.listen")}
        title={t("nav.playing")}
        detail={current ? undefined : t("playing.idle")}
      >
        {current ? (
          <>
            <ViewSwitcher />
            <button
              type="button"
              disabled={busy}
              onClick={() => void saveHere(current)}
            >
              {t("bookmarks.save")}
            </button>
          </>
        ) : null}
      </PageHeader>

      {current ? (
        <>
          {/* The immersive view is laid over the screen rather than put in
              its place, so leaving it returns to a page that was never torn
              down — and the detail stays beneath it for the frame the dialog
              takes to open. */}
          {view === "immersive" ? <ImmersiveView song={current} /> : null}
          {view === "lyrics" ? (
            <LyricsView song={current} />
          ) : (
            <DetailView song={current} />
          )}
        </>
      ) : (
        <EmptyState message={t("playing.empty")} />
      )}

      {bookmarks?.length ? (
        <article className="playing-bookmarks">
          <header className="playing-bookmarks-header">
            <div>
              <h3 className="playing-bookmarks-title">
                {t("bookmarks.title")}
              </h3>
              <p className="playing-bookmarks-detail muted">
                {t("bookmarks.detail")}
              </p>
            </div>
          </header>
          <ul className="bookmark-list">
            {bookmarks.map((bookmark) => (
              <li key={bookmark.song.id}>
                <button
                  type="button"
                  className="bookmark-track"
                  onClick={() => player.play([bookmark.song], 0)}
                >
                  <Artwork
                    artworkId={bookmark.song.artwork_hash}
                    title={bookmark.song.title}
                    className="bookmark-cover"
                  />
                  <span className="bookmark-copy">
                    <strong className="bookmark-title">
                      {bookmark.song.title}
                    </strong>
                    <small className="bookmark-artist muted">
                      {bookmark.song.artist ?? t("common.unknownArtist")}
                    </small>
                  </span>
                </button>
                <span className="bookmark-position muted">
                  {formatDuration(bookmark.position_ms)}
                </span>
                <button
                  type="button"
                  className="bookmark-forget"
                  onClick={() => void forget(bookmark.song.id)}
                  aria-label={`${t("bookmarks.forget")}: ${bookmark.song.title}`}
                >
                  {t("bookmarks.forget")}
                </button>
              </li>
            ))}
          </ul>
        </article>
      ) : null}
    </section>
  );
}
