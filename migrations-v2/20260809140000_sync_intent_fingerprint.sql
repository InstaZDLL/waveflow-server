-- An operation UUID is idempotent only for the exact normalized mutation
-- intent that first claimed it. Existing rows remain NULL and are rejected on
-- replay because their original intent cannot be proven.
ALTER TABLE sync_operation ADD COLUMN intent_hash BLOB;
