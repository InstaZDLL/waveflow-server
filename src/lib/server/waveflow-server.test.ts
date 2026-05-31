// Unit tests for the waveflow-server fetcher. The helper is pure
// modulo `process.env.WAVEFLOW_SERVER_URL` + the global `fetch`, so
// we mock both and assert the wire-level behavior.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { waveflowFetch, WaveflowServerError } from './waveflow-server'

const ORIGINAL_ENV = process.env.WAVEFLOW_SERVER_URL
const fetchMock = vi.fn<typeof fetch>()

beforeEach(() => {
  fetchMock.mockReset()
  vi.stubGlobal('fetch', fetchMock)
  process.env.WAVEFLOW_SERVER_URL = 'http://server.example:4000'
})

afterEach(() => {
  vi.unstubAllGlobals()
  if (ORIGINAL_ENV === undefined) delete process.env.WAVEFLOW_SERVER_URL
  else process.env.WAVEFLOW_SERVER_URL = ORIGINAL_ENV
})

describe('waveflowFetch', () => {
  it('throws when WAVEFLOW_SERVER_URL is missing', async () => {
    delete process.env.WAVEFLOW_SERVER_URL
    await expect(waveflowFetch('/api/v1/profiles', { token: 't' })).rejects.toThrow(
      /WAVEFLOW_SERVER_URL/,
    )
  })

  it('attaches Authorization Bearer + parses JSON', async () => {
    const body = [{ id: 1, name: 'Default' }]
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    )

    const result = await waveflowFetch<typeof body>('/api/v1/profiles', { token: 'jwt-abc' })

    expect(result).toEqual(body)
    expect(fetchMock).toHaveBeenCalledOnce()
    const [url, init] = fetchMock.mock.calls[0]!
    expect(url).toBe('http://server.example:4000/api/v1/profiles')
    expect(init?.method).toBe('GET')
    expect((init?.headers as Record<string, string>).Authorization).toBe('Bearer jwt-abc')
  })

  it('strips trailing slashes off the base URL', async () => {
    process.env.WAVEFLOW_SERVER_URL = 'http://server.example:4000/'
    fetchMock.mockResolvedValueOnce(new Response('[]', { status: 200 }))

    await waveflowFetch('/api/v1/profiles', { token: 't' })

    expect(fetchMock.mock.calls[0]![0]).toBe('http://server.example:4000/api/v1/profiles')
  })

  it('serializes the body and sets Content-Type when one is supplied', async () => {
    fetchMock.mockResolvedValueOnce(new Response(JSON.stringify({ id: 7 }), { status: 201 }))

    await waveflowFetch('/api/v1/profiles', {
      token: 't',
      method: 'POST',
      body: { name: 'Daisy', color_id: 'sea' },
    })

    const init = fetchMock.mock.calls[0]![1]!
    expect(init.method).toBe('POST')
    expect((init.headers as Record<string, string>)['Content-Type']).toBe('application/json')
    expect(init.body).toBe(JSON.stringify({ name: 'Daisy', color_id: 'sea' }))
  })

  it('throws WaveflowServerError carrying status + body on non-2xx', async () => {
    fetchMock.mockResolvedValueOnce(new Response('list failed', { status: 500 }))

    await expect(waveflowFetch('/api/v1/profiles', { token: 't' })).rejects.toMatchObject({
      name: 'WaveflowServerError',
      status: 500,
      message: 'list failed',
    })
  })

  it('returns undefined on 204', async () => {
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }))
    await expect(
      waveflowFetch('/api/v1/profiles/7', { token: 't', method: 'DELETE' }),
    ).resolves.toBeUndefined()
  })

  it('returns undefined when a 200 has an empty body', async () => {
    fetchMock.mockResolvedValueOnce(new Response('', { status: 200 }))
    await expect(waveflowFetch('/api/v1/profiles', { token: 't' })).resolves.toBeUndefined()
  })

  it('exposes WaveflowServerError as a regular Error subclass', () => {
    const e = new WaveflowServerError(401, 'nope')
    expect(e).toBeInstanceOf(Error)
    expect(e.status).toBe(401)
    expect(e.message).toBe('nope')
  })
})
