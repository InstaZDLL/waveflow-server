// AccountCard render + sign-out tests. The `useSession` hook is
// mocked module-scoped so each spec swaps the session shape without
// re-rendering the whole hook stack; `authClient.signOut` is spied
// on so we can assert the click delegates.

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

let session: { user: { name: string | null; email: string; createdAt: Date | null } } | null = null
let isPending = false

const signOut = vi.fn(async () => undefined)
const navigate = vi.fn(async () => undefined)

vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (config: unknown) => config,
  useNavigate: () => navigate,
}))

// Real `useSession` returns `{ data, isPending, error, refetch }`.
// Carrying every key in the mock keeps a future destructure
// (`const { error } = useSession()`) from silently passing the
// suite against `undefined` — the suite would mask a real
// regression where Better Auth sets `error` alongside null `data`.
vi.mock('@/lib/auth-client', () => ({
  useSession: () => ({ data: session, isPending, error: null, refetch: vi.fn() }),
  authClient: {
    signOut: () => signOut(),
  },
}))

// Theme picker pulls in CSS + a theme context — none of which the
// account-card spec needs. Stub it out.
vi.mock('@/components/ThemePicker', () => ({
  ThemePicker: () => null,
}))

const { AccountCard } = await import('./_authed.settings')

describe('AccountCard', () => {
  beforeEach(() => {
    signOut.mockClear()
    navigate.mockClear()
    isPending = false
    session = null
  })

  it('renders a loading placeholder while the session is pending', () => {
    isPending = true
    render(<AccountCard />)
    expect(screen.getByText(/loading account details/i)).toBeTruthy()
  })

  it('renders an "session expired" alert when useSession resolves to null', () => {
    session = null
    isPending = false
    render(<AccountCard />)
    const alert = screen.getByRole('alert')
    expect(alert.textContent).toMatch(/session expired/i)
  })

  it('renders the resolved name + email + member-since + Sign out button', () => {
    session = {
      user: {
        name: 'Alice',
        email: 'alice@example.com',
        createdAt: new Date('2024-03-15T00:00:00Z'),
      },
    }
    render(<AccountCard />)
    expect(screen.getByText('Alice')).toBeTruthy()
    expect(screen.getByText('alice@example.com')).toBeTruthy()
    // `Intl.DateTimeFormat` output depends on the test locale —
    // assert on the year alone so the spec stays stable across
    // CI locales without faking Intl.
    expect(screen.getByText(/2024/)).toBeTruthy()
    expect(screen.getByRole('button', { name: /sign out/i })).toBeTruthy()
  })

  it('renders an "Not set" placeholder when the name is null', () => {
    session = {
      user: { name: null, email: 'alice@example.com', createdAt: null },
    }
    render(<AccountCard />)
    expect(screen.getByText(/not set/i)).toBeTruthy()
    // `createdAt: null` falls back to the dash.
    expect(screen.getByText('—')).toBeTruthy()
  })

  it('calls authClient.signOut + navigates to /sign-in when the button is clicked', async () => {
    session = {
      user: { name: 'Alice', email: 'alice@example.com', createdAt: new Date('2024-03-15') },
    }
    const user = userEvent.setup()
    render(<AccountCard />)
    await user.click(screen.getByRole('button', { name: /sign out/i }))
    expect(signOut).toHaveBeenCalledTimes(1)
    expect(navigate).toHaveBeenCalledWith({ to: '/sign-in' })
  })

  it('still navigates to /sign-in when signOut rejects', async () => {
    session = {
      user: { name: 'Alice', email: 'alice@example.com', createdAt: new Date('2024-03-15') },
    }
    // Same rationale as the Header sign-out: even if the network
    // call fails, we want the user landed somewhere sensible.
    signOut.mockRejectedValueOnce(new Error('network'))
    // Silence the expected console.error so the test output stays
    // clean.
    const consoleErr = vi.spyOn(console, 'error').mockImplementation(() => {})
    const user = userEvent.setup()
    render(<AccountCard />)
    await user.click(screen.getByRole('button', { name: /sign out/i }))
    expect(navigate).toHaveBeenCalledWith({ to: '/sign-in' })
    consoleErr.mockRestore()
  })
})
