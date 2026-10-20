import { useEffect } from "react";

import { artworkUrl } from "../api";
import { glowOf, sampleCover } from "../lib/cover-light";
import { usePlayer } from "../player/player";

/**
 * Lights the page with the playing record's cover.
 *
 * Writes `--glow` and `--glow-deep` on the document root, where the page's
 * atmosphere and the player bar read them. Only the dark themes use them: the
 * Fluid Editorial direction is dark-only, and a light theme keeps the ground
 * it has.
 *
 * Kept when playback stops — the guide's light is the playing *or last-played*
 * cover — and left alone when a cover cannot be sampled, so the page keeps the
 * previous light rather than flashing back to the default.
 */
export function CoverLight() {
  const artworkId = usePlayer().current?.artwork_hash ?? null;

  useEffect(() => {
    if (!artworkId) return;
    let cancelled = false;
    void (async () => {
      try {
        const url = await artworkUrl(artworkId);
        if (!url || cancelled) return;
        const colour = await sampleCover(url);
        // A sample that lands after the track changed must not write: it
        // would light the page with the previous record.
        if (!colour || cancelled) return;
        const { glow, deep } = glowOf(colour);
        const root = document.documentElement.style;
        root.setProperty("--glow", glow);
        root.setProperty("--glow-deep", deep);
      } catch {
        // A cover that cannot be decoded keeps the light already there.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [artworkId]);

  return null;
}
