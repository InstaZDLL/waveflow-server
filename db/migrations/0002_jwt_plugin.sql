-- JWT plugin schema. Stores the active + grace-period signing keys
-- the Better Auth `jwt()` plugin minted in `src/lib/auth.ts`. The
-- plugin reads the latest row to sign new JWTs and exposes every
-- non-expired row's `publicKey` through the JWKS endpoint at
-- `/api/auth/jwks` so verifiers (waveflow-server) can validate
-- bearer tokens without sharing a secret.
--
-- Column shape mirrors Better Auth's plugin schema declaration
-- (see `node_modules/better-auth/dist/plugins/jwt/index.d.mts`,
-- `schema.jwks.fields`). Names stay quoted-camelCase to match the
-- rest of Better Auth's tables.
--
-- The plugin generates a fresh key pair on first sign attempt if
-- the table is empty — no seeding needed. Rotation is opt-in via
-- `jwks.rotationInterval` in the plugin config; left disabled here.
--
-- The runner in `scripts/db-migrate.ts` owns the transaction for
-- this file, so no `BEGIN;` / `COMMIT;` wrapper.

CREATE TABLE jwks (
    id           TEXT      PRIMARY KEY,
    "publicKey"  TEXT      NOT NULL,
    "privateKey" TEXT      NOT NULL,
    "createdAt"  TIMESTAMP NOT NULL DEFAULT now(),
    "expiresAt"  TIMESTAMP
);

-- The plugin's `getLatest` query orders by `createdAt DESC LIMIT 1`,
-- the JWKS endpoint pulls every row where `expiresAt` is null or in
-- the future. Index both.
CREATE INDEX jwks_created_at_idx ON jwks("createdAt" DESC);
CREATE INDEX jwks_expires_at_idx ON jwks("expiresAt");
