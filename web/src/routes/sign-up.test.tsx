// Client-side validation tests for the sign-up form. We don't hit
// Better Auth here — the form should refuse to submit when the
// inputs don't pass the locally enforced rules, so the network call
// never happens in those cases. The mocked `signUp.email` asserts
// that: it is never invoked when validation short-circuits, and a
// successful call triggers the post-submit navigate-home.
//
// We mock `@tanstack/react-router` rather than rendering inside the
// real router so the test stays a unit test on the form behavior.

import type { PropsWithChildren } from 'react'
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

// `Route.useLoaderData` is consumed by the component since the
// OAuth providers flag landed — extend the mocked file route so it
// returns a stub instead of `undefined.useLoaderData`. Email-only
// keeps the OAuth section out of the rendered tree, which is what
// the existing assertions on labels + the single "Sign up" button
// expect.
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (config: unknown) => ({
    ...(config as Record<string, unknown>),
    useLoaderData: () => ({ email: true, google: false, apple: false }),
  }),
  useNavigate: () => navigate,
  Link: ({ children, ...rest }: PropsWithChildren<Record<string, unknown>>) => (
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

  it('blocks submit when the display name is empty', async () => {
    // The `name` validation gate runs BEFORE the email regex
    // gate; whitespace-only names must trip "Display name is
    // required." rather than slipping through with a trimmed
    // empty string.
    render(<SignUp />)
    fillForm({ name: '   ', email: 'daisy@example.com', password: 'correct-horse-battery' })
    fireEvent.click(screen.getByRole('button', { name: /sign up/i }))

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/display name is required/i)
    expect(signUpEmail).not.toHaveBeenCalled()
  })

  it('blocks submit when the password is at the upper-bound + 1', async () => {
    // MAX_PASSWORD = 128. 129 chars must trip the upper bound
    // message; 128 itself passes (covered by the success-path
    // test below via mock).
    render(<SignUp />)
    fillForm({
      name: 'Daisy',
      email: 'daisy@example.com',
      password: 'a'.repeat(129),
    })
    fireEvent.click(screen.getByRole('button', { name: /sign up/i }))

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/at most 128 characters/i)
    expect(signUpEmail).not.toHaveBeenCalled()
  })

  it('trims surrounding whitespace from name + email before submit', async () => {
    // The component strips whitespace before handing values to
    // Better Auth; the wire payload must show the trimmed
    // strings, not the user's raw input.
    signUpEmail.mockResolvedValueOnce({ data: { user: { id: 'u_2' } }, error: null })
    render(<SignUp />)
    fillForm({
      name: '  Daisy  ',
      email: '  daisy@example.com  ',
      password: 'correct-horse-battery',
    })
    fireEvent.click(screen.getByRole('button', { name: /sign up/i }))

    await waitFor(() => expect(signUpEmail).toHaveBeenCalled())
    expect(signUpEmail).toHaveBeenCalledWith({
      email: 'daisy@example.com',
      password: 'correct-horse-battery',
      name: 'Daisy',
    })
  })
})
