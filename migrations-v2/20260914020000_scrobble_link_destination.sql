-- Which of a destination's instances a link authorises, and how to tell it is
-- still the same machine.
--
-- RFC-010 decision 10, revised: one URL field per recipient assumed a singular
-- that self-hosting does not have. ListenBrainz mostly means the public
-- instance; Maloja is self-hosted by nature, and on a family server everyone
-- has their own. The operator therefore declares *named* destinations, and a
-- link carries `(provider, destination)`.
--
-- The account still describes no URL: it picks a name among those the server
-- publishes, which is decision 10's barrier, untouched.
ALTER TABLE scrobble_link ADD COLUMN destination TEXT;

-- A name is not an identity: the URL is part of it.
--
-- Removing `maloja/alice` breaks its links, but pointing the same name at
-- another URL would send them elsewhere with nothing changing name — the same
-- substitution decision 4 builds a whole tier of identifiers to prevent, by the
-- side door, and the easiest one to commit since it looks like fixing a typo. A
-- link therefore keeps enough to recognise the destination it meant, and a
-- reconciliation that no longer finds a match breaks it as if the name had
-- disappeared.
--
-- A digest of the canonical URL rather than the URL. `scrobble_link` carries no
-- destination address today — decision 10 again, since a member never describes
-- one — and this column exists to be *compared*, never read back. Storing the
-- address would widen what the table holds for no gain: two fingerprints
-- answer the only question anybody asks of them.
ALTER TABLE scrobble_link ADD COLUMN destination_fingerprint TEXT;

-- Both nullable, and filled at the next boot rather than here.
--
-- `Database::migrate` has only the database; a fingerprint is computed over a
-- URL that comes from `Config::from_env`, which a migration never sees.
-- `initialize` fills them, under the writer gate and before the drain starts,
-- as the first turn of the reconciliation the same place performs afterwards.

-- At most one live generation per account *and instance*.
--
-- The previous index was `(user_id, provider)`, which simply forbade the case
-- this migration exists for. Rebuilt rather than added beside: two indexes, one
-- of them still refusing a second instance, would leave the feature
-- unreachable while looking present.
--
-- Legacy rows carry a NULL destination until `initialize` fills them, and
-- SQLite holds NULLs distinct in a unique index, so the constraint is briefly
-- weaker than it was. It is briefly weaker over a boot, between `migrate` and
-- `initialize`, before the server answers anything — and the old index
-- guaranteed there was at most one such row per pair to begin with.
DROP INDEX scrobble_link_live_idx;
CREATE UNIQUE INDEX scrobble_link_live_idx
    ON scrobble_link(user_id, provider, destination) WHERE status <> 'unlinked';
