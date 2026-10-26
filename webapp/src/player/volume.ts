// The level this browser was left at, kept per device.

export const VOLUME_KEY = "waveflow.volume";

/**
 * The last level this browser was left at. Per-viewer and per-device on
 * purpose: how loud a laptop should be is not a property of the account.
 */
export function readStoredVolume(): number {
  try {
    // `Number(null)` is 0, and so is `Number("")`. Converting before checking
    // that anything was stored started every fresh browser silent, at a level
    // nobody had chosen and with the mute button reporting nothing was muted.
    const raw = localStorage.getItem(VOLUME_KEY);
    if (raw === null || raw.trim() === "") return 1;
    const saved = Number(raw);
    if (Number.isFinite(saved) && saved >= 0 && saved <= 1) return saved;
  } catch {
    // Private windows and blocked site data both land here; full volume is a
    // safe answer and the session still works without persistence.
  }
  return 1;
}
