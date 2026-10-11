/**
 * The first-load screen, and when it is allowed to leave.
 *
 * It lives in `index.html` rather than in this bundle, which is the whole
 * point: `#root` is empty until React has parsed and committed, and React and
 * TanStack are 276 kB arriving before any of that. A splash that shipped with
 * them could only appear after the wait it exists to dress.
 */

/**
 * The shortest time the mark stays on screen, measured from navigation start.
 *
 * **Three hundred milliseconds, and deliberately not one or two seconds.** A
 * longer floor would make the application permanently slower for everyone in
 * order to hide a load that takes 79ms. Dressing a wait and manufacturing one
 * are not the same gesture, and the second cannot be taken back later without
 * somebody noticing.
 *
 * What the floor buys is the absence of a flash: a mark drawn for 40ms and
 * yanked reads as a glitch, not as a brand.
 */
const FLOOR_MS = 300;

/** Long enough to read as a fade, short enough not to be a wait of its own. */
const FADE_MS = 180;

/**
 * Take the first-load screen away, once it has been up long enough.
 *
 * Idempotent: StrictMode runs an effect twice in development, and a second
 * call must not schedule a second removal.
 *
 * `performance.now()` is milliseconds since navigation start, which is the
 * origin the floor is about — the screen became visible when the document
 * painted, not when this module was reached.
 */
export function dismissSplash(): void {
  const splash = document.getElementById("splash");
  if (!splash || splash.dataset.leaving) return;
  splash.dataset.leaving = "pending";
  const remaining = Math.max(0, FLOOR_MS - performance.now());
  window.setTimeout(() => {
    // The attribute both starts the fade and lifts `pointer-events`, so the
    // page beneath becomes clickable from this instant rather than from the
    // removal 180ms later — which matters most where the fade is disabled and
    // the screen is invisible for all of it.
    splash.dataset.leaving = "true";
    // Removed as well as hidden: an element over the whole viewport is still
    // walked by an accessibility sweep, and `pointer-events` is no reason to
    // leave one lying about.
    window.setTimeout(() => splash.remove(), FADE_MS);
  }, remaining);
}
