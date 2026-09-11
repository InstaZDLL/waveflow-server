import { type ReactNode, useEffect, useState } from "react";

import { canvasUrl } from "./api";
import type { TranslationKey } from "./i18n";

/**
 * A refused placement or removal, in words. The statuses are the ones the
 * canvas route documents; anything else, including no answer at all, is the
 * server failing rather than refusing.
 */
export function canvasRefusal(status: number): TranslationKey {
  switch (status) {
    case 404:
      return "canvas.notAllowed";
    case 409:
      return "canvas.quota";
    case 413:
      return "canvas.tooLarge";
    case 422:
      return "canvas.refused";
    default:
      return "canvas.error";
  }
}

const REDUCED_MOTION = "(prefers-reduced-motion: reduce)";

function reducedMotionQuery(): MediaQueryList | null {
  return typeof window.matchMedia === "function"
    ? window.matchMedia(REDUCED_MOTION)
    : null;
}

/** Whether the viewer has asked for less motion. A loop is nothing but motion. */
export function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(
    () => reducedMotionQuery()?.matches ?? false,
  );
  useEffect(() => {
    const query = reducedMotionQuery();
    if (!query) return;
    const onChange = () => setReduced(query.matches);
    onChange();
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return reduced;
}

/**
 * A track's canvas laid over its cover, on the page that shows what is playing.
 *
 * Rendered the way the desktop renders it: muted, looping, hidden from
 * assistive technology. The cover stays underneath and keeps the accessible
 * name, which is why the video is laid over it rather than put in its place,
 * and it shows through until the loop can play — or for good, when the browser
 * cannot play it. Nothing is asked of the server when motion is reduced.
 *
 * The ticket is the question: a track without a canvas answers 404 to it, so
 * there is no separate request to find out whether there is one.
 */
export function CanvasStage({
  trackId,
  children,
}: {
  trackId: string;
  children: ReactNode;
}) {
  const reduced = usePrefersReducedMotion();
  const [source, setSource] = useState<{
    trackId: string;
    url: string;
  } | null>(null);

  useEffect(() => {
    if (reduced) return;
    let cancelled = false;
    void canvasUrl(trackId)
      .then((ticket) => {
        if (!cancelled) setSource(ticket ? { trackId, url: ticket.url } : null);
      })
      // A canvas is decoration. Not finding out whether there is one leaves
      // the cover, which is not a failure the listener has to read about.
      .catch(() => {
        if (!cancelled) setSource(null);
      });
    return () => {
      cancelled = true;
    };
  }, [trackId, reduced]);

  // Compared by track, so the previous track's loop is never shown over the
  // next one's cover while its own answer is still out.
  const url = !reduced && source?.trackId === trackId ? source.url : null;
  return (
    <div className="canvas-stage">
      {children}
      {url ? <CanvasVideo key={url} url={url} /> : null}
    </div>
  );
}

function CanvasVideo({ url }: { url: string }) {
  const [ready, setReady] = useState(false);
  const [failed, setFailed] = useState(false);
  if (failed) return null;
  return (
    <video
      className={ready ? "canvas-video ready" : "canvas-video"}
      src={url}
      autoPlay
      loop
      muted
      playsInline
      disablePictureInPicture
      // Out of the tab order as well as the accessibility tree: there are no
      // controls to reach, and a hidden element must not take focus.
      tabIndex={-1}
      aria-hidden="true"
      onCanPlay={() => setReady(true)}
      onError={() => setFailed(true)}
    />
  );
}
