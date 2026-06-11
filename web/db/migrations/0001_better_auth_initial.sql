-- Initial Better Auth schema. Mirrors the canonical shape Better Auth
-- expects (https://www.better-auth.com/docs/concepts/database) so the
-- email/password adapter wired in `src/lib/auth.ts` finds every table
-- + column on first request.
--
-- Hand-written rather than CLI-generated so contributors can read the
-- schema without standing up a live DB first. Drift policy: bump the
-- migration file name and ALTER from there — never edit an applied
-- migration (mirrors the rule from waveflow-server's sqlx setup).
--
-- Apply with any Postgres client:
--   psql "$DATABASE_URL" -f db/migrations/0001_better_auth_initial.sql
-- Or via the `bun run db:migrate` script (added in 1.c.2b).

BEGIN;

-- Better Auth uses TEXT primary keys (UUID strings). The `id` value
-- on the `user` row is exactly what gets minted into the JWT `sub`
-- claim — that's the same string waveflow-server resolves against
-- `users.external_id` (the column landed in waveflow-server PR #11).

CREATE TABLE "user" (
    id              TEXT        PRIMARY KEY,
    name            TEXT        NOT NULL,
    email           TEXT        NOT NULL UNIQUE,
    "emailVerified" BOOLEAN     NOT NULL DEFAULT FALSE,
    image           TEXT,
    "createdAt"     TIMESTAMP   NOT NULL DEFAULT now(),
    "updatedAt"     TIMESTAMP   NOT NULL DEFAULT now()
);

CREATE TABLE session (
    id           TEXT        PRIMARY KEY,
    "userId"     TEXT        NOT NULL
                 REFERENCES "user"(id) ON DELETE CASCADE,
    "expiresAt"  TIMESTAMP   NOT NULL,
    token        TEXT        NOT NULL UNIQUE,
    "ipAddress"  TEXT,
    "userAgent"  TEXT,
    "createdAt"  TIMESTAMP   NOT NULL DEFAULT now(),
    "updatedAt"  TIMESTAMP   NOT NULL DEFAULT now()
);
CREATE INDEX session_user_id_idx ON session("userId");
CREATE INDEX session_expires_at_idx ON session("expiresAt");

-- Each `account` row links a `user` to one credential source.
-- For email/password auth the password lives here (hashed by Better
-- Auth). For OAuth, `providerId` is the provider key (`google`,
-- `github`, …) and `accountId` is the provider's user id.
CREATE TABLE account (
    id                       TEXT      PRIMARY KEY,
    "userId"                 TEXT      NOT NULL
                             REFERENCES "user"(id) ON DELETE CASCADE,
    "providerId"             TEXT      NOT NULL,
    "accountId"              TEXT      NOT NULL,
    "accessToken"            TEXT,
    "refreshToken"           TEXT,
    "idToken"                TEXT,
    "accessTokenExpiresAt"   TIMESTAMP,
    "refreshTokenExpiresAt"  TIMESTAMP,
    scope                    TEXT,
    password                 TEXT,
    "createdAt"              TIMESTAMP NOT NULL DEFAULT now(),
    "updatedAt"              TIMESTAMP NOT NULL DEFAULT now()
);
CREATE INDEX account_user_id_idx ON account("userId");
-- A given provider account can only link to one user.
CREATE UNIQUE INDEX account_provider_id_account_id_idx
    ON account("providerId", "accountId");

-- Short-lived tokens for email-verification + password-reset flows.
-- Better Auth wipes expired rows on read; the explicit index keeps
-- the cleanup query flat as the table grows.
CREATE TABLE verification (
    id           TEXT        PRIMARY KEY,
    identifier   TEXT        NOT NULL,
    value        TEXT        NOT NULL,
    "expiresAt"  TIMESTAMP   NOT NULL,
    "createdAt"  TIMESTAMP   NOT NULL DEFAULT now(),
    "updatedAt"  TIMESTAMP   NOT NULL DEFAULT now()
);
CREATE INDEX verification_identifier_idx ON verification(identifier);
CREATE INDEX verification_expires_at_idx ON verification("expiresAt");

COMMIT;
