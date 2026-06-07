// PlayerContext unit tests. We exercise the state machine through
// `usePlayer` without rendering an `<audio>` element — jsdom's
// media element doesn't actually play, so the assertions live at
// the context layer (current / queue / history transitions).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook } from '@testing-library/react'

const getStreamUrl = vi.fn()

vi.mock('@/server-fns/stream', () => ({
  getStreamUrl: (...args: unknown[]) => getStreamUrl(...args),
}))

const { PlayerProvider, usePlayer } = await import('./player-context')

function makeEntry(trackId: number, title = `Track ${trackId}`) {
  return {
    profileId: 1,
    libraryId: 1,
    trackId,
    title,
    durationMs: 60_000,
  }
}

beforeEach(() => {
  getStreamUrl.mockReset()
  getStreamUrl.mockImplementation(async ({ data }: { data: { trackId: number } }) => ({
    url: `https://stream.example/track/${data.trackId}.mp3`,
  }))
  window.localStorage.clear()
})

afterEach(() => {
  window.localStorage.clear()
})

function mount() {
  return renderHook(() => usePlayer(), {
    wrapper: ({ children }) => <PlayerProvider>{children}</PlayerProvider>,
  })
}

describe('PlayerProvider — initial state', () => {
  it('idles with no current / empty queue / volume from localStorage default', () => {
    const { result } = mount()
    expect(result.current.current).toBeNull()
    expect(result.current.queue).toEqual([])
    expect(result.current.history).toEqual([])
    expect(result.current.isPlaying).toBe(false)
    expect(result.current.volume).toBe(0.8)
    expect(result.current.isLoading).toBe(false)
  })

  it('reads a persisted volume from localStorage', () => {
    window.localStorage.setItem('waveflow-player-volume', '0.3')
    const { result } = mount()
    expect(result.current.volume).toBe(0.3)
  })

  it('falls back to the default volume when the stored value is invalid', () => {
    window.localStorage.setItem('waveflow-player-volume', 'not-a-number')
    const { result } = mount()
    expect(result.current.volume).toBe(0.8)
  })
})

describe('playTrack', () => {
  it('mints a URL, sets current, seeds the queue, clears history', async () => {
    const { result } = mount()
    const entry = makeEntry(7)
    const queue = [makeEntry(8), makeEntry(9)]
    await act(async () => {
      await result.current.playTrack(entry, queue)
    })
    expect(result.current.current).toEqual({
      ...entry,
      url: 'https://stream.example/track/7.mp3',
    })
    expect(result.current.queue).toEqual(queue)
    expect(result.current.history).toEqual([])
    expect(result.current.isLoading).toBe(false)
  })

  it('flips isPlaying to true on a fresh load (autoplay)', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1))
    })
    expect(result.current.isPlaying).toBe(true)
  })

  it('leaves prior state untouched + rethrows on a mint failure', async () => {
    const { result } = mount()
    // First load — succeeds, becomes the prior state we want
    // preserved across the failed second call.
    await act(async () => {
      await result.current.playTrack(makeEntry(1), [makeEntry(99)])
    })
    const priorCurrent = result.current.current
    const priorQueue = result.current.queue
    getStreamUrl.mockRejectedValueOnce(new Error('mint failed'))
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})

    await act(async () => {
      await expect(result.current.playTrack(makeEntry(2))).rejects.toThrow('mint failed')
    })

    expect(result.current.current).toBe(priorCurrent)
    expect(result.current.queue).toBe(priorQueue)
    expect(result.current.isLoading).toBe(false)
    errorSpy.mockRestore()
  })

  it('drops the response when a newer playTrack supersedes it', async () => {
    const { result } = mount()
    let resolveFirst: ((value: { url: string }) => void) | undefined
    getStreamUrl.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveFirst = resolve
        }),
    )
    getStreamUrl.mockResolvedValueOnce({ url: 'https://stream.example/track/2.mp3' })

    let firstCall: Promise<void> | undefined
    act(() => {
      firstCall = result.current.playTrack(makeEntry(1))
    })
    let secondCall: Promise<void> | undefined
    act(() => {
      secondCall = result.current.playTrack(makeEntry(2))
    })
    await act(async () => {
      await secondCall
    })
    expect(result.current.current?.trackId).toBe(2)

    // First call resolves AFTER the second one — it must NOT
    // clobber the current track.
    await act(async () => {
      resolveFirst?.({ url: 'https://stream.example/track/1.mp3' })
      await firstCall
    })
    expect(result.current.current?.trackId).toBe(2)
  })
})

