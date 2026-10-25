import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import {
  type RefObject,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import {
  deleteBookmark,
  formatDuration,
  type LyricsLine,
  type Song,
  type StructuredLyrics,
  setBookmark,
} from "../api";
import { bookmarksQuery, lyricsQuery } from "../api/queries";
import { Artwork } from "../components/artwork";
import { CanvasStage, usePrefersReducedMotion } from "../components/canvas";
import { FileQuality } from "../components/file-quality";
import { Icon } from "../components/icons";
import { EmptyState, PageHeader, useReload } from "../components/page-shell";
import { usePlayer, usePlayerProgress } from "../player/player";
import { type TranslationKey, useI18n } from "../state/i18n";

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

/**
 * The line a synced lyric sheet is on at `seconds`. Lines carry their own start
 * in milliseconds and arrive in order, so this is the last one already begun;
 * `-1` before the first. An unsynced sheet has no starts and never highlights.
 */
export function currentLyricLine(lines: LyricsLine[], seconds: number): number {
  let current = -1;
  for (const [index, line] of lines.entries()) {
    if (line.start === undefined || line.start > seconds * 1000) break;
    current = index;
  }
  return current;
}

/**
 * What the lyrics panel has to say, from what the query holds.
 *
 * **Three states and not two**: a sheet that has not arrived is not a track
 * without words, and saying "no lyrics" while the request is out is a claim
 * about the track.
 *
 * **And words already held beat a failed attempt.** The cache serves an answer
 * for thirty seconds, so coming back to this view later revalidates — and a
 * revalidation that failed used to blank the sheet and print "no lyrics travel
 * with this file", which is a false statement about a track whose words were on
 * the screen a moment earlier. A failure decides only when there is nothing to
 * show.
 *
 * That ordering is also why the scroll effect below does not watch this value:
 * the column it scrolls can no longer be torn down and rebuilt while a sheet is
 * held, so the only remount left is one where `active` moves with it.
 */
export function lyricSheetState(answer: {
  /** The query holds an answer, even a stale one. */
  answered: boolean;
  /** The last attempt failed. */
  failed: boolean;
  /** How many lines the first sheet carries. */
  lines: number;
}): "waiting" | "sheet" | "none" {
  if (answer.answered) return answer.lines > 0 ? "sheet" : "none";
  return answer.failed ? "none" : "waiting";
}

/**
 * What a line with no words shows: a non-breaking space, not a plain one.
 *
 * A plain space is collapsible, so the line box around it has no height — a
 * blank timed line became a button zero pixels tall, which the keyboard still
 * reached and nobody could see, focus ring included. A non-breaking space
 * holds one line of height, so the pause keeps its place in the sheet and its
 * button a box. It is still whitespace to axe, which trims it before asking
 * whether the name contains what is on screen.
 */
const BLANK_LINE = "\u00a0";

/**
 * The sheet a track travels with, and where in it the playing head is.
 *
 * Shared by the strip and the dedicated view below, so the two cannot disagree
 * about which line is being sung — or about how a refrain is keyed, which is
 * the part that is easy to get subtly different twice.
 */
function useLyricSheet(trackId: string) {
  const progress = usePlayerProgress();
  const { data: value, error } = useQuery(lyricsQuery(trackId));
  const sheet: StructuredLyrics | undefined = value?.structuredLyrics[0];
  const active = sheet?.synced
    ? currentLyricLine(sheet.line, progress.position)
    : -1;
  // A refrain repeats word for word and an unsynced sheet has no start to key
  // on, so the line's text alone is not unique. Numbering the repetitions is
  // what the queue does with the same problem.
  const keys = useMemo(() => {
    const seen = new Map<string, number>();
    return (sheet?.line ?? []).map((line) => {
      const occurrence = seen.get(line.value) ?? 0;
      seen.set(line.value, occurrence + 1);
      return `${line.value}-${occurrence}`;
    });
  }, [sheet]);
  return {
    sheet,
    active,
    keys,
    state: lyricSheetState({
      answered: value !== undefined,
      failed: error !== null,
      lines: sheet?.line.length ?? 0,
    }),
  };
}

/**
 * Holds the sung line in the middle of `column`, and only `column`.
 *
 * Shared by the strip beside the cover and the lyrics view, so the two follow
 * the words the same way: the strip used to bold the sung line and leave it to
 * scroll out of its own box, which marked a line nobody could see.
 */
function useFollowSungLine(
  column: RefObject<HTMLOListElement | null>,
  sheet: StructuredLyrics | undefined,
  active: number,
) {
  const reduced = usePrefersReducedMotion();
  // **Before paint, and only when there is something to do.** `useEffect`
  // would let the new line show at the bottom of the column for a frame before
  // the scroll caught up. And a progress tick that stays on the same line
  // changes none of the dependencies, so it costs nothing.
  useLayoutEffect(() => {
    // `sheet` is read here and not only named below: there is no column
    // without one, and a dependency the body never touches is a decorative
    // dependency — which is exactly what the linter says about it.
    if (!sheet || active < 0) return;
    const list = column.current;
    const line = list?.children[active];
    if (!list || !line) return;
    // **The column and nothing else.** `scrollIntoView` scrolls every
    // scrollable ancestor it has, the window included, so each new sung line
    // would drag the whole page back to the words — and somebody reading the
    // bookmarks underneath would be hauled upwards every few seconds.
    //
    // Measured from boxes rather than from `offsetTop`, which is relative to
    // whichever ancestor happens to be positioned: two rectangles need no
    // assumption about where the list sits in the document.
    const lineBox = line.getBoundingClientRect();
    const listBox = list.getBoundingClientRect();
    list.scrollTo({
      top:
        list.scrollTop +
        (lineBox.top - listBox.top) -
        (listBox.height - lineBox.height) / 2,
      // A sheet that jumps rather than glides, for whoever asked for less
      // motion. The following itself is not decoration — it is how the words
      // are read — so it is the gliding that goes, not the following.
      behavior: reduced ? "auto" : "smooth",
    });
    // `sheet` as well as `active`, for the track change where the two
    // sheets agree on the index. Moving to the next track early in it leaves
    // `active` at 0, which it already was — so without this the column keeps
    // the previous track's scroll and the opening line of the new one is above
    // the fold. The object is react-query's and stable between renders, so
    // naming it costs no extra run.
  }, [active, column, reduced, sheet]);
}

/**
 * A sheet's lines, the sung one marked.
 *
 * Where the sheet carries timings, each line is a **button that seeks to it**,
 * in the strip as in the lyrics view. A sheet without timings gets no buttons
 * at all: rendering an unsynced line as something pressable would promise a
 * seek the sheet cannot compute.
 */
function LyricLines({
  sheet,
  active,
  keys,
  column,
}: {
  sheet: StructuredLyrics;
  active: number;
  keys: string[];
  column: RefObject<HTMLOListElement | null>;
}) {
  const { t } = useI18n();
  const player = usePlayer();
  return (
    <ol ref={column} aria-label={t("lyrics.label")}>
      {sheet.line.map((line, index) => {
        const at = line.start;
        const on = index === active;
        return (
          <li
            key={keys[index]}
            className={on ? "on" : undefined}
            aria-current={on ? "true" : undefined}
          >
            {sheet.synced && at !== undefined ? (
              <button
                type="button"
                className="link"
                onClick={() => player.seek(at / 1000)}
                // The words first, then what pressing does. The name must
                // contain the text on screen (WCAG 2.5.3), or someone saying
                // "click the lamps come on" to voice control reaches
                // nothing; the time after it is what a bare line leaves
                // unsaid. A blank line has no words to repeat.
                aria-label={
                  line.value
                    ? t("lyrics.seekLine", {
                        line: line.value,
                        when: formatDuration(at),
                      })
                    : t("lyrics.seekTo", { when: formatDuration(at) })
                }
              >
                {line.value || BLANK_LINE}
              </button>
            ) : (
              line.value || BLANK_LINE
            )}
          </li>
        );
      })}
    </ol>
  );
}

/**
 * The words beside the cover, in the detail view.
 *
 * Smaller than the lyrics view and inside a box of its own, but reading the
 * same way: the sung line is held in view and, where the sheet is synced, a
 * line seeks to itself.
 */
function LyricsStrip({ trackId }: { trackId: string }) {
  const { t } = useI18n();
  const { sheet, active, keys, state } = useLyricSheet(trackId);
  const column = useRef<HTMLOListElement | null>(null);
  useFollowSungLine(column, sheet, active);

  if (state === "waiting")
    return <p className="muted">{t("common.loading")}</p>;
  if (state === "none" || !sheet)
    return <p className="muted">{t("lyrics.none")}</p>;
  return (
    <div className="lyrics">
      <h3>
        {sheet.displayTitle}
        {sheet.displayArtist ? <small>{sheet.displayArtist}</small> : null}
      </h3>
      <LyricLines sheet={sheet} active={active} keys={keys} column={column} />
      {sheet.source === "lrclib" ? (
        <p className="muted lyrics-credit">
          <small>{t("lyrics.fromLrclib")}</small>
        </p>
      ) : null}
    </div>
  );
}

/**
 * The words as the subject of the screen: one wide column, centred, with the
 * names and the hint above it. It follows and seeks as the strip does; what it
 * adds is the room.
 */
function LyricsView({ song }: { song: Song }) {
  const { t } = useI18n();
  const { sheet, active, keys, state } = useLyricSheet(song.id);
  const column = useRef<HTMLOListElement | null>(null);
  useFollowSungLine(column, sheet, active);

  // Inside the view's own frame, like the empty state below it: a column that
  // appears out of nowhere once the words land is a jump the wrapper absorbs.
  if (state === "waiting")
    return (
      <div className="lyrics-view">
        <p className="muted">{t("common.loading")}</p>
      </div>
    );
  if (state === "none" || !sheet)
    return (
      <div className="lyrics-view">
        <EmptyState message={t("lyrics.none")} />
      </div>
    );

  return (
    <div className="lyrics-view">
      <header>
        <h2>{sheet.displayTitle}</h2>
        <p className="muted">
          {sheet.displayArtist ?? song.artist ?? t("common.unknownArtist")}
        </p>
        {sheet.synced ? (
          <p className="muted">
            <small>{t("lyrics.seekHint")}</small>
          </p>
        ) : null}
        {sheet.source === "lrclib" ? (
          <p className="muted lyrics-credit">
            <small>{t("lyrics.fromLrclib")}</small>
          </p>
        ) : null}
      </header>
      <LyricLines sheet={sheet} active={active} keys={keys} column={column} />
    </div>
  );
}

/**
 * The artwork, and the canvas where there is one, over everything else.
 *
 * A native `<dialog>` opened with `showModal`, which is where the focus trap,
 * the Escape key and the inertness of the page behind come from — the same
 * reasoning the navigation sheet records, and the same reason not to hand-roll
 * an overlay.
 *
 * **It carries transport controls because it covers the player bar.** A view
 * that fills the screen with a cover and leaves no way to pause is a trap
 * rather than a mode. The keyboard shortcuts keep working regardless: the
 * player listens on the window, which a modal dialog does not intercept.
 */
function ImmersiveView({ song }: { song: Song }) {
  const { t } = useI18n();
  const navigate = useNavigate();
  const player = usePlayer();
  const progress = usePlayerProgress();
  const frame = useRef<HTMLDialogElement | null>(null);
  // Set while this component is going away, so the close that unmounting
  // causes is told apart from the close a person asked for. Without it,
  // leaving the screen by any other route would navigate back onto it.
  const leaving = useRef(false);

  useLayoutEffect(() => {
    const dialog = frame.current;
    if (!dialog) return;
    // **Cleared on the way in, because an effect has to be able to run
    // twice.** `StrictMode` mounts, cleans up and mounts again on the same
    // instance, so the ref survives into the second run — and left at `true`
    // it would swallow a close the person asked for, leaving the view shut and
    // the address still naming it. In development only, which is exactly why
    // no test saw it: the end-to-end suite runs the production build, where
    // the effect runs once.
    leaving.current = false;
    dialog.showModal();
    return () => {
      leaving.current = true;
      if (dialog.open) dialog.close();
    };
  }, []);

  return (
    // **One closer.** The button closes the dialog and nothing else; the
    // navigation hangs off `onClose`, which Escape and the backdrop reach too.
    // A button that both closed and navigated would be a second closer, and a
    // second closer is a second thing to get out of step.
    // biome-ignore lint/a11y/useKeyWithClickEvents: dismissing by keyboard is Escape, which <dialog> handles natively
    <dialog
      ref={frame}
      className="immersive"
      aria-label={t("playing.immersiveTitle")}
      onClose={() => {
        // `close()` queues the event rather than firing it, so the close from
        // that first cleanup can arrive **after** the second `showModal` has
        // reopened the dialog. Navigating then would shut a view the person is
        // looking at, on a close that is already spent.
        if (frame.current?.open) return;
        // **`replace`, or Back reopens what was just closed.** Pushing
        // would leave `/playing → ?view=immersive → /playing` in the
        // history, so the next Back press would land on the immersive entry
        // and open the view again — the opposite of the reason the mode is in
        // the location at all. Replacing swaps the immersive entry for the
        // detail one, and Back goes past both.
        if (!leaving.current)
          void navigate({ to: "/playing", search: {}, replace: true });
      }}
      onClick={(event) => {
        if (event.target === frame.current) frame.current?.close();
      }}
    >
      <CanvasStage trackId={song.id} className="immersive-stage">
        <Artwork
          artworkId={song.artwork_hash}
          title={song.title}
          className="cover immersive-cover"
        />
      </CanvasStage>
      <div className="immersive-detail">
        <span className="eyebrow">{song.album ?? t("common.album")}</span>
        <h2>{song.title}</h2>
        <p className="muted">{song.artist ?? t("common.unknownArtist")}</p>
        <p className="muted">
          <small>
            {formatDuration(progress.position * 1000)}
            {" / "}
            {formatDuration(progress.duration * 1000)}
          </small>
        </p>
        <div className="immersive-transport">
          <button
            type="button"
            aria-label={t("player.previous")}
            onClick={() => player.previous()}
          >
            <Icon name="previous" />
          </button>
          <button
            type="button"
            className="primary"
            aria-label={t(player.playing ? "player.pause" : "player.play")}
            onClick={() => player.toggle()}
          >
            <Icon name={player.playing ? "pause" : "play"} />
          </button>
          <button
            type="button"
            aria-label={t("player.next")}
            onClick={() => player.next()}
          >
            <Icon name="next" />
          </button>
        </div>
      </div>
      <button
        type="button"
        className="immersive-close"
        aria-label={t("playing.leaveImmersive")}
        onClick={() => frame.current?.close()}
      >
        <Icon name="close" />
      </button>
    </dialog>
  );
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
            <div className="playing">
              <CanvasStage trackId={current.id}>
                <Artwork
                  artworkId={current.artwork_hash}
                  title={current.title}
                  className="cover large"
                />
              </CanvasStage>
              <div className="playing-detail">
                <span className="eyebrow">
                  {current.album ?? t("common.album")}
                </span>
                <h2>{current.title}</h2>
                <p className="muted">
                  {current.artist ?? t("common.unknownArtist")}
                </p>
                {/* Under the names and above the words: what the file is belongs
                    with the track and not with the lyrics, and a listener who
                    came here to read along should not have to pass it twice. */}
                <FileQuality song={current} />
                <LyricsStrip trackId={current.id} />
              </div>
            </div>
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
