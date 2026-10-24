-- Splitting a joined artist credit, by decision (RFC-015).
--
-- A comma is not a separator the scanner may cut on by itself: `Tyler, The
-- Creator` is one artist and `Zedd, Alessia Cara` is two, and nothing in the
-- tags says which. So a person decides, once per library, and the decision is
-- a row every scan reads back — which is what the desktop's own list never
-- had, and why its splits came undone whenever a file changed.

-- One decision per credit and library. The key is the credit folded the way
-- the artist identity folds a name (lower case, then `pid::str_clear`), so it
-- names exactly the spellings that make one artist row, and it survives a
-- change of `WAVEFLOW_PID_ARTIST` that an artist id would not.
--
-- `parts` holds the fragments as they were shown and accepted, so a recorded
-- split keeps its meaning if the fragment rule ever changes. `decided_by` is an
-- account id without a foreign key, as `artist_image_choice.set_by` is: an
-- account removed later does not take the library's decision with it.
CREATE TABLE artist_credit_decision (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    credit_key TEXT NOT NULL CHECK (length(credit_key) > 0),
    credit TEXT NOT NULL CHECK (length(trim(credit)) > 0),
    decision TEXT NOT NULL CHECK (decision IN ('split', 'keep')),
    parts TEXT CHECK (
        (decision = 'split' AND json_type(parts) = 'array')
        OR (decision = 'keep' AND parts IS NULL)),
    decided_by TEXT NOT NULL,
    decided_at INTEGER NOT NULL,
    UNIQUE (library_id, credit_key)
) STRICT;

-- The credits as the tag mapper produced them, before any decision: what a
-- decision is applied to, and what an undo goes back to. The participants
-- became a function of this table and of the decisions, so taking or removing
-- a decision needs no file read, and two credits sharing a part (`A, B` and
-- `A, C`) keep the file's order whichever is undone first.
--
-- Positions are per role, across sub-roles, as in `track_participant`: the
-- performers of one track are numbered in one sequence whatever their
-- instruments.
CREATE TABLE track_credit (
    track_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (length(trim(role)) > 0),
    sub_role TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL CHECK (position >= 0),
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    sort_name TEXT,
    PRIMARY KEY (track_id, role, position),
    FOREIGN KEY (track_id, library_id)
        REFERENCES track(id, library_id) ON DELETE CASCADE
) STRICT;

-- No decision exists yet, so the credits are what the participants already
-- say. The copy loses only a later credit naming the same artist as an earlier
-- one, which the participants never held and which the computation would drop
-- again by keeping the first occurrence; the next scan of that file writes it
-- back. Under a correction the copied `artist` rows are the corrected list
-- rather than the file's, which nothing reads while the correction stands, and
-- removing a correction re-reads the file. So no rescan is scheduled.
INSERT INTO track_credit (track_id, library_id, role, sub_role, position, name, sort_name)
SELECT tp.track_id, tp.library_id, tp.role, tp.sub_role, tp.position, ar.name, tp.sort_name
  FROM track_participant tp
  JOIN artist ar ON ar.id = tp.artist_id;
