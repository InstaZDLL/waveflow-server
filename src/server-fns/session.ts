// Server-side session lookup, exposed as a TanStack server function
// so route `beforeLoad` hooks can gate on auth without spinning the
// component up first. Resolves the session from the request cookies
// via Better Auth, returns just the bits we need (id / email / name)
// so the wire payload is minimal — and the rest of `auth.api.getSession`
// (raw session row, expiresAt, etc.) stays server-side.

import { createServerFn } from '@tanstack/react-start'
import { getRequestHeaders } from '@tanstack/react-start/server'
import { auth } from '@/lib/auth'

export interface SessionSummary {
  id: string
  email: string
  name: string
}

export const getCurrentSession = createServerFn({ method: 'GET' }).handler(
  async (): Promise<SessionSummary | null> => {
    const headers = getRequestHeaders()
    const session = await auth.api.getSession({
      headers: new Headers(headers as HeadersInit),
    })
    if (!session?.user) return null
    return {
      id: session.user.id,
      email: session.user.email,
      name: session.user.name,
    }
  },
)
