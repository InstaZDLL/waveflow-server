// Unit tests for the loopback validator. This is the security-
// critical boundary of `/desktop-login` — every code path the JWT
// flows through (mint → redirect) trusts `parseLoopback` to keep an
// attacker-controlled URL out of the `Location` header. Failure here
// would let a phishing link pivot the user's freshly-minted JWT to
// an arbitrary host.
//
// The desktop-login module transitively imports `@/lib/auth`, which
// throws at import time when `DATABASE_URL` isn't set (so the prod
// boot fails fast instead of crashing per-request). Mock both
// pieces here so the unit test doesn't need an env file.

import { describe, expect, it, vi } from 'vitest'

vi.mock('@/lib/db', () => ({ db: {} }))
vi.mock('@/lib/auth', () => ({ auth: { api: {} } }))
vi.mock('@tanstack/react-start', () => ({
  createServerFn: () => ({
    inputValidator: () => ({ handler: () => () => undefined }),
  }),
}))
vi.mock('@tanstack/react-start/server', () => ({ getRequestHeaders: () => ({}) }))

const { parseLoopback } = await import('./desktop-login')

describe('parseLoopback', () => {
  it.each([
    'http://127.0.0.1:49388/cb',
    'http://localhost:49388/cb',
    'http://127.0.0.1:1024/wf?x=1',
    'http://[::1]:65535/cb',
  ])('accepts loopback URL %s', (raw) => {
    const url = parseLoopback(raw)
    expect(url).not.toBeNull()
    expect(url?.protocol).toBe('http:')
  })

  it.each([
    ['empty string', ''],
    ['not a URL', 'not-a-url'],
    ['https scheme', 'https://127.0.0.1:49388/cb'],
    ['file scheme', 'file:///tmp/cb'],
    ['javascript scheme', 'javascript:alert(1)'],
    ['external host', 'http://attacker.com:49388/cb'],
    ['malicious unicode', 'http://127.0.0.1.attacker.com:49388/cb'],
    ['privileged port', 'http://127.0.0.1:80/cb'],
    ['port zero', 'http://127.0.0.1:0/cb'],
    ['port over 65535', 'http://127.0.0.1:65536/cb'],
    ['no port', 'http://127.0.0.1/cb'],
    ['public IP', 'http://8.8.8.8:49388/cb'],
    ['link-local', 'http://169.254.169.254:49388/cb'],
  ])('rejects %s', (_label, raw) => {
    expect(parseLoopback(raw)).toBeNull()
  })

  it('treats hostname case-insensitively', () => {
    expect(parseLoopback('http://LOCALHOST:49388/cb')).not.toBeNull()
  })
})
