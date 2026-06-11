# web/CLAUDE.md

Half-specific guidance for the TanStack Start app under `web/`. Pairs with the monorepo root [`CLAUDE.md`](../CLAUDE.md), which covers cross-cutting rules (auth boundary, sync wire shape, repo hygiene).

## Stack at a glance

- **React 19** + **TypeScript** + **Vite 8** + **Nitro** (server runtime) via **TanStack Start**.
- **Better Auth** for sessions + JWKS. The `jwt()` plugin issues ES256 tokens the Rust server verifies against `/api/auth/jwks`.
- **Kysely** + **pg** for Better Auth's tables (separate database from the server's, see root README).
- **Tailwind v4** with the `@theme inline` block remapping accent / surface utilities to the OKLCH variables declared by `@waveflow/design-tokens`.
- **Vitest** + **@testing-library** for unit + behaviour tests.
- **bun** as the package manager.

## Commands (run from `web/`)

```bash
bun install
bun run dev                  # Vite dev server on :3000
bun run typecheck            # tsc --noEmit (run after a build so routeTree.gen.ts exists)
bun run lint                 # eslint
bun run format               # prettier --write
bun run build                # Vite + Nitro production build
bun run test                 # vitest run
bun run db:migrate           # idempotent Better Auth migration runner
bun run db:migrate --dry-run # list pending migrations without applying
```

## Architecture

- **Routes** live under `src/routes/`. File-based per TanStack Router conventions — `_authed.*` is the authenticated layout, public routes (`sign-in`, `sign-up`, `/`) sit alongside. A route file that ends in `.test.tsx` is treated as a test and excluded from the route tree by the `-` prefix convention.
- **Server functions** (`src/server-fns/`) run on the Nitro runtime. The dev convention is: a server-fn that needs to call the Rust server mints a fresh JWT off the active Better Auth session and forwards the request with `Authorization: Bearer <jwt>`. The browser never sees `WAVEFLOW_SERVER_URL` — sidesteps CORS plumbing.
- **Auth glue:** [`src/lib/auth.ts`](src/lib/auth.ts) configures Better Auth + the `jwt()` plugin. JWKS lives at `/api/auth/jwks`, token mint at `/api/auth/token` (15-min TTL). First request after sign-in lazy-provisions the user in `waveflow-server`'s `users` table via `find_or_provision_by_external_id`.
- **`@waveflow/design-tokens`** workspace package: OKLCH accent palettes + 14 theme presets + `applyTheme` helper. Ported from the desktop's `src/lib/themes.ts` so a Lavender preset on either side tints to the exact same violet. Live as a workspace dep (`"@waveflow/design-tokens": "workspace:*"`).

## Conventions

- **TanStack Router file routing.** `_authed.profiles.$profileId.libraries.$libraryId.tsx` reads as `/profiles/{profileId}/libraries/{libraryId}` with the `_authed` parent layout enforcing authentication. Use `beforeLoad` for early auth checks, `loader` for prefetch.
- **Server-fn defensive symmetry.** A handler that throws on the server side must reject the same way on the client (validate args at both ends — don't trust the harness).
- **No client-side server URL.** The Rust server URL is a Nitro-side env (`WAVEFLOW_SERVER_URL`). Never expose it to the browser bundle.
- **Tests + routing.** Route files (`.tsx` directly in `src/routes/`) need a `Route = createFileRoute(...)` export. Test files in `src/routes/` must end in `.test.tsx` so the router plugin's `-` prefix convention excludes them from the route tree.

## Better Auth schema

The canonical schema is documented at <https://www.better-auth.com/docs/concepts/database>. The migrations in `db/migrations/` are hand-written to match — keeping them in-tree means a contributor can read the schema without a running DB. **Never edit an applied migration** (same rule the Rust sqlx setup enforces).

## License

[AGPL-3.0-only](../LICENSE). The web client is part of the SaaS-hosted backend story — same licence as `waveflow-server`. Forks of the hosted product must publish their client-side modifications too.

## Contributing

See the monorepo-level [CONTRIBUTING.md](../CONTRIBUTING.md). Conventional commits enforced locally via husky + commitlint (`commit-msg` hook). Header ≤ 100 chars, kebab-case scopes, lowercase subject. DCO `Signed-off-by:` trailer required on every commit (`git commit -s`).
