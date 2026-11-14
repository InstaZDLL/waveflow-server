-- A track's music video, as the scan found it on the disk. RFC-029 decision 3.
--
-- The disk is the authority: no route writes here, only the scan's link pass,
-- which records what the desktop's rule (`waveflow_core::scanner::clips`)
-- answers for each track it walked. A table rather than a column on `track`
-- because an incremental scan skips an unchanged audio file without writing
-- its row, and a clip added beside it must still be recorded.
--
-- `library_id` is the track's, read in the writing transaction, never
-- computed from the clip's path, as `track_canvas` does it.
--
-- `byte_size` and `modified_at` let the next incremental scan take an
-- unchanged candidate as confirmed without opening it, the trade RFC-018
-- already makes for audio; a full scan reads it again.
CREATE TABLE track_clip (
    track_id TEXT PRIMARY KEY NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('mp4', 'm4v', 'mov', 'webm', 'mkv')),
    origin TEXT NOT NULL CHECK (origin IN ('sibling', 'clips_dir', 'itself')),
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    modified_at INTEGER NOT NULL
) STRICT;

CREATE INDEX track_clip_by_library ON track_clip(library_id);
