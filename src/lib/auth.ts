// Better Auth server instance — the canonical export every server
// handler imports. The catch-all route at
// `src/routes/api/auth/$.ts` delegates every `/api/auth/**` request
// to this instance's `handler`.
//
// Phase 1.c.2a (this file) wires the bare minimum: email/password
// sign-up + sign-in. Phase 1.c.2c adds the JWT plugin so
// waveflow-server can verify Bearer tokens against the JWKS endpoint
// at `/api/auth/jwks`. Phase 1.c.3 wires sign-up to call
// waveflow-server's user-onboarding API so the freshly-minted Better
// Auth `user.id` lands as `users.external_id` on the Rust side.

import { betterAuth } from 'better-auth'
import { db } from './db'

if (!process.env.BETTER_AUTH_SECRET) {
  // Better Auth signs session tokens + JWTs with this secret; a
  // missing value would either crash on first request or — worse —
  // silently fall back to a dev default. Fail loud at boot instead.
  throw new Error(
    'BETTER_AUTH_SECRET is required. Generate one with `openssl rand -base64 32` and put it in .env.',
  )
}

if (!process.env.BETTER_AUTH_URL) {
  // Better Auth uses this URL to construct the issuer claim on JWTs
  // and the redirect URLs for OAuth providers. Must match the
  // public-facing origin of this Nitro instance.
  throw new Error(
    'BETTER_AUTH_URL is required. Set it to the public-facing origin (e.g. http://localhost:3000).',
  )
}

export const auth = betterAuth({
  database: {
    db,
    type: 'postgres',
  },
  secret: process.env.BETTER_AUTH_SECRET,
  baseURL: process.env.BETTER_AUTH_URL,
  emailAndPassword: {
    enabled: true,
    // Email verification stays off for the 1.c.2 transition window.
    // It lands in a follow-up alongside the email-sender plumbing —
    // shipping it now would mean either accepting unverified accounts
    // or stubbing the sender, both fragile.
    requireEmailVerification: false,
    // Better Auth defaults to bcrypt with cost 10. The argon2id
    // plugin is the upgrade path once the user base grows past a few
    // thousand accounts.
    minPasswordLength: 12,
    maxPasswordLength: 128,
  },
  // Session storage — Better Auth stamps a server-side row + a
  // signed cookie. The cookie carries only the session id; lookups
  // hit the `session` table. `expiresIn` is 7 days; `updateAge` is
  // 1 day so a session that's actively used keeps refreshing
  // without forcing a re-login.
  session: {
    expiresIn: 60 * 60 * 24 * 7,
    updateAge: 60 * 60 * 24,
  },
})
