import { Outlet, createFileRoute, redirect } from '@tanstack/react-router'
import { getCurrentSession } from '@/server-fns/session'

/**
 * Pathless layout (`_authed`) that gates every child route on a live
 * Better Auth session. Centralising the auth check here means the
 * child routes (`_authed.profiles`, `_authed.profiles.$profileId`,
 * etc.) don't each repeat the `beforeLoad → redirect` boilerplate —
 * a single guard runs once per navigation.
 *
 * The underscore prefix is the TanStack file-router convention for a
 * pathless segment: this file contributes ZERO to the URL but is the
 * parent of any sibling file whose name also starts with `_authed.`,
 * so children mount at e.g. `/profiles` not `/_authed/profiles`.
 */
export const Route = createFileRoute('/_authed')({
  beforeLoad: async () => {
    const session = await getCurrentSession()
    if (!session) {
      throw redirect({ to: '/sign-in' })
    }
  },
  component: AuthedLayout,
})

function AuthedLayout() {
  return <Outlet />
}
