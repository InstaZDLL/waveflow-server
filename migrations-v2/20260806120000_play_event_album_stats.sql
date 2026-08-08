-- The album read model counts and dates plays with correlated subqueries over
-- play_event joined to track. The existing play_event(user_id, played_at) index
-- does not serve them: they filter on user_id and submission, then look up by
-- track_id. Without this pair, listing albums degrades into a scan of every
-- play event per album as listening history grows.
CREATE INDEX play_event_user_submission_track_idx
    ON play_event (user_id, submission, track_id, played_at);

CREATE INDEX track_album_idx ON track (album_id);
