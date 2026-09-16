export type JobKind = "download" | "transcode" | "compress";
export type JobStatus = "queued" | "running" | "completed" | "failed";

export interface Job {
  id: string;
  kind: JobKind;
  status: JobStatus;
  progress: number;
  title: string;
  detail: string;
  output_name?: string | null;
  output_size?: number | null;
  error?: string | null;
  created_at: number;
}

const API = "/api";

async function asError(res: Response): Promise<never> {
  let message = `${res.status} ${res.statusText}`;
  try {
    const text = await res.text();
    if (text) message = text;
  } catch {
    /* ignore */
  }
  throw new Error(message);
}

export interface DownloadParams {
  url: string;
  format: string;
  quality: string;
}

export async function startDownload(params: DownloadParams): Promise<Job> {
  const res = await fetch(`${API}/downloads`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(params),
  });
  if (!res.ok) return asError(res);
  return res.json();
}

export async function startTranscode(
  file: File,
  format: string,
  combineAudio: boolean
): Promise<Job> {
  const form = new FormData();
  form.append("format", format);
  form.append("combine_audio", combineAudio ? "true" : "false");
  form.append("file", file);
  const res = await fetch(`${API}/transcode`, { method: "POST", body: form });
  if (!res.ok) return asError(res);
  return res.json();
}

export async function startCompress(
  file: File,
  targetMb: number,
  combineAudio: boolean
): Promise<Job> {
  const form = new FormData();
  form.append("target_mb", String(targetMb));
  form.append("combine_audio", combineAudio ? "true" : "false");
  form.append("file", file);
  const res = await fetch(`${API}/compress`, { method: "POST", body: form });
  if (!res.ok) return asError(res);
  return res.json();
}

export async function listJobs(): Promise<Job[]> {
  const res = await fetch(`${API}/jobs`);
  if (!res.ok) return asError(res);
  return res.json();
}

export async function deleteJob(id: string): Promise<void> {
  const res = await fetch(`${API}/jobs/${id}`, { method: "DELETE" });
  if (!res.ok && res.status !== 404) return asError(res);
}

export function fileUrl(id: string): string {
  return `${API}/jobs/${id}/file`;
}
