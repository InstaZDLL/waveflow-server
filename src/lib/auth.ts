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
import { jwt } from 'better-auth/plugins'
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

// Social providers (OAuth) — opt-in per provider via env vars.
// A provider whose credentials are absent stays unconfigured: the
// sign-in / sign-up routes hide its button (see
// `getEnabledProviders` in `server-fns/providers.ts`) and Better
// Auth never advertises the endpoint, so a crafted POST against
// `/api/auth/sign-in/social` with `provider: 'google'` 404s
// cleanly instead of throwing midway through the OAuth dance.
//
// Apple's `clientSecret` is a JWT signed with the Apple Developer
// private key (Better Auth doesn't sign it for us); operators
// generate the JWT out-of-band with `openssl` + the team's
// AuthKey p8 and set the result in `APPLE_CLIENT_SECRET`. See
// `.env.example` for the exact recipe.
const socialProviders: Record<string, { clientId: string; clientSecret: string }> = {}
if (process.env.GOOGLE_CLIENT_ID && process.env.GOOGLE_CLIENT_SECRET) {
  socialProviders.google = {
    clientId: process.env.GOOGLE_CLIENT_ID,
    clientSecret: process.env.GOOGLE_CLIENT_SECRET,
  }
}
if (process.env.APPLE_CLIENT_ID && process.env.APPLE_CLIENT_SECRET) {
  socialProviders.apple = {
    clientId: process.env.APPLE_CLIENT_ID,
    clientSecret: process.env.APPLE_CLIENT_SECRET,
  }
}

export const auth = betterAuth({
  database: {
    db,
    type: 'postgres',
  },
  secret: process.env.BETTER_AUTH_SECRET,
  baseURL: process.env.BETTER_AUTH_URL,
  socialProviders,
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
  plugins: [
    // JWT plugin — mints short-lived bearer tokens that waveflow-server
    // verifies against the JWKS endpoint this plugin also exposes at
    // `/api/auth/jwks`. The browser keeps using cookie-backed sessions
    // for the web app; the JWT is the API-call vehicle for talking to
    // the Rust backend (and, later, the desktop app once it points at
    // this server instead of its embedded SQLite).
    //
    // Algorithm: ES256 (P-256 ECDSA) — small, widely interoperable,
    // and what the `jsonwebtoken` crate on waveflow-server speaks
    // natively. EdDSA / Ed25519 (Better Auth's default) is also
    // supported by jsonwebtoken 10 but is a niche pick that some JWKS
    // tooling still mishandles, so we stick with the boring choice.
    //
    // Audience: defaults to `waveflow-server` so the verifier on the
    // Rust side has a stable string to check the `aud` claim against.
    // Override with `WAVEFLOW_JWT_AUDIENCE` when running multiple
    // backend instances that share this auth server.
    jwt({
      jwks: { keyPairConfig: { alg: 'ES256' } },
      jwt: {
        issuer: process.env.BETTER_AUTH_URL,
        audience: process.env.WAVEFLOW_JWT_AUDIENCE ?? 'waveflow-server',
        expirationTime: '15m',
      },
    }),
  ],
})
