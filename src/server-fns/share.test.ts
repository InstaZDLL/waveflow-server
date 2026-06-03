import { describe, expect, it } from 'vitest'

// The token validator is the only boundary between an untrusted
// caller and a path interpolated into the upstream URL — same
// concern as `asPathId` for numeric ids. Pin the accept / reject
// sets so a future change to the alphabet / length range surfaces
// here before it surfaces on the network.

import { isWellShapedToken } from './share'

describe('isWellShapedToken', () => {
  it.each([
    ['typical 32-char token', 'abcdefghijklmnopqrstuvwxyz012345'],
    ['mixed case + digits', 'ABCdef123ABCdef123ABCdef123ABCde'],
    ['minimum length 8', 'ABCdef12'],
  ])('accepts %s', (_label, value) => {
    expect(isWellShapedToken(value)).toBe(true)
  })

  it.each([
    ['empty string', ''],
    ['too short', 'abc'],
    ['slash (path traversal)', '../../etc/passwd'],
    ['dot (relative path)', 'ab.cd'],
    ['percent-encoded', 'abc%2Fdef'],
    ['query string', 'abc?def=1'],
    ['hash', 'abc#def'],
    ['hyphen', 'abcd-efgh'],
    ['number primitive', 42],
    ['null', null],
    ['undefined', undefined],
    ['object', { token: 'abc' }],
  ])('rejects %s', (_label, value) => {
    expect(isWellShapedToken(value)).toBe(false)
  })
})
