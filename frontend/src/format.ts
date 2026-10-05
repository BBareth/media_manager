export function formatBytes(bytes?: number | null): string {
  if (bytes == null) return "";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i++;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[i]}`;
}

export function timeAgo(unixSeconds: number, nowMs = Date.now()): string {
  const seconds = Math.max(0, Math.floor(nowMs / 1000 - unixSeconds));
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.floor(hours / 24);
  return `${days} d ago`;
}

/// "45 s", "3 min", "1 h 20 min" — for estimates, where false precision reads as a promise.
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "";
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))} s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours < 48) return rest ? `${hours} h ${rest} min` : `${hours} h`;
  return `${Math.round(hours / 24)} days`;
}

/// "24 hours", "30 minutes", "7 days" — for settings, where spelled-out units read better.
export function formatPeriod(seconds: number): string {
  const plural = (n: number, unit: string) => `${n} ${unit}${n === 1 ? "" : "s"}`;
  if (seconds < 3600) return plural(Math.max(1, Math.round(seconds / 60)), "minute");
  if (seconds < 48 * 3600) return plural(Math.round(seconds / 3600), "hour");
  return plural(Math.round(seconds / 86400), "day");
}

/// Format names as people write them.
export function formatLabel(ext: string): string {
  return ({ webm: "WebM", opus: "Opus" } as Record<string, string>)[ext] ?? ext.toUpperCase();
}

export function formatSpeed(bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) return "";
  return `${formatBytes(bytesPerSecond)}/s`;
}

export type PreviewKind = "video" | "audio" | "image";

const PREVIEW: Record<string, PreviewKind> = {
  mp4: "video",
  m4v: "video",
  webm: "video",
  mov: "video",
  mp3: "audio",
  m4a: "audio",
  aac: "audio",
  ogg: "audio",
  opus: "audio",
  wav: "audio",
  flac: "audio",
  jpg: "image",
  jpeg: "image",
  png: "image",
  webp: "image",
  gif: "image",
};

/// What a browser can reasonably show inline, judged by extension. MKV and AVI are left out:
/// most browsers cannot play them, and a broken player is worse than no button.
export function previewKind(name?: string | null): PreviewKind | null {
  const ext = name?.split(".").pop()?.toLowerCase() ?? "";
  return PREVIEW[ext] ?? null;
}

/// A URL is long and mostly noise; the host and path are what identify it at a glance.
export function shortTitle(title: string): string {
  try {
    const u = new URL(title);
    if (u.protocol === "http:" || u.protocol === "https:") {
      const path = decodeURIComponent(u.pathname + u.search).replace(/\/$/, "");
      return `${u.host.replace(/^www\./, "")}${path}`;
    }
  } catch {
    /* not a URL */
  }
  return title;
}

/// The line of a tool's output most likely to explain a failure: the last one that says
/// "error", or failing that the last non-empty line.
export function errorSummary(error: string): string {
  const lines = error
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  if (lines.length <= 1) return lines[0] ?? "";
  const flagged = [...lines].reverse().find((l) => /error|invalid|not available|unsupported/i.test(l));
  return (flagged ?? lines[lines.length - 1]).replace(/^ERROR:\s*/i, "");
}
