// Pin the rendering contract for the public share preview's
// duration + runtime helpers (Phase 1.j.c). These pure functions
// drive both the in-page header and the `<meta>` description, so a
// silent regression would degrade social previews + the rendered
// page in lockstep.

import { describe, expect, it } from 'vitest'

import { formatDuration, formatTrackCountAndRuntime } from './share-format'
import type { PublicTrack } from '@/server-fns/share'

function track(durationMs: number): PublicTrack {
  return { title: 't', artist: null, duration_ms: durationMs }
}

describe('formatDuration', () => {
  it('renders mm:ss for sub-hour durations', () => {
    expect(formatDuration(60_000)).toBe('1:00')
    expect(formatDuration(125_000)).toBe('2:05')
    expect(formatDuration(599_000)).toBe('9:59')
  })

  it('renders h:mm:ss when the duration crosses an hour', () => {
    expect(formatDuration(3_600_000)).toBe('1:00:00')
    // 1h 23m 04s
    expect(formatDuration(4_984_000)).toBe('1:23:04')
  })

  it('rounds milliseconds rather than truncates them', () => {
    // 320500 ms → 321 s → 5:21, NOT 5:20. Mirrors the desktop UI.
    expect(formatDuration(320_500)).toBe('5:21')
  })

  it('renders an empty string for non-positive or non-finite input', () => {
    // Pre-1.j.b desktops emit `0` for the duration snapshot; render
    // the row with no duration column instead of "0:00".
    expect(formatDuration(0)).toBe('')
    expect(formatDuration(-1)).toBe('')
    expect(formatDuration(Number.NaN)).toBe('')
    expect(formatDuration(Number.POSITIVE_INFINITY)).toBe('')
  })
})

describe('formatTrackCountAndRuntime', () => {
  it('singularises the track noun for a single entry', () => {
    expect(formatTrackCountAndRuntime([track(180_000)])).toBe('1 track · 3 min')
  })

  it('plurals zero and many', () => {
    expect(formatTrackCountAndRuntime([])).toBe('0 tracks')
    expect(formatTrackCountAndRuntime([track(60_000), track(120_000)])).toBe('2 tracks · 3 min')
  })

  it('omits the runtime fragment when no track carries a duration', () => {
    // Pre-1.j.b wire — every snapshot ships `duration_ms = 0`.
    // We refuse to print "0 min" since it's misleading.
    expect(formatTrackCountAndRuntime([track(0), track(0)])).toBe('2 tracks')
  })

  it('switches to "Xh YY" past 60 minutes', () => {
    // 2 hours 27 minutes total (147 min) → "2 h 27" — reads faster
    // than "147 min".
    const totalMs = (2 * 60 + 27) * 60_000
    expect(formatTrackCountAndRuntime([track(totalMs)])).toBe('1 track · 2 h 27')
  })

  it('pads single-digit remainder minutes in the hour form', () => {
    // 1 hour 5 minutes total → "1 h 05" with a leading zero so the
    // column lines up across cards.
    const totalMs = (60 + 5) * 60_000
    expect(formatTrackCountAndRuntime([track(totalMs)])).toBe('1 track · 1 h 05')
  })

  it('clamps negative durations to zero in the runtime sum', () => {
    // A defensive reader could emit a negative — clamp so the total
    // doesn't go backwards.
    expect(formatTrackCountAndRuntime([track(60_000), track(-30_000)])).toBe('2 tracks · 1 min')
  })
})
