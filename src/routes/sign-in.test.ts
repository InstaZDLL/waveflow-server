// `safeContinueTarget` is the open-redirect gate for the
// post-sign-in navigate. The naive `startsWith('/desktop-login')`
// guard let `/desktop-login/../admin` slip past — the browser
// normalises that to `/admin` after navigation. These tests pin the
// hardened behavior so a future tweak can't quietly weaken the
// check.

import { describe, expect, it, vi } from 'vitest'

vi.mock('@/lib/auth-client', () => ({ authClient: {} }))
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (config: unknown) => config,
  useNavigate: () => () => undefined,
  Link: () => null,
}))

const { safeContinueTarget } = await import('./sign-in')

describe('safeContinueTarget', () => {
  it.each([
    'undefined input',
    '',
    'plain string with no leading slash',
    '/other',
    '/admin',
    '/desktop-login/../admin',
    '/desktop-login/..',
    '/desktop-login/../../etc/passwd',
    '/desktoplogin-trick',
    '/desktop-login-evil',
    '/desktop-loginXYZ',
    'http://attacker.com/desktop-login',
    'https://attacker.com/desktop-login',
    '//attacker.com/desktop-login',
    'javascript:alert(1)',
    'data:text/html,<script>alert(1)</script>',
  ])('rejects %s → /', (raw) => {
    const input = raw === 'undefined input' ? undefined : raw
    expect(safeContinueTarget(input)).toBe('/')
  })

  it('accepts plain /desktop-login', () => {
    expect(safeContinueTarget('/desktop-login')).toBe('/desktop-login')
  })

  it('preserves the search params the OAuth flow depends on', () => {
    expect(
      safeContinueTarget('/desktop-login?cb=http%3A%2F%2F127.0.0.1%3A49388%2Fcb&state=abc'),
    ).toBe('/desktop-login?cb=http%3A%2F%2F127.0.0.1%3A49388%2Fcb&state=abc')
  })

  it('drops the hash to keep the URL surface tight', () => {
    expect(safeContinueTarget('/desktop-login?state=x#fragment')).toBe('/desktop-login?state=x')
  })

  it('normalises trailing slash + nested path under desktop-login', () => {
    // /desktop-login/foo isn't a real route today, but the guard
    // should still accept the prefix — the route's own validator
    // rejects unknown sub-paths.
    expect(safeContinueTarget('/desktop-login/foo')).toBe('/desktop-login/foo')
  })
})
