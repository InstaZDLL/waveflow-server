// Idempotent migration runner for `db/migrations/*.sql`.
//
// Reads `DATABASE_URL` from the environment (Bun auto-loads `.env`
// from the project root) and applies any migration filename not yet
// recorded in `_applied_migrations`. Each migration normally runs
// inside a transaction together with its bookkeeping INSERT — schema
// + checksum row commit (or roll back) together. A file's own outer
// `BEGIN;` / `COMMIT;` is stripped first so the runner's tx can wrap
// the whole apply cleanly without nesting.
//
// Opt-out: a leading `-- no-transaction` line (within the first 20
// lines) disables the wrapper for that file. Use it for statements
// Postgres refuses inside a transaction — `CREATE INDEX CONCURRENTLY`,
// `ALTER TYPE ... ADD VALUE` on enums, etc. Caveat: a mid-file failure
// leaves the schema partially applied; the bookkeeping row is only
// inserted on success, so re-running re-tries from the top.
//
// Usage:
//   bun run db:migrate              # apply all pending
//   bun run db:migrate --dry-run    # list pending without applying
//
// We deliberately don't track a checksum here (yet): the policy is
// "never edit an applied migration" — same rule as waveflow-server's
// sqlx setup. Filename-only tracking keeps the runner trivial and
// fails loudly if someone tries to re-number a merged file.
//
// Concurrency: this is a dev-time bootstrap script run by a single
// developer at a time, so we skip the `pg_advisory_lock` dance. The
// `filename` PRIMARY KEY on `_applied_migrations` makes a parallel
// apply degenerate cleanly anyway — the second runner's INSERT hits
// a unique-violation and its transaction rolls back. Production
// migrations should run from the deploy pipeline, not from here.

import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { Client } from 'pg'

const MIGRATIONS_DIR = join(process.cwd(), 'db', 'migrations')
const DRY_RUN = process.argv.includes('--dry-run')
// Opt-out marker for migrations that can't run in a transaction
// (e.g. `CREATE INDEX CONCURRENTLY`). Looked for on any of the first
// ~20 lines so it can sit next to or below the file's banner comment.
const NO_TRANSACTION_MARKER = /^(?:[^\n]*\n){0,20}?[ \t]*--[ \t]*no-transaction\b/i

async function main() {
  const url = process.env.DATABASE_URL
  if (!url) {
    console.error('[db:migrate] DATABASE_URL is not set. Copy .env.example to .env first.')
    process.exit(1)
  }

  const files = (await readdir(MIGRATIONS_DIR)).filter((name) => name.endsWith('.sql')).sort()

  if (files.length === 0) {
    console.log('[db:migrate] No migration files found.')
    return
  }

  const client = new Client({ connectionString: url })
  await client.connect()
  try {
    await client.query(`
      CREATE TABLE IF NOT EXISTS _applied_migrations (
        filename   TEXT        PRIMARY KEY,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
      )
    `)

    const { rows: applied } = await client.query<{ filename: string }>(
      'SELECT filename FROM _applied_migrations',
    )
    const seen = new Set(applied.map((r) => r.filename))
    const pending = files.filter((name) => !seen.has(name))

    if (pending.length === 0) {
      console.log(`[db:migrate] Up to date (${files.length} applied).`)
      return
    }

    if (DRY_RUN) {
      console.log(`[db:migrate] ${pending.length} pending migration(s):`)
      for (const name of pending) console.log(`  - ${name}`)
      return
    }

    for (const name of pending) {
      const raw = await readFile(join(MIGRATIONS_DIR, name), 'utf8')
      console.log(`[db:migrate] Applying ${name}…`)

      const noTx = NO_TRANSACTION_MARKER.test(raw)
      // Strip a file's own outer BEGIN/COMMIT so we can wrap the
      // whole apply + bookkeeping in a single transaction. Allows
      // any number of `--` comment lines and blank lines before the
      // BEGIN — `0001_better_auth_initial.sql` opens with a 13-line
      // preamble, so a simple `^\s*BEGIN` would have left BEGIN in
      // place and produced a nested-transaction warning followed by
      // a "no transaction in progress" failure on our COMMIT.
      const sql = raw
        .replace(/^(?:[ \t]*(?:--[^\n]*)?\n)*[ \t]*BEGIN[ \t]*;[ \t]*\n?/i, '')
        .replace(/\n?[ \t]*COMMIT[ \t]*;[ \t]*(?:\n[ \t]*(?:--[^\n]*)?)*\s*$/i, '')

      if (noTx) {
        // Caller opted out (e.g. `CREATE INDEX CONCURRENTLY`). Run
        // the body and the bookkeeping row outside a transaction.
        // If the body fails, the bookkeeping row never lands and the
        // next run will re-apply from the top.
        await client.query(sql)
        await client.query('INSERT INTO _applied_migrations (filename) VALUES ($1)', [name])
        continue
      }

      await client.query('BEGIN')
      try {
        await client.query(sql)
        await client.query('INSERT INTO _applied_migrations (filename) VALUES ($1)', [name])
        await client.query('COMMIT')
      } catch (err) {
        // Roll back in its own try/catch so a failing ROLLBACK
        // (e.g. connection already dropped) doesn't shadow the
        // original error the user actually needs to see.
        try {
          await client.query('ROLLBACK')
        } catch (rollbackErr) {
          console.error(`[db:migrate] ROLLBACK after failure also failed:`, rollbackErr)
        }
        throw err
      }
    }
    console.log(`[db:migrate] Done (${pending.length} applied).`)
  } finally {
    await client.end()
  }
}

main().catch((err) => {
  console.error('[db:migrate] Failed:', err)
  process.exit(1)
})
