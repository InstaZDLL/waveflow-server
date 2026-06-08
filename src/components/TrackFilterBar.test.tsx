// TrackFilterBar — render + applyFilters tests. The bar is a pure
// state-out component (the parent owns the filter object) so most of
// the coverage targets `applyFilters` directly; render specs check
// the search input + codec chips + sort dropdown wire up to the
// onFiltersChange callback.

import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import {
  TrackFilterBar,
  applyFilters,
  initialTrackFilters,
  type TrackFilters,
} from './TrackFilterBar'
import type { Track } from '@/server-fns/tracks'

function makeTrack(id: number, overrides: Partial<Track> = {}): Track {
  return {
    id,
    library_id: 2,
    album_id: null,
    title: `Track ${id}`,
    file_path: `/${id}.flac`,
    file_size: 1024,
    duration_ms: 60_000,
    track_number: null,
    disc_number: null,
    year: null,
    bitrate: null,
    sample_rate: null,
    channels: null,
    bit_depth: null,
    codec: null,
    rating: null,
    ...overrides,
  }
}

describe('applyFilters', () => {
  const tracks: Track[] = [
    makeTrack(1, { title: 'Cosmic Dust', codec: 'FLAC', duration_ms: 240_000 }),
    makeTrack(2, { title: 'Aurora', codec: 'MP3', duration_ms: 180_000 }),
    makeTrack(3, { title: 'cosmic ray', codec: 'flac', duration_ms: 90_000 }),
    makeTrack(4, { title: 'Twilight', codec: null, duration_ms: 300_000 }),
  ]

  it('returns the original array when no filters are active', () => {
    const result = applyFilters(tracks, initialTrackFilters)
    expect(result).toEqual(tracks)
  })

  it('matches the search query case-insensitively against title', () => {
    const result = applyFilters(tracks, { ...initialTrackFilters, query: 'cosmic' })
    expect(result.map((t) => t.id)).toEqual([1, 3])
  })

  it('trims whitespace around the search query', () => {
    const result = applyFilters(tracks, { ...initialTrackFilters, query: '  aurora  ' })
    expect(result.map((t) => t.id)).toEqual([2])
  })

  it('filters by codec case-insensitively', () => {
    // `'FLAC'` and the lower-case `'flac'` track both surface — the
    // bar's chip normalises to upper-case for display but the
    // filter compares the raw values lowercased.
    const result = applyFilters(tracks, { ...initialTrackFilters, codec: 'FLAC' })
    expect(result.map((t) => t.id)).toEqual([1, 3])
  })

  it('excludes tracks with a null codec when a codec filter is set', () => {
    const result = applyFilters(tracks, { ...initialTrackFilters, codec: 'MP3' })
    expect(result.map((t) => t.id)).toEqual([2])
  })

  it('sorts by title alphabetically with locale-aware compare', () => {
    const result = applyFilters(tracks, { ...initialTrackFilters, sortMode: 'title' })
    // Aurora < Cosmic Dust < cosmic ray < Twilight (localeCompare
    // is case-insensitive by default for the same script).
    expect(result.map((t) => t.title)).toEqual(['Aurora', 'Cosmic Dust', 'cosmic ray', 'Twilight'])
  })

  it('sorts by duration shortest first', () => {
    const result = applyFilters(tracks, { ...initialTrackFilters, sortMode: 'duration' })
    expect(result.map((t) => t.duration_ms)).toEqual([90_000, 180_000, 240_000, 300_000])
  })

  it('combines search + codec + sort in one pass', () => {
    const result = applyFilters(tracks, {
      query: 'cosmic',
      codec: 'FLAC',
      sortMode: 'duration',
    })
    // Both "Cosmic Dust" + "cosmic ray" match the search AND the
    // FLAC codec filter; sort by duration puts the ray first.
    expect(result.map((t) => t.id)).toEqual([3, 1])
  })
})

describe('TrackFilterBar', () => {
  const tracks: Track[] = [
    makeTrack(1, { codec: 'FLAC' }),
    makeTrack(2, { codec: 'MP3' }),
    makeTrack(3, { codec: 'flac' }),
  ]

  it('renders the search input + sort select + a chip per distinct codec', () => {
    render(
      <TrackFilterBar tracks={tracks} filters={initialTrackFilters} onFiltersChange={() => {}} />,
    )
    expect(screen.getByRole('searchbox')).toBeTruthy()
    expect(screen.getByRole('combobox')).toBeTruthy()
    // "All" chip has an explicit aria-label so screen readers
    // distinguish "clear filter" from a codec name. With no
    // codec selected the label is the "selected" form;
    // selecting a codec flips it to the action prompt
    // "Show all codecs".
    expect(screen.getByRole('button', { name: /all codecs \(selected\)/i })).toBeTruthy()
    // The 2 distinct codecs surface (FLAC dedupes the
    // case-mismatched pair).
    expect(screen.getByRole('button', { name: 'FLAC' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'MP3' })).toBeTruthy()
  })

  it('emits an onFiltersChange call when the search input changes', async () => {
    const onFiltersChange = vi.fn()
    const user = userEvent.setup()
    render(
      <TrackFilterBar
        tracks={tracks}
        filters={initialTrackFilters}
        onFiltersChange={onFiltersChange}
      />,
    )
    await user.type(screen.getByRole('searchbox'), 'X')
    expect(onFiltersChange).toHaveBeenLastCalledWith({ ...initialTrackFilters, query: 'X' })
  })

  it('emits an onFiltersChange call selecting the codec when a chip is clicked', async () => {
    const onFiltersChange = vi.fn()
    const user = userEvent.setup()
    render(
      <TrackFilterBar
        tracks={tracks}
        filters={initialTrackFilters}
        onFiltersChange={onFiltersChange}
      />,
    )
    await user.click(screen.getByRole('button', { name: 'FLAC' }))
    expect(onFiltersChange).toHaveBeenCalledWith({ ...initialTrackFilters, codec: 'FLAC' })
  })

  it('toggles a codec chip off when it is already active', async () => {
    const onFiltersChange = vi.fn()
    const filters: TrackFilters = { ...initialTrackFilters, codec: 'FLAC' }
    const user = userEvent.setup()
    render(<TrackFilterBar tracks={tracks} filters={filters} onFiltersChange={onFiltersChange} />)
    await user.click(screen.getByRole('button', { name: 'FLAC' }))
    expect(onFiltersChange).toHaveBeenCalledWith({ ...filters, codec: null })
  })

  it('hides the codec row entirely when no track carries a codec', () => {
    const codecless = tracks.map((t) => ({ ...t, codec: null }))
    render(
      <TrackFilterBar
        tracks={codecless}
        filters={initialTrackFilters}
        onFiltersChange={() => {}}
      />,
    )
    // No codec group renders when there's nothing to filter on.
    expect(screen.queryByRole('group', { name: /filter by codec/i })).toBeNull()
  })
})
