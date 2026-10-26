// Accounts and libraries as an administrator sees them; one's own keys.

import { queryOptions } from "@tanstack/react-query";
import {
  getLibrarySettings,
  listApiTokens,
  listLibraries,
  listLibraryMembers,
  listOwnApiTokens,
  listUsers,
  readOwnSubsonicCredential,
} from "../index";

export const apiTokensQuery = (username: string, open: boolean) =>
  queryOptions({
    queryKey: ["api-tokens", username],
    queryFn: () => listApiTokens(username),
    // Closed until asked for. This panel is rendered once per account, so
    // loading on mount meant one request per account every time the admin
    // screen opened — for a list almost nobody opens.
    enabled: open,
  });

/**
 * This account's own registered clients.
 *
 * Unconditional, unlike `apiTokensQuery`, which is gated on a disclosure
 * because the administration screen renders one per account. There is exactly
 * one of these and the page it sits on is about it.
 */
export const ownApiTokensQuery = () =>
  queryOptions({
    queryKey: ["own-api-tokens"],
    queryFn: listOwnApiTokens,
  });

/**
 * Whether this account holds a Subsonic credential.
 *
 * One of these, like the clients listing beside it, because the page it sits
 * on is about this account.
 */
export const ownSubsonicCredentialQuery = () =>
  queryOptions({
    queryKey: ["own-subsonic-credential"],
    queryFn: readOwnSubsonicCredential,
  });

export const libraryMembersQuery = (libraryId: string, open: boolean) =>
  queryOptions({
    queryKey: ["library-members", libraryId],
    queryFn: () => listLibraryMembers(libraryId),
    enabled: open,
  });

/**
 * A library as an administrator manages it (RFC-021), asked for only once its
 * panel is open: the admin screen draws one per library.
 */
export const librarySettingsQuery = (libraryId: string, open: boolean) =>
  queryOptions({
    queryKey: ["library-settings", libraryId],
    queryFn: () => getLibrarySettings(libraryId),
    enabled: open,
  });

export const administrationQuery = () =>
  queryOptions({
    queryKey: ["administration"],
    queryFn: () => Promise.all([listLibraries(), listUsers()]),
  });
