-- The other half of carrying scopes through the grant.
--
-- A session is no longer unscoped by construction: one redeemed from a
-- narrowed authorization inherits that narrowing, so `authenticate` reads the
-- limit off the session the same way it already reads it off an API token.
--
-- Existing rows default to the empty list — a session issued before this
-- migration came from a password login or from a grant that could only have
-- been made by an unscoped credential, so unrestricted is what they were.
ALTER TABLE session
    ADD COLUMN scopes_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(scopes_json));
