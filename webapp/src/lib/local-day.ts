/**
 * The listener's day, as the questions that depend on it name it.
 *
 * Mixes are drawn once per local day and statistics are cut into local days
 * (RFC-024), so the day and the UTC offset belong in those queries' keys: a
 * tab left open past midnight, or carried into another timezone, must ask
 * again rather than keep yesterday's answer. A key that never changes is never
 * refetched by `staleTime` alone.
 */

import { utcOffsetMinutes } from "../api/discovery";

/** `YYYY-MM-DD@offset`, the local date and its offset from UTC in minutes. */
export function localDayKey(now: Date = new Date()): string {
  const date = [
    now.getFullYear(),
    String(now.getMonth() + 1).padStart(2, "0"),
    String(now.getDate()).padStart(2, "0"),
  ].join("-");
  return `${date}@${utcOffsetMinutes(now)}`;
}

/** How long until the next local midnight, in milliseconds. */
export function msUntilNextLocalMidnight(now: Date = new Date()): number {
  const midnight = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() + 1,
  );
  return Math.max(0, midnight.getTime() - now.getTime());
}

/** How often the offset is looked at again while the page stays open. */
export const OFFSET_CHECK_MS = 60_000;

/**
 * Calls `onChange` with the new key whenever the local day or the UTC offset
 * changes, and returns what stops watching.
 *
 * Three triggers, because a midnight timer alone is not enough:
 * - a timer set for the next midnight, re-armed each time it fires; a second
 *   past midnight is waited, so a timer firing a hair early does not read the
 *   old date;
 * - a check every minute, for the offset: a daylight-saving change happens in
 *   the small hours, not at midnight, and a page left open through it must
 *   not go on cutting days at the old hour;
 * - a check whenever the page becomes visible again: a laptop asleep across
 *   midnight runs no timer, and a move to another timezone moves no clock.
 */
export function watchLocalDay(onChange: (key: string) => void): () => void {
  let current = localDayKey();
  let timer: ReturnType<typeof setTimeout> | undefined;

  const check = () => {
    const next = localDayKey();
    if (next !== current) {
      current = next;
      onChange(next);
    }
  };
  const arm = () => {
    timer = setTimeout(() => {
      check();
      arm();
    }, msUntilNextLocalMidnight() + 1_000);
  };
  const onVisible = () => {
    if (document.visibilityState === "visible") check();
  };

  arm();
  const every = setInterval(check, OFFSET_CHECK_MS);
  document.addEventListener("visibilitychange", onVisible);
  return () => {
    if (timer !== undefined) clearTimeout(timer);
    clearInterval(every);
    document.removeEventListener("visibilitychange", onVisible);
  };
}
