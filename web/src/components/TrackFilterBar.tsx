// TrackFilterBar — shared search/sort/codec filter strip for the
// library tracks browse surface. Owns no state itself; the parent
// holds the filter values + the filtered result so the filter is
// preserved across re-renders + can be reused for the album / artist
// drill-downs in a follow-up.
//
// All filtering happens client-side over the already-loaded tracks
// array. Server-side FTS5 over `title` / `album_title` / `artist_name`
// is a future endpoint (the schema's FTS5 trigger exists on the
// desktop SQLite but Postgres ships pg_trgm / full-text via tsvector
// which is a separate migration — out of scope here).

import { useMemo } from 'react'
import type { Track } from '@/server-fns/tracks'

/**
 * Sort discriminator. `'recent'` is "as the server returned" — the
 * `/tracks` endpoint already orders by `added_at DESC` so we don't
 * have to touch it. `'title'` / `'duration'` re-sort the rendered
 * array client-side; the parent renders the result as-is.
 */
export type SortMode = 'recent' | 'title' | 'duration'

export interface TrackFilters {
  /** Free-text query, matched case-insensitively against `track.title`. */
  query: string
  sortMode: SortMode
  /**
   * Codec to keep (e.g. `'FLAC'`), or `null` to keep everything.
   * Comparison is case-insensitive — tags ship `FLAC` / `flac`
   * interchangeably so a naive `===` would split the same codec
   * into two chips.
   */
  codec: string | null
}

interface TrackFilterBarProps {
  tracks: Track[]
  filters: TrackFilters
  onFiltersChange: (next: TrackFilters) => void
}

export function TrackFilterBar({ tracks, filters, onFiltersChange }: TrackFilterBarProps) {
  // Distinct codec list, sorted alphabetically. Memoised on the
  // tracks reference so re-renders from filter typing don't re-walk
  // the array.
  const codecs = useMemo(() => collectCodecs(tracks), [tracks])

  return (
    <div className="quiet-panel mb-6 flex flex-col gap-3 p-3">
      <div className="flex flex-wrap items-center gap-3">
        <label className="flex-1 min-w-[12rem]">
          <span className="sr-only">Search tracks</span>
          <input
            type="search"
            value={filters.query}
            onChange={(e) => onFiltersChange({ ...filters, query: e.target.value })}
            placeholder="Search by title…"
            className="input text-sm"
          />
        </label>
        <label className="flex items-center gap-2 text-sm text-[var(--sea-ink-soft)]">
          Sort:
          <select
            value={filters.sortMode}
            onChange={(e) => onFiltersChange({ ...filters, sortMode: e.target.value as SortMode })}
            className="select min-h-0 py-2 text-sm"
          >
            <option value="recent">Recently added</option>
            <option value="title">Title (A→Z)</option>
            <option value="duration">Duration (shortest first)</option>
          </select>
        </label>
      </div>
      {codecs.length > 0 && (
        <div
          role="group"
          aria-label="Filter by codec"
          className="flex flex-wrap items-center gap-2"
        >
          <CodecChip
            label="All"
            // The "All" chip clears the filter rather than
            // selecting a codec — without an explicit aria-label
            // a screen reader announces it as "All button"
            // alongside the codec chips with no hint of its
            // reset semantics.
            ariaLabel={filters.codec === null ? 'All codecs (selected)' : 'Show all codecs'}
            active={filters.codec === null}
            onClick={() => onFiltersChange({ ...filters, codec: null })}
          />
          {codecs.map((codec) => (
            <CodecChip
              key={codec}
              label={codec}
              active={isSameCodec(filters.codec, codec)}
              onClick={() =>
                onFiltersChange({
                  ...filters,
                  // Toggle off if the user clicks the active chip
                  // (Spotify / Apple Music convention) so they don't
                  // have to hunt for the "All" chip to clear it.
                  codec: isSameCodec(filters.codec, codec) ? null : codec,
                })
              }
            />
          ))}
        </div>
      )}
    </div>
  )
}

function CodecChip({
  label,
  ariaLabel,
  active,
  onClick,
}: {
  label: string
  ariaLabel?: string
  active: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      aria-label={ariaLabel}
      className={`rounded-lg border px-3 py-1.5 text-xs font-semibold transition ${
        active
          ? 'border-[var(--accent-600)] bg-[var(--accent-600)] text-white'
          : 'border-[var(--line)] bg-[var(--chip-bg)] text-[var(--sea-ink)] hover:opacity-90'
      }`}
    >
      {label}
    </button>
  )
}

function collectCodecs(tracks: Track[]): string[] {
  // Normalise to upper-case for display since most tags ship the
  // codec name as-is (FLAC / flac / Flac all surface in the wild).
  // The chip click compares case-insensitively via `isSameCodec`
  // so the user clicking "FLAC" still matches a `flac`-tagged row.
  const seen = new Set<string>()
  for (const t of tracks) {
    if (t.codec) seen.add(t.codec.toUpperCase())
  }
  return Array.from(seen).sort()
}

function isSameCodec(a: string | null, b: string): boolean {
  if (a === null) return false
  return a.toLowerCase() === b.toLowerCase()
}

/**
 * Apply `filters` to `tracks` and return a NEW array — callers can
 * pass the result straight to a memoised renderer. The original
 * array is never mutated.
 *
 * Convention: `sortMode === 'recent'` is a no-op because the server
 * already returns tracks `added_at DESC`. The branch is here for
 * future-proofing against a server-side re-ordering (e.g. an
 * alphabetic default) — the wire shape stays decoupled from the
 * client's preference.
 */
export function applyFilters(tracks: Track[], filters: TrackFilters): Track[] {
  let result = tracks
  const q = filters.query.trim().toLowerCase()
  if (q) {
    result = result.filter((t) => t.title.toLowerCase().includes(q))
  }
  if (filters.codec !== null) {
    const target = filters.codec.toLowerCase()
    result = result.filter((t) => t.codec !== null && t.codec.toLowerCase() === target)
  }
  if (filters.sortMode === 'title') {
    // localeCompare so accented / non-ASCII titles sort the way the
    // user's locale expects (`É` before `F` in French, etc.).
    result = [...result].sort((a, b) => a.title.localeCompare(b.title))
  } else if (filters.sortMode === 'duration') {
    result = [...result].sort((a, b) => a.duration_ms - b.duration_ms)
  }
  return result
}

/** Default `TrackFilters` for first mount. */
export const initialTrackFilters: TrackFilters = {
  query: '',
  sortMode: 'recent',
  codec: null,
}
