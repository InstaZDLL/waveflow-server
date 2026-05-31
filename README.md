# waveflow-web

WaveFlow's web client — React + [TanStack Start](https://tanstack.com/start) frontend for [`waveflow-server`](https://github.com/InstaZDLL/waveflow-server).

Companion to the desktop app at [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow). Architecture intent and the long-term roadmap live in [RFC-001](https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-001-waveflow-server.md).

## Stack

- **React 19** + **TypeScript** — UI layer
- **TanStack Start** (Vite + Nitro) — file-based routing, SSR, server functions, API routes
- **Tailwind CSS 4** — styling
- **Vitest** + **Testing Library** — test runner

## Development

```bash
bun install        # one-shot; runs `prepare: husky` to install the commit-msg hook
cp .env.example .env # then fill in DATABASE_URL, BETTER_AUTH_SECRET (openssl rand -base64 32), and BETTER_AUTH_URL
bun run dev        # Vite dev server on http://localhost:3000
bun run typecheck  # tsc --noEmit (run after a build so routeTree.gen.ts exists)
bun run lint       # eslint
bun run format     # prettier --write
bun run build      # production build (Vite + Nitro bundling)
bun run test       # vitest run
```

### Database

Better Auth's tables live in Postgres. See [`db/README.md`](db/README.md) for the schema + bootstrap steps. A local dev DB via Docker takes ~10 seconds to stand up.

```bash
bun run db:migrate              # apply pending migrations
bun run db:migrate --dry-run    # show what would be applied
```

### Wired to `waveflow-server`

The `/profiles` route ([`src/routes/profiles.tsx`](src/routes/profiles.tsx)) calls `waveflow-server`'s `/api/v1/profiles` through a TanStack server function ([`src/server-fns/profiles.ts`](src/server-fns/profiles.ts)) that mints a fresh JWT off the active Better Auth session and forwards the request server-side. The browser never sees `WAVEFLOW_SERVER_URL` and never crosses an origin — sidesteps CORS plumbing on `waveflow-server`.

Run both servers locally:

```bash
# terminal 1 — waveflow-server on :4000
WAVEFLOW_BIND=127.0.0.1:4000 cargo run --manifest-path ../waveflow-server/Cargo.toml

# terminal 2 — waveflow-web on :3000
bun run dev
```

The first authenticated request lazy-provisions the user in `waveflow-server`'s `users` table (no separate onboarding round-trip needed — `waveflow-server` PR #16).

### JWT endpoint (for `waveflow-server`)

The [`jwt()` plugin](src/lib/auth.ts) exposes two endpoints once a session is active:

- `GET /api/auth/jwks` — public key set (ES256). Point `waveflow-server`'s `WAVEFLOW_JWT_JWKS_URL` at this URL.
- `GET /api/auth/token` — mints a fresh bearer token for the current session (15-minute TTL). The web app's fetch wrapper (1.c.3) will call this on demand.

Smoke-test the JWKS endpoint after `bun run dev`:

```bash
curl http://localhost:3000/api/auth/jwks | jq
# => { "keys": [ { "kty": "EC", "crv": "P-256", "alg": "ES256", "kid": "...", "x": "...", "y": "..." } ] }
```

The first request lazy-generates the key pair into the `jwks` table — subsequent requests reuse it. Issuer = `BETTER_AUTH_URL`; audience defaults to `waveflow-server` (override with `WAVEFLOW_JWT_AUDIENCE`).

### Sign up / sign in

Routes:

- [`/sign-up`](src/routes/sign-up.tsx) — create a new account (email + password, 12–128 chars)
- [`/sign-in`](src/routes/sign-in.tsx) — sign back in
- The header auto-swaps to a user chip + sign-out button once a session is active

Smoke-test from the CLI once the dev server is up:

```bash
curl -X POST http://localhost:3000/api/auth/sign-up/email \
    -H 'Content-Type: application/json' \
    -d '{"email":"daisy@example.com","password":"correct-horse-battery","name":"Daisy"}'

curl -X POST http://localhost:3000/api/auth/sign-in/email \
    -H 'Content-Type: application/json' \
    -d '{"email":"daisy@example.com","password":"correct-horse-battery"}'
```

## Deploying

The Nitro build produces a self-contained Node server in `.output/`:

```bash
bun run build
node .output/server/index.mjs
```

For host-specific presets (Vercel, Cloudflare, AWS Lambda, etc.), see [nitro.build/deploy](https://nitro.build/deploy).

## License

[AGPL-3.0-only](LICENSE). The web client is part of the SaaS-hosted backend story — same license as [`waveflow-server`](https://github.com/InstaZDLL/waveflow-server). Forks of the hosted product must publish their client-side modifications too.

The companion desktop app at [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow) is GPL-3.0-only — no network clause needed since it's a local-only player.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Every commit must carry a `Signed-off-by:` trailer (DCO) — `git commit -s` adds it automatically.

Conventional commits enforced locally via husky + commitlint (`commit-msg` hook). Header ≤ 100 chars, kebab-case scopes, lowercase subject.

---

For TanStack-specific reference (file routing, server functions, layouts, data loaders), see the [TanStack documentation](https://tanstack.com/start).
