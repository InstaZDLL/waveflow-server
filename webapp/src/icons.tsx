import type { ReactNode } from "react";

export type IconName =
  | "albums"
  | "artists"
  | "search"
  | "heart"
  | "playlists"
  | "queue"
  | "shares"
  | "admin"
  | "logout"
  | "previous"
  | "play"
  | "pause"
  | "next";

const paths: Record<IconName, ReactNode> = {
  albums: (
    <>
      <rect x="3" y="3" width="18" height="18" rx="3" />
      <circle cx="12" cy="12" r="4" />
      <circle cx="12" cy="12" r="1" />
    </>
  ),
  artists: (
    <>
      <circle cx="9" cy="8" r="3" />
      <circle cx="17" cy="9" r="2.5" />
      <path d="M3.5 19c.6-4 2.4-6 5.5-6s5 2 5.5 6M14 14c3.5-.4 5.7 1.2 6.5 4" />
    </>
  ),
  search: (
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="m16 16 4.5 4.5" />
    </>
  ),
  heart: (
    <path d="M20.5 5.8c-2.1-2.2-5.6-1.8-7.5.7L12 7.8l-1-1.3C9.1 4 5.6 3.6 3.5 5.8 1.1 8.3 1.4 12.2 4 14.7L12 22l8-7.3c2.6-2.5 2.9-6.4.5-8.9Z" />
  ),
  playlists: (
    <>
      <path d="M4 6h10M4 11h10M4 16h7" />
      <path d="M18 5v10.5a2.5 2.5 0 1 1-2-2.4V7l5-1.5" />
    </>
  ),
  queue: (
    <>
      <path d="M4 6h12M4 11h12M4 16h8" />
      <path d="m17 15 4 3-4 3Z" />
    </>
  ),
  shares: (
    <>
      <circle cx="18" cy="5" r="2.5" />
      <circle cx="6" cy="12" r="2.5" />
      <circle cx="18" cy="19" r="2.5" />
      <path d="m8.3 10.8 7.4-4.5M8.3 13.2l7.4 4.5" />
    </>
  ),
  admin: (
    <>
      <circle cx="12" cy="8" r="3" />
      <path d="M6 20v-2c0-3.3 2.7-6 6-6s6 2.7 6 6v2M19 4v4M17 6h4" />
    </>
  ),
  logout: (
    <>
      <path d="M10 5H5v14h5M14 8l4 4-4 4M8 12h10" />
    </>
  ),
  previous: (
    <>
      <path d="M6 5v14M18 6l-9 6 9 6Z" />
    </>
  ),
  play: <path d="m8 5 11 7-11 7Z" />,
  pause: (
    <>
      <path d="M8 5v14M16 5v14" />
    </>
  ),
  next: (
    <>
      <path d="M18 5v14M6 6l9 6-9 6Z" />
    </>
  ),
};

export function Icon({
  name,
  size = 20,
  filled = false,
}: {
  name: IconName;
  size?: number;
  /** Fills the glyph instead of outlining it, for on/off pairs like a heart. */
  filled?: boolean;
}) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {paths[name]}
    </svg>
  );
}
