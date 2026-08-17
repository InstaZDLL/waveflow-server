import { useEffect, useState } from "react";

import { artworkUrl } from "./api";

type ArtworkProps = {
  artworkId: string | null;
  title: string;
  className?: string;
};

export function Artwork({ artworkId, title, className = "" }: ArtworkProps) {
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setSrc(null);
    void artworkUrl(artworkId).then((url) => {
      if (!cancelled) setSrc(url);
    });
    return () => {
      cancelled = true;
    };
  }, [artworkId]);

  const classes = `cover ${className}`.trim();
  if (src) return <img className={classes} src={src} alt="" />;
  return (
    <div className={`${classes} cover-fallback`} aria-hidden="true">
      <span>{title.slice(0, 1).toLocaleUpperCase()}</span>
    </div>
  );
}
