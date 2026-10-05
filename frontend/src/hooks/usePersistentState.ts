import { useEffect, useState } from "react";

const PREFIX = "media-manager:";

function read<T>(key: string, fallback: T, valid: (v: unknown) => v is T): T {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    if (raw == null) return fallback;
    const parsed: unknown = JSON.parse(raw);
    return valid(parsed) ? parsed : fallback;
  } catch {
    // Private windows and blocked storage throw; a preference is never worth an error.
    return fallback;
  }
}

/// useState that remembers its value in this browser. `valid` guards against values saved by an
/// older version of the page, so a stale preference falls back instead of breaking a form.
export function usePersistentState<T>(
  key: string,
  fallback: T,
  valid: (v: unknown) => v is T
): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(() => read(key, fallback, valid));
  useEffect(() => {
    try {
      localStorage.setItem(PREFIX + key, JSON.stringify(value));
    } catch {
      /* see above */
    }
  }, [key, value]);
  return [value, setValue];
}

export const isString = (v: unknown): v is string => typeof v === "string";
export const isBoolean = (v: unknown): v is boolean => typeof v === "boolean";
export const oneOf =
  <T extends string>(options: readonly T[]) =>
  (v: unknown): v is T =>
    typeof v === "string" && (options as readonly string[]).includes(v);
