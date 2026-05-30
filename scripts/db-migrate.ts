// Idempotent migration runner for `db/migrations/*.sql`.
//
// Reads `DATABASE_URL` from the environment (Bun auto-loads `.env`
// from the project root) and applies any migration filename not yet
// recorded in `_applied_migrations`. Files that already wrap their
// own `BEGIN;` / `COMMIT;` run as-is; everything else gets wrapped
// in a transaction by the runner so a SQL error rolls back cleanly.
// The bookkeeping `INSERT INTO _applied_migrations` always lives in
// the same transaction as the schema changes — either both land or
// neither does.
//
// Usage:
//   bun run db:migrate              # apply all pending
//   bun run db:migrate --dry-run    # list pending without applying
//
// We deliberately don't track a checksum here (yet): the policy is
// "never edit an applied migration" — same rule as waveflow-server's
// sqlx setup. Filename-only tracking keeps the runner trivial and
// fails loudly if someone tries to re-number a merged file.

import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { Client } from 'pg'

const MIGRATIONS_DIR = join(process.cwd(), 'db', 'migrations')
const DRY_RUN = process.argv.includes('--dry-run')

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
      // Strip a file's own outer BEGIN/COMMIT so we can wrap the
      // whole apply + bookkeeping in a single transaction. Postgres
      // would otherwise treat the inner COMMIT as ending the outer
      // tx and the runner's own COMMIT would then fail with "no
      // transaction in progress". Matches at most one leading BEGIN
      // and one trailing COMMIT — anything fancier (savepoints,
      // multiple BEGINs) is left untouched.
      const sql = raw.replace(/^\s*BEGIN\s*;\s*/i, '').replace(/\s*COMMIT\s*;\s*$/i, '')
      await client.query('BEGIN')
      try {
        await client.query(sql)
        await client.query('INSERT INTO _applied_migrations (filename) VALUES ($1)', [name])
        await client.query('COMMIT')
      } catch (err) {
        await client.query('ROLLBACK')
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
