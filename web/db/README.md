# db/

Better Auth's database schema for waveflow-web.

## Schema source of truth

The canonical Better Auth schema is documented at <https://www.better-auth.com/docs/concepts/database>. The migrations in this directory are hand-written to match — keeping them in-tree means a contributor can read the schema without a running DB.

When Better Auth ships a schema change in a new release, bump the migration file name and `ALTER` from there. **Never edit an applied migration** (same rule waveflow-server's sqlx setup enforces — a checksum diff on a previously-applied migration would crash every existing install at boot).

## Apply locally

```bash
# 1. Bring up a Postgres (any version >= 14):
docker run -d --name wf-auth-pg -p 5432:5432 \
    -e POSTGRES_USER=wf \
    -e POSTGRES_PASSWORD=wf \
    -e POSTGRES_DB=waveflow_auth \
    postgres:17

# 2. Apply every migration in order:
for f in db/migrations/*.sql; do
    psql "postgres://wf:wf@localhost:5432/waveflow_auth" -f "$f"
done
```

Or use the bundled wrapper, which reads `DATABASE_URL` from `.env` and applies pending migrations idempotently:

```bash
bun run db:migrate              # apply all pending
bun run db:migrate --dry-run    # list pending without applying
```

Applied filenames are recorded in `_applied_migrations(filename TEXT PRIMARY KEY, applied_at TIMESTAMPTZ)`. The runner strips a file's own outer `BEGIN;` / `COMMIT;` so the schema change and the bookkeeping row commit (or roll back) together. Re-running is a no-op; renaming a file that's already applied creates a fresh apply.

## Schema layout

- `user` — one row per registered user. The `id` (TEXT, UUID) is what gets minted into the JWT `sub` claim and resolved against `waveflow-server`'s `users.external_id`.
- `session` — server-side session rows; the browser cookie carries only the session id, lookups hit this table.
- `account` — per-credential-source link. Email/password auth stores the hashed password here; OAuth providers store their `providerId`/`accountId` pair.
- `verification` — short-lived tokens for email-verification + password-reset flows.
- `jwks` — active + grace-period signing keys for the JWT plugin (1.c.2c). The plugin generates a fresh ES256 key pair on first sign attempt, exposes every non-expired row's public key at `/api/auth/jwks`, and signs new tokens with the latest row.
