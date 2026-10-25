import { useEffect, useState } from "react";

import { localDayKey, watchLocalDay } from "../lib/local-day";

/**
 * The listener's day, kept current: what the mix and statistics queries put
 * in their keys, so that the first render after midnight asks again.
 */
export function useLocalDay(): string {
  const [day, setDay] = useState(localDayKey);
  useEffect(() => watchLocalDay(setDay), []);
  return day;
}
