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

A `bun run db:migrate` wrapper script lands in 1.c.2b — it'll source `.env` and apply pending migrations idempotently.

## Schema layout

- `user` — one row per registered user. The `id` (TEXT, UUID) is what gets minted into the JWT `sub` claim and resolved against `waveflow-server`'s `users.external_id`.
- `session` — server-side session rows; the browser cookie carries only the session id, lookups hit this table.
- `account` — per-credential-source link. Email/password auth stores the hashed password here; OAuth providers store their `providerId`/`accountId` pair.
- `verification` — short-lived tokens for email-verification + password-reset flows.

The JWT plugin's `jwks` table lands in 1.c.2c when we add the JWKS endpoint.
