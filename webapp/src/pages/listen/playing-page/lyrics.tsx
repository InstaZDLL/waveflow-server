// The words: the sheet, the sung line, the strip and the lyrics view.

import { useQuery } from "@tanstack/react-query";
import { type RefObject, useLayoutEffect, useMemo, useRef } from "react";

import {
  formatDuration,
  type LyricsLine,
  type Song,
  type StructuredLyrics,
} from "../../api";
import { lyricsQuery } from "../../api/queries";
import { usePrefersReducedMotion } from "../../components/canvas";
import { EmptyState } from "../../components/page-shell";
import { usePlayer, usePlayerProgress } from "../../player/player";
import { useI18n } from "../../state/i18n";

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

/** A track's sheet as `useLyricSheet` reads it: the words, the sung line. */
export type LyricSheet = {
  sheet: StructuredLyrics | undefined;
  active: number;
  keys: string[];
  state: "waiting" | "sheet" | "none";
};

/**
 * The sheet a track travels with, and where in it the playing head is.
 *
 * Shared by the strip and the dedicated view below, so the two cannot disagree
 * about which line is being sung — or about how a refrain is keyed, which is
 * the part that is easy to get subtly different twice.
 */
export function useLyricSheet(trackId: string): LyricSheet {
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
 * The words in the detail view: a column of their own beside the track on a
 * wide screen, under its names on a narrow one.
 *
 * Smaller than the lyrics view and inside a box of its own, but reading the
 * same way: the sung line is held in view and, where the sheet is synced, a
 * line seeks to itself. The sheet is read by `DetailView` and handed down,
 * because that view lays itself out by whether there is one.
 */
export function LyricsStrip({ lyrics }: { lyrics: LyricSheet }) {
  const { t } = useI18n();
  const { sheet, active, keys, state } = lyrics;
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
export function LyricsView({ song }: { song: Song }) {
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
