// Catch-all route for `/api/auth/**` — delegates to the Better
// Auth handler instance from `src/lib/auth.ts`.
//
// TanStack Start's file-router uses `$.ts` for splat / catch-all
// routes. Every method (GET, POST, PATCH, DELETE) is forwarded to
// Better Auth's single `handler(request)` entry point; Better Auth
// owns the URL → operation mapping internally.

import { createFileRoute } from '@tanstack/react-router'
import { auth } from '@/lib/auth'

export const Route = createFileRoute('/api/auth/$')({
  server: {
    handlers: {
      GET: ({ request }) => auth.handler(request),
      POST: ({ request }) => auth.handler(request),
      PATCH: ({ request }) => auth.handler(request),
      DELETE: ({ request }) => auth.handler(request),
    },
  },
})