describe('next / previous', () => {
  it('next advances through the queue and pushes the prior current to history', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1), [makeEntry(2), makeEntry(3)])
    })
    await act(async () => {
      await result.current.next()
    })
    expect(result.current.current?.trackId).toBe(2)
    expect(result.current.queue.map((q) => q.trackId)).toEqual([3])
    expect(result.current.history.map((h) => h.trackId)).toEqual([1])
  })

  it('next is a no-op when the queue is empty', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1))
    })
    const callsBefore = getStreamUrl.mock.calls.length
    await act(async () => {
      await result.current.next()
    })
    expect(getStreamUrl.mock.calls.length).toBe(callsBefore)
    expect(result.current.current?.trackId).toBe(1)
  })

  it('previous pops history back into current and pushes current to queue head', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1), [makeEntry(2)])
    })
    await act(async () => {
      await result.current.next()
    })
    // Now: current=2, queue=[], history=[1]. Previous should go
    // back to 1 and put 2 at the front of the queue.
    await act(async () => {
      await result.current.previous()
    })
    expect(result.current.current?.trackId).toBe(1)
    expect(result.current.queue.map((q) => q.trackId)).toEqual([2])
    expect(result.current.history).toEqual([])
  })

  it('previous restarts the current track when position > 3s', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1), [makeEntry(2)])
    })
    await act(async () => {
      await result.current.next()
    })
    // History has 1, but position > 3 should suppress the back step.
    act(() => {
      result.current.setPosition(10)
    })
    await act(async () => {
      await result.current.previous()
    })
    expect(result.current.current?.trackId).toBe(2)
    expect(result.current.position).toBe(0)
    expect(result.current.history.map((h) => h.trackId)).toEqual([1])
  })

  it('previous is a no-op when history is empty and position ≤ 3s', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1))
    })
    const callsBefore = getStreamUrl.mock.calls.length
    await act(async () => {
      await result.current.previous()
    })
    expect(getStreamUrl.mock.calls.length).toBe(callsBefore)
    expect(result.current.current?.trackId).toBe(1)
  })
})

describe('playQueueAt', () => {
  it('jumps to a queue entry + pushes the prior current into history', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1), [makeEntry(2), makeEntry(3), makeEntry(4)])
    })
    // Jump to index 2 → track 4. Skipped 2 + 3 were never heard,
    // so they DON'T enter history — only the prior current (1)
    // does. Mirrors next()'s history shape so
    // history[0] = most-recently-heard remains invariant.
    await act(async () => {
      await result.current.playQueueAt(2)
    })
    expect(result.current.current?.trackId).toBe(4)
    expect(result.current.queue).toEqual([])
    expect(result.current.history.map((h) => h.trackId)).toEqual([1])
  })

  it('plays a queue entry at index 0 without leaking it back into history', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1), [makeEntry(2)])
    })
    await act(async () => {
      await result.current.playQueueAt(0)
    })
    expect(result.current.current?.trackId).toBe(2)
    expect(result.current.history.map((h) => h.trackId)).toEqual([1])
  })

  it('a jump + a previous returns to the prior current (not a skipped item)', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1), [makeEntry(2), makeEntry(3), makeEntry(4)])
    })
    // Play 1, jump 4 ; previous should walk back to 1, NOT 3 or 2.
    await act(async () => {
      await result.current.playQueueAt(2)
    })
    await act(async () => {
      await result.current.previous()
    })
    expect(result.current.current?.trackId).toBe(1)
  })

  it('is a no-op for an out-of-range index', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1), [makeEntry(2)])
    })
    const callsBefore = getStreamUrl.mock.calls.length
    await act(async () => {
      await result.current.playQueueAt(99)
    })
    await act(async () => {
      await result.current.playQueueAt(-1)
    })
    expect(getStreamUrl.mock.calls.length).toBe(callsBefore)
    expect(result.current.current?.trackId).toBe(1)
  })
})

describe('volume + togglePlayPause', () => {
  it('togglePlayPause flips isPlaying when a track is loaded', async () => {
    const { result } = mount()
    await act(async () => {
      await result.current.playTrack(makeEntry(1))
    })
    // playTrack autoplays — first toggle pauses, second resumes.
    expect(result.current.isPlaying).toBe(true)
    act(() => result.current.togglePlayPause())
    expect(result.current.isPlaying).toBe(false)
    act(() => result.current.togglePlayPause())
    expect(result.current.isPlaying).toBe(true)
  })

  it('togglePlayPause is a no-op when idle', () => {
    const { result } = mount()
    act(() => result.current.togglePlayPause())
    expect(result.current.isPlaying).toBe(false)
  })

  it('setVolume clamps to [0, 1] and writes to localStorage', () => {
    const { result } = mount()
    act(() => result.current.setVolume(0.42))
    expect(result.current.volume).toBe(0.42)
    expect(window.localStorage.getItem('waveflow-player-volume')).toBe('0.42')
    act(() => result.current.setVolume(2))
    expect(result.current.volume).toBe(1)
    act(() => result.current.setVolume(-0.5))
    expect(result.current.volume).toBe(0)
  })
})

describe('seek', () => {
  it('bumps seekVersion + carries the target seconds + mirrors position', () => {
    const { result } = mount()
    const initialVersion = result.current.seekVersion
    act(() => result.current.seek(42))
    expect(result.current.seekVersion).toBe(initialVersion + 1)
    expect(result.current.seekTargetSec).toBe(42)
    expect(result.current.position).toBe(42)
  })

  it('every seek call bumps the version even with the same target', () => {
    const { result } = mount()
    const v0 = result.current.seekVersion
    act(() => result.current.seek(10))
    act(() => result.current.seek(10))
    // Second seek to the same value still counts — PlayerBar
    // needs to apply it again in case the audio drifted.
    expect(result.current.seekVersion).toBe(v0 + 2)
  })
})

describe('usePlayer outside provider', () => {
  it('throws', () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    expect(() => renderHook(() => usePlayer())).toThrow(/inside <PlayerProvider>/)
    errorSpy.mockRestore()
  })
})
