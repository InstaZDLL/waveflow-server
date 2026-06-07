// Shared playback-time formatter — `123.4` seconds → `"2:03"`.
// Used by PlayerBar's seek strip and by NowPlayingOverlay's
// timeline. Falls back to `"0:00"` for negative / NaN inputs so a
// brief read of `audio.currentTime` before metadata loads doesn't
// flicker `"NaN:NaN"`.

export function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '0:00'
  const total = Math.floor(seconds)
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${s.toString().padStart(2, '0')}`
}
