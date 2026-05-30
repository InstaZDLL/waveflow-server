// Postgres connection for Better Auth's Kysely adapter.
//
// Better Auth supports several DB backends (Drizzle, Prisma, Kysely).
// Kysely is the lightest — type-safe query builder, no codegen — and
// pairs cleanly with `node-postgres`, which we already need for the
// future direct queries that don't go through Better Auth.
//
// The pool is built lazily on first import. Production sits behind a
// pgbouncer / RDS proxy in the canonical deploy, so the in-process
// pool stays small (10 connections is plenty for a single Nitro
// instance). Bump via `BETTER_AUTH_DB_MAX` if you co-deploy without
// a pooler.

import { Kysely, PostgresDialect } from 'kysely'
import { Pool } from 'pg'

if (!process.env.DATABASE_URL) {
  // Fail loud at boot — Better Auth's middleware would otherwise
  // throw on every request with a less helpful stack.
  throw new Error(
    'DATABASE_URL is required. Copy .env.example to .env and fill in the connection string.',
  )
}

const max = Number.parseInt(process.env.BETTER_AUTH_DB_MAX ?? '10', 10)
if (!Number.isFinite(max) || max <= 0) {
  throw new Error(
    `BETTER_AUTH_DB_MAX must be a positive integer, got ${process.env.BETTER_AUTH_DB_MAX}`,
  )
}

const pool = new Pool({
  connectionString: process.env.DATABASE_URL,
  max,
})

// Kysely instance is exported untyped (`Kysely<unknown>`) for now —
// Better Auth manages its own tables and supplies the types internally
// via its adapter. The day we run direct Kysely queries from our own
// code, we'll codegen a `Database` interface and swap `unknown` for it.
export const db = new Kysely<unknown>({
  dialect: new PostgresDialect({ pool }),
})
