-- Drop and recreate `profile` with `user_id` as a NOT NULL FK to the
-- newly-created `users` table. CRUD endpoints aren't live yet — the
-- table is empty in every known deployment (only the schema canary
-- in tests/ready.rs ever asserted on its existence), so a clean
-- DROP + CREATE is simpler than an in-place ALTER that would have to
-- backfill `user_id` for hypothetical pre-existing rows.
--
-- The shape mirrors `20260529000000_profile.sql` with the new
-- `user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE`
-- column threaded in. CASCADE so a user delete cleans up their
-- profile rows in one statement — every profile must belong to a
-- user, an orphaned profile would be a constraint violation anyway.

DROP TABLE IF EXISTS profile;

CREATE TABLE profile (
    id            BIGSERIAL    PRIMARY KEY,
    user_id       BIGINT       NOT NULL
                  REFERENCES users(id) ON DELETE CASCADE,
    name          TEXT         NOT NULL,
    color_id      TEXT         NOT NULL,
    avatar_hash   TEXT,
    data_dir      TEXT         NOT NULL DEFAULT '',
    created_at    BIGINT       NOT NULL,
    last_used_at  BIGINT       NOT NULL
);

-- `list_for_user` orders by `last_used_at DESC` filtered by user_id;
-- the composite index keeps the per-user MRU lookup flat as the
-- table grows across tenants. It also serves the equality filter on
-- its leading column for the `delete_guarded_for_user` lock query
-- (`SELECT id FROM profile WHERE user_id = $1 ORDER BY id FOR
-- UPDATE`) and the FK ON DELETE CASCADE lookup, so a dedicated
-- `user_id`-only index would be pure write amplification.
CREATE INDEX profile_user_last_used_idx
    ON profile (user_id, last_used_at DESC);
