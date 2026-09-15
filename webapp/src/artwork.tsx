import { useCallback, useEffect, useState } from "react";

import { artworkUrl, cachedArtworkUrl } from "./api";

type ArtworkProps = {
  artworkId: string | null;
  title: string;
  className?: string;
};

/**
 * How far ahead of the viewport a cover starts loading.
 *
 * Enough that scrolling at a normal speed meets an image already there, and
 * little enough that a long list still asks for a fraction of itself. One
 * row's height, roughly, on every layout this client has.
 */
const AHEAD = "300px";

/**
 * Whether this browser can tell us what is on screen.
 *
 * Where it cannot — an old engine, or a test environment that stubs the DOM —
 * every cover loads at once, which is exactly what this component did before.
 * Degrading to the previous behaviour is the only safe direction: the other
 * one leaves a page of empty squares that nothing will ever fill.
 */
const observable = typeof IntersectionObserver !== "undefined";

/**
 * A cover, fetched when it is about to be seen.
 *
 * `/api/v2/artwork/{hash}` requires an `Authorization` header, so `<img src>`
 * cannot fetch it and every thumbnail costs this client a `fetch` and an
 * object URL of its own. Rendering an album page used to start all of them on
 * mount — 127 requests in half a second, measured on a library of 164 tracks,
 * against a connection limit of six. The covers below the fold were queued
 * ahead of the ones being looked at, so the visible page filled last.
 *
 * Asking only for what is on screen does not make the requests cheaper; it
 * stops making the ones nobody asked for. A ticket like the stream's would
 * make `<img src>` work and let the HTTP cache serve between sessions, which
 * is the deeper fix and is not this one.
 */
export function Artwork({ artworkId, title, className = "" }: ArtworkProps) {
  // Read straight out of the cache, during the render rather than after it.
  // A cover whose bytes are already held has nothing to wait for, and starting
  // at `null` put the grey placeholder on screen for a frame every time this
  // remounted — which is every navigation back to a grid already visited.
  const held = cachedArtworkUrl(artworkId);
  const [src, setSrc] = useState<string | null>(held);
  // And nothing to observe for, either: asking whether it is on screen before
  // showing what is already in memory is a wait with no question behind it.
  const [wanted, setWanted] = useState(!observable || held !== null);

  // A callback ref rather than a `useRef`: this component renders a different
  // element depending on whether the image has arrived, so there is no one node
  // to observe for its lifetime. React hands the new node here on every swap.
  const observe = useCallback(
    (node: HTMLElement | null) => {
      if (!node || !observable || wanted) return;
      const observer = new IntersectionObserver(
        (entries) => {
          if (entries.some((entry) => entry.isIntersecting)) {
            setWanted(true);
            observer.disconnect();
          }
        },
        { rootMargin: AHEAD },
      );
      observer.observe(node);
      return () => observer.disconnect();
    },
    [wanted],
  );

  // A new id is a new question, asked again when that cover is next on screen.
  // Adjusted while rendering rather than in an effect: an effect would paint
  // the previous album's cover once under the new one's title first.
  const [asked, setAsked] = useState(artworkId);
  if (asked !== artworkId) {
    setAsked(artworkId);
    setSrc(held);
    setWanted(!observable || held !== null);
  }

  useEffect(() => {
    if (!wanted) return;
    let cancelled = false;
    void artworkUrl(artworkId).then((url) => {
      if (!cancelled) setSrc(url);
    });
    return () => {
      cancelled = true;
    };
  }, [artworkId, wanted]);

  const classes = `cover ${className}`.trim();
  if (src) return <img ref={observe} className={classes} src={src} alt="" />;
  return (
    <div
      ref={observe}
      className={`${classes} cover-fallback`}
      aria-hidden="true"
    >
      <span>{title.slice(0, 1).toLocaleUpperCase()}</span>
    </div>
  );
}
