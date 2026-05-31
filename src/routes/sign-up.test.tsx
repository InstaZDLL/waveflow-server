// Client-side validation tests for the sign-up form. We don't hit
// Better Auth here — the form should refuse to submit when the
// inputs don't pass the locally enforced rules, so the network call
// never happens in those cases. The mocked `signUp.email` asserts
// that: it is never invoked when validation short-circuits, and a
// successful call triggers the post-submit navigate-home.
//
// We mock `@tanstack/react-router` rather than rendering inside the
// real router so the test stays a unit test on the form behavior.

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'

const signUpEmail = vi.fn()
const navigate = vi.fn()

vi.mock('@/lib/auth-client', () => ({
  authClient: {
    signUp: {
      email: (...args: unknown[]) => signUpEmail(...args),
    },
  },
}))

vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (config: unknown) => config,
  useNavigate: () => navigate,
  Link: ({ children, ...rest }: React.PropsWithChildren<Record<string, unknown>>) => (
    <a {...rest}>{children}</a>
  ),
}))

const { SignUp } = await import('./sign-up')

beforeEach(() => {
  signUpEmail.mockReset()
  navigate.mockReset()
})

function fillForm({ name, email, password }: { name: string; email: string; password: string }) {
  fireEvent.change(screen.getByLabelText(/display name/i), { target: { value: name } })
  fireEvent.change(screen.getByLabelText(/email/i), { target: { value: email } })
  fireEvent.change(screen.getByLabelText(/^password/i), { target: { value: password } })
}

describe('sign-up form', () => {
  it('blocks submit when the password is too short', async () => {
    render(<SignUp />)
    fillForm({ name: 'Daisy', email: 'daisy@example.com', password: 'short' })
    fireEvent.click(screen.getByRole('button', { name: /sign up/i }))

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/at least 12 characters/i)
    expect(signUpEmail).not.toHaveBeenCalled()
  })

  it('blocks submit when the email lacks an @', async () => {
    render(<SignUp />)
    fillForm({ name: 'Daisy', email: 'not-an-email', password: 'correct-horse-battery' })
    fireEvent.click(screen.getByRole('button', { name: /sign up/i }))

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/valid email/i)
    expect(signUpEmail).not.toHaveBeenCalled()
  })

  it('surfaces a server error and stays on the form', async () => {
    signUpEmail.mockResolvedValueOnce({
      data: null,
      error: { message: 'Email already in use' },
    })
    render(<SignUp />)
    fillForm({ name: 'Daisy', email: 'daisy@example.com', password: 'correct-horse-battery' })
    fireEvent.click(screen.getByRole('button', { name: /sign up/i }))

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/email already in use/i)
    expect(navigate).not.toHaveBeenCalled()
  })

  it('navigates home on a successful sign-up', async () => {
    signUpEmail.mockResolvedValueOnce({ data: { user: { id: 'u_1' } }, error: null })
    render(<SignUp />)
    fillForm({ name: 'Daisy', email: 'daisy@example.com', password: 'correct-horse-battery' })
    fireEvent.click(screen.getByRole('button', { name: /sign up/i }))

    await waitFor(() => expect(navigate).toHaveBeenCalledWith({ to: '/' }))
    expect(signUpEmail).toHaveBeenCalledWith({
      email: 'daisy@example.com',
      password: 'correct-horse-battery',
      name: 'Daisy',
    })
  })
})
