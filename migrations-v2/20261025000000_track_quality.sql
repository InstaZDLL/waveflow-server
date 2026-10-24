-- The real quality of a file, measured from its audio (RFC-016).
--
-- One measurement per track. It describes the content whose `full_hash` it
-- carries, taken by the analyser generation `version`, through the FFmpeg
-- build whose `ffmpeg -version` output hashes to `decoder`: a row whose hash
-- is no longer the track's is about a file that has gone, and a row from
-- another generation or another FFmpeg is about samples the server would not
-- produce today. Either way the track reads as `pending` until it is measured
-- again.
--
-- No verdict is stored. The verdict is SQL over these numbers and the
-- track's own columns, computed where it is read, so a change to its rules is
-- a code change and never a re-analysis.
CREATE TABLE track_quality (
    track_id TEXT PRIMARY KEY NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    full_hash TEXT NOT NULL CHECK (length(full_hash) > 0),
    version INTEGER NOT NULL CHECK (version > 0),
    decoder TEXT NOT NULL CHECK (length(decoder) = 64),
    outcome TEXT NOT NULL CHECK (outcome IN ('measured', 'silent', 'unreadable')),
    -- Where the wall stands, in hertz; NULL when the spectrum has none.
    cutoff_hz REAL,
    -- How far the level falls across the steepest candidate, wall or not.
    wall_db REAL,
    -- The bits the samples use; NULL for floating point, or all zero.
    effective_bits INTEGER CHECK (effective_bits BETWEEN 1 AND 32),
    -- The averaged spectrum: 256 bands from 0 Hz to Nyquist, one byte each,
    -- in half-decibels under the loudest band.
    spectrum BLOB CHECK (spectrum IS NULL OR length(spectrum) = 256),
    -- Why a file was unreadable. Never a message: FFmpeg's own words can carry
    -- the path.
    failure TEXT CHECK (failure IN ('decode', 'timeout')),
    analyzed_at INTEGER NOT NULL,
    CHECK ((outcome = 'measured') = (spectrum IS NOT NULL)),
    CHECK ((outcome = 'unreadable') = (failure IS NOT NULL))
) STRICT;
