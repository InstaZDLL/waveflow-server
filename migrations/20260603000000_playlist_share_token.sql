-- Public share tokens for playlists. Phase 1.g.1 of the WaveFlow
-- roadmap. The desktop's "Share" modal calls the mint endpoint to
-- generate an opaque, unguessable token; the resulting URL
-- (`/p/{token}` on waveflow-web) opens an anonymous, read-only
-- preview of the playlist — no account required to view, no
-- streaming inside the preview (Phase 1.g.0 keeps the surface
-- minimal until server-side `playlist_track` materialisation
-- arrives in a follow-up).
--
-- Design choices, mirroring the same defaults the desktop will
-- inherit when the column lands there too:
--
-- - `TEXT` rather than `UUID` because the desktop mints via
--   `rand::distributions::Alphanumeric` (URL-safe 32-char string),
--   not a UUID — matches the stream-token convention and keeps the
--   public URL short. Validation against any specific format lives
--   in the application layer.
-- - `UNIQUE` index is partial (`WHERE share_token IS NOT NULL`) so
--   the vast majority of playlists (private) don't pay the index
--   bloat. Postgres supports partial UNIQUE indexes natively; a
--   plain UNIQUE column would reject the second NULL row on most
--   databases that key NULLs.
-- - Revoke = `UPDATE playlist SET share_token = NULL WHERE id = ?`.
--   Idempotent and instantaneously closes the public URL.

ALTER TABLE playlist ADD COLUMN share_token TEXT;

CREATE UNIQUE INDEX idx_playlist_share_token
    ON playlist (share_token)
    WHERE share_token IS NOT NULL;
