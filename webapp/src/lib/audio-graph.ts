/**
 * The order the ReplayGain graph is wired in (RFC-022 decision 1).
 *
 * No React here. The order is the whole point, and it is easier to pin with a
 * fake context than through a player: once `createMediaElementSource` has
 * claimed an element, the element's sound reaches the speakers only through
 * the context. A context left suspended — created outside a gesture, or by a
 * browser that does not count the gesture — would then play the music as
 * silence while the element reports it playing. So the context is resumed
 * first, and `wire` (which claims the element) runs only once it runs;
 * otherwise the candidate is closed and the element keeps its direct output.
 */

/** What this needs of an `AudioContext`, so a test can supply one. */
export type ResumableContext = {
  readonly state: string;
  resume(): Promise<void>;
  close(): Promise<void>;
};

export async function connectWhenRunning<T>(
  context: ResumableContext,
  wire: () => T,
): Promise<T | null> {
  const abandon = () => {
    void context.close().catch(() => undefined);
    return null;
  };
  try {
    if (context.state !== "running") await context.resume();
    if (context.state !== "running") return abandon();
    return wire();
  } catch {
    // Whatever failed — the resume, the gain, the source — the graph is not
    // usable; a closed context frees the audio device rather than holding it
    // for nothing.
    return abandon();
  }
}

/**
 * Whether a graph's context should be asked to run again: it has stopped
 * (suspended, or `interrupted` on Safari) while the element is playing.
 */
export function needsWaking(state: string, elementPaused: boolean): boolean {
  return !elementPaused && (state === "suspended" || state === "interrupted");
}
