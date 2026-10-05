import { useEffect, useState } from "react";

/// The current time, refreshed every `intervalMs`, so relative times ("3 min ago") keep moving
/// while nothing else on the page changes.
export function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}
