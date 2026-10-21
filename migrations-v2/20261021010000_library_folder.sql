-- Filed folders, and where each track is filed (RFC-012 decision 2).
--
-- A filed folder exists only here: nothing on disk is created, moved, renamed
-- or deleted by anything that writes these tables. The disk tree is derived
-- from `track.relative_path` and is never stored.
--
-- `library_id` rides on every row so a parent, a folder and a track can only
-- ever meet inside one library: the composite keys make a cross-library
-- reference impossible to write rather than merely checked.
CREATE TABLE library_folder (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    -- NULL at the root. Deleting a folder takes its sub-folders with it.
    parent_id TEXT,
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 120),
    -- The name folded for comparison, written by the server: SQLite's NOCASE
    -- folds ASCII only, and "Été" and "été" must collide among siblings.
    name_key TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (id, library_id),
    FOREIGN KEY (parent_id, library_id)
        REFERENCES library_folder(id, library_id) ON DELETE CASCADE
) STRICT;

-- Sibling names are unique, the root counting as one parent. The service
-- checks first, to answer cleanly; this is what holds if it ever did not.
CREATE UNIQUE INDEX library_folder_sibling_name
    ON library_folder(library_id, ifnull(parent_id, ''), name_key);
CREATE INDEX library_folder_parent ON library_folder(parent_id);

-- At most one filed folder per track: the track id is the key. A track in no
-- folder is unfiled and has no row. A track deleted by a scan, and a folder
-- deleted by its owner, take their placements with them.
CREATE TABLE track_placement (
    track_id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    placed_by TEXT NOT NULL,
    placed_at INTEGER NOT NULL,
    FOREIGN KEY (track_id, library_id)
        REFERENCES track(id, library_id) ON DELETE CASCADE,
    FOREIGN KEY (folder_id, library_id)
        REFERENCES library_folder(id, library_id) ON DELETE CASCADE
) STRICT;

CREATE INDEX track_placement_folder ON track_placement(folder_id);
