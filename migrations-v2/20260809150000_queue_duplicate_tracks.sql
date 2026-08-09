CREATE TABLE play_queue_track_by_position (
    user_id TEXT NOT NULL REFERENCES play_queue(user_id) ON DELETE CASCADE,
    track_id TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (user_id, position)
) STRICT;

INSERT INTO play_queue_track_by_position (user_id, track_id, position)
SELECT user_id, track_id, position FROM play_queue_track;

DROP TABLE play_queue_track;
ALTER TABLE play_queue_track_by_position RENAME TO play_queue_track;
