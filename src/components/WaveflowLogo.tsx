// WaveFlow brand mark — 5 vertical bars rising then falling, the
// silhouette of a music-equaliser snapshot. Ported from the
// desktop's `assets/logo.svg` but inlined as currentColor so the
// active theme's accent palette drives the tint instead of a
// hardcoded emerald gradient.
//
// Sized via the `size` prop (in px). Default 24 — fits the Header
// chip. A larger size (64+) suits hero sections.

export interface WaveflowLogoProps {
  size?: number
  className?: string
  /** Sets `aria-label`; default `"WaveFlow"`. Pass `null` to mark decorative. */
  label?: string | null
}

export default function WaveflowLogo({
  size = 24,
  className,
  label = 'WaveFlow',
}: WaveflowLogoProps) {
  const ariaProps = label === null ? { 'aria-hidden': true } : { role: 'img', 'aria-label': label }
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 256 256"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      {...ariaProps}
    >
      <rect x="36" y="58" width="24" height="140" rx="12" />
      <rect x="76" y="88" width="24" height="80" rx="12" />
      <rect x="116" y="108" width="24" height="40" rx="12" />
      <rect x="156" y="88" width="24" height="80" rx="12" />
      <rect x="196" y="58" width="24" height="140" rx="12" />
    </svg>
  )
}
