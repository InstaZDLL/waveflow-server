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
bun run dev        # Vite dev server on http://localhost:3000
bun run typecheck  # tsc --noEmit
bun run lint       # eslint
bun run format     # prettier --write
bun run build      # production build (Vite + Nitro bundling)
bun run test       # vitest run
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
