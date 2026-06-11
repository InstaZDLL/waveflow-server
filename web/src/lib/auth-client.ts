// Better Auth React client — the canonical export every component
// that needs `signIn` / `signUp` / `signOut` / `useSession` imports.
//
// The client talks to its own server (relative URLs) by default, so
// no `baseURL` is needed at runtime. Tests + storybook can override
// via the constructor's `baseURL` option if they isolate the auth
// surface.

import { createAuthClient } from 'better-auth/react'

export const authClient = createAuthClient()

// Re-export the React hooks so component files don't need to know
// about the client export path.
export const { useSession, signIn, signUp, signOut } = authClient
