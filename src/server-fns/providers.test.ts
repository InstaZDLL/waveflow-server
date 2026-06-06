// Pin the per-provider env contract — a future tweak to which env
// var spells "this provider is enabled" should land here first so
// the sign-in / sign-up UI stays in sync with what Better Auth has
// actually registered.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// Behind `getEnabledProviders` is a TanStack `createServerFn`
// wrapper, which calls into a runtime that needs a request context
// to dispatch. For a unit test we don't care about the wrapper —
// we want to exercise the handler. Stub `createServerFn` so it
// returns whatever the `.handler` callback resolves to, sidestepping
// the request plumbing.
vi.mock('@tanstack/react-start', () => ({
  createServerFn: () => ({
    handler: (fn: (...args: unknown[]) => unknown) => fn,
  }),
}))

const { getEnabledProviders } = await import('./providers')

// Process env state is global, so each test snapshots + restores
// the OAuth vars rather than racing with whatever the developer
// has in `.env`.
const VARS = [
  'GOOGLE_CLIENT_ID',
  'GOOGLE_CLIENT_SECRET',
  'APPLE_CLIENT_ID',
  'APPLE_CLIENT_SECRET',
] as const

let snapshot: Record<(typeof VARS)[number], string | undefined>
beforeEach(() => {
  snapshot = Object.fromEntries(VARS.map((k) => [k, process.env[k]])) as typeof snapshot
  for (const k of VARS) delete process.env[k]
})
afterEach(() => {
  for (const k of VARS) {
    if (snapshot[k] === undefined) delete process.env[k]
    else process.env[k] = snapshot[k]
  }
})

describe('getEnabledProviders', () => {
  it('always reports email as enabled', async () => {
    // Better Auth is always initialised with email/password on, so
    // the flag is wired-on regardless of what the OAuth env says.
    const result = await (getEnabledProviders as unknown as () => Promise<unknown>)()
    expect((result as { email: boolean }).email).toBe(true)
  })

  it('hides google + apple when no env is set', async () => {
    const result = (await (getEnabledProviders as unknown as () => Promise<unknown>)()) as {
      google: boolean
      apple: boolean
    }
    expect(result.google).toBe(false)
    expect(result.apple).toBe(false)
  })

  it('enables google only when BOTH id and secret are set', async () => {
    process.env.GOOGLE_CLIENT_ID = 'fake-id'
    let result = (await (getEnabledProviders as unknown as () => Promise<unknown>)()) as {
      google: boolean
    }
    expect(result.google).toBe(false)
    process.env.GOOGLE_CLIENT_SECRET = 'fake-secret'
    result = (await (getEnabledProviders as unknown as () => Promise<unknown>)()) as {
      google: boolean
    }
    expect(result.google).toBe(true)
  })

  it('enables apple only when BOTH id and secret are set', async () => {
    process.env.APPLE_CLIENT_ID = 'com.example.web'
    let result = (await (getEnabledProviders as unknown as () => Promise<unknown>)()) as {
      apple: boolean
    }
    expect(result.apple).toBe(false)
    process.env.APPLE_CLIENT_SECRET = 'apple-jwt-stub'
    result = (await (getEnabledProviders as unknown as () => Promise<unknown>)()) as {
      apple: boolean
    }
    expect(result.apple).toBe(true)
  })

  it('treats an empty string the same as unset', async () => {
    // Some shells export `Foo=""` which surfaces as `Ok("")` —
    // hiding the provider keeps a misconfigured deploy from
    // exposing an unusable button.
    process.env.GOOGLE_CLIENT_ID = ''
    process.env.GOOGLE_CLIENT_SECRET = ''
    const result = (await (getEnabledProviders as unknown as () => Promise<unknown>)()) as {
      google: boolean
    }
    expect(result.google).toBe(false)
  })
})
