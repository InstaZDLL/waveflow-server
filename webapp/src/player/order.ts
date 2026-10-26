// The queue's reading order: shuffle, and where next and previous go.

export type RepeatMode = "off" | "all" | "one";

/**
 * The order playback walks, as positions into the queue.
 *
 * Shuffling is a *reading order* and not a rearrangement of the queue: the
 * queue page keeps showing what was queued, in the order it was queued, and
 * turning shuffle off resumes the line where it left it. Shuffling the array
 * itself would lose that, and would make "add to queue" land somewhere the
 * listener did not choose.
 *
 * The current position leads, so enabling shuffle never interrupts what is
 * playing.
 */
export function shuffledOrder(
  length: number,
  current: number,
  random: () => number = Math.random,
): number[] {
  const rest = Array.from({ length }, (_, index) => index).filter(
    (index) => index !== current,
  );
  for (let i = rest.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [rest[i], rest[j]] = [rest[j] as number, rest[i] as number];
  }
  return current >= 0 && current < length ? [current, ...rest] : rest;
}

/**
 * Where playback goes after `current`, or `null` to stop there.
 *
 * `automatic` says whether the track ended by itself or the listener pressed
 * next, and it is the whole of what separates them: `repeat: "one"` replays a
 * track that ended, but pressing next under it still moves on. A next button
 * that did nothing would look broken.
 */
export function advance(
  order: number[],
  current: number,
  repeat: RepeatMode,
  automatic: boolean,
): number | null {
  if (repeat === "one" && automatic) return current;
  const at = order.indexOf(current);
  if (at === -1) return null;
  const following = order[at + 1];
  if (following !== undefined) return following;
  return repeat === "all" ? (order[0] ?? null) : null;
}

/**
 * Whether the queue an order was drawn for is still the queue being played,
 * and where its entries went.
 *
 * `false` means a different queue and a fresh draw. Anything else is the
 * mapping `followQueue` needs — `moved[before]` is where that entry sits now,
 * or -1 if it is gone. An append and a truncation are the identity mapping,
 * with the entries that went away pointing past the end.
 *
 * Compared by entry identity, not by length or by prefix. Length alone let one
 * five-track album inherit another's draw; a prefix test refused every removal,
 * including one from the middle, and reshuffled the rest of the listening under
 * the listener.
 */
export function continuationOf<T>(
  drawn: { entries: readonly T[]; shuffle: boolean } | null,
  entries: readonly T[],
  shuffle: boolean,
): false | number[] {
  if (drawn === null || drawn.shuffle !== shuffle) return false;
  const moved = drawn.entries.map((entry) => entries.indexOf(entry));
  return moved.some((position) => position >= 0) ? moved : false;
}

/**
 * Re-points a reading order at a queue whose entries have moved.
 *
 * `moved[before]` is where the entry that was at `before` sits now, or -1 if it
 * is gone. Removing a track from the middle shifts everything after it down by
 * one, so an order made of positions stops meaning what it meant — following
 * the numbers alone would keep playing, but not the songs that were chosen.
 */
export function followQueue(
  previous: number[],
  moved: readonly number[],
  length: number,
): number[] {
  const kept: number[] = [];
  const seen = new Set<number>();
  for (const before of previous) {
    const now = moved[before];
    if (now === undefined || now < 0 || now >= length || seen.has(now))
      continue;
    kept.push(now);
    seen.add(now);
  }
  for (let position = 0; position < length; position++) {
    if (!seen.has(position)) kept.push(position);
  }
  return kept;
}

/**
 * The reading order a queue should have, given the one it had before.
 *
 * `continues` says whether this is the same listening session rather than a
 * different queue altogether. It cannot be inferred from the length: replacing
 * a five-track album with another five-track album left the previous draw in
 * place, and a starting position that fell at the end of that draw stopped
 * playback after one track.
 *
 * `continues` is `false` for a different queue, or the mapping saying where the
 * previous queue's entries went. There is no separate "kept their positions"
 * case: an append and a truncation are the identity mapping, and `followQueue`
 * reads them the same way it reads a removal from the middle.
 */
export function orderForQueue(
  length: number,
  at: number,
  shuffle: boolean,
  previous: number[],
  continues: false | readonly number[],
  random: () => number = Math.random,
): number[] {
  if (!shuffle) return Array.from({ length }, (_, position) => position);
  if (continues === false || previous.length === 0) {
    return shuffledOrder(length, at, random);
  }
  return followQueue(previous, continues, length);
}

/** Where "previous" goes. Wraps only when the whole queue repeats. */
export function retreat(
  order: number[],
  current: number,
  repeat: RepeatMode,
): number | null {
  const at = order.indexOf(current);
  if (at === -1) return null;
  if (at > 0) return order[at - 1] ?? null;
  return repeat === "all" ? (order[order.length - 1] ?? null) : null;
}
