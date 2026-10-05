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

export interface ServerInfo {
  retention_secs: number;
  max_upload_bytes: number;
  max_encodes: number;
  max_downloads: number;
}

const API = "/api";

export const EVENTS_URL = `${API}/events`;

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

export interface UploadProgress {
  loaded: number;
  total: number;
}

export interface UploadOptions {
  onProgress?: (p: UploadProgress) => void;
  signal?: AbortSignal;
}

/// fetch() cannot report upload progress, and these uploads can be gigabytes, so the multipart
/// endpoints go through XMLHttpRequest.
function upload(path: string, form: FormData, opts: UploadOptions = {}): Promise<Job> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", `${API}${path}`);
    xhr.responseType = "text";

    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) opts.onProgress?.({ loaded: e.loaded, total: e.total });
    };
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          resolve(JSON.parse(xhr.responseText) as Job);
        } catch {
          reject(new Error("The server sent an unreadable reply."));
        }
      } else {
        reject(new Error(xhr.responseText || `${xhr.status} ${xhr.statusText}`));
      }
    };
    xhr.onerror = () => reject(new Error("The upload failed — check the connection and try again."));
    xhr.onabort = () => reject(new DOMException("Upload cancelled", "AbortError"));

    if (opts.signal) {
      if (opts.signal.aborted) {
        reject(new DOMException("Upload cancelled", "AbortError"));
        return;
      }
      opts.signal.addEventListener("abort", () => xhr.abort(), { once: true });
    }
    xhr.send(form);
  });
}

// Small fields go first: the server validates them as they arrive, so a bad value is
// rejected before the file is streamed.

export function startTranscode(
  file: File,
  format: string,
  combineAudio: boolean,
  opts?: UploadOptions
): Promise<Job> {
  const form = new FormData();
  form.append("format", format);
  form.append("combine_audio", combineAudio ? "true" : "false");
  form.append("file", file);
  return upload("/transcode", form, opts);
}

export function startCompress(
  file: File,
  targetMb: number,
  combineAudio: boolean,
  opts?: UploadOptions
): Promise<Job> {
  const form = new FormData();
  form.append("target_mb", String(targetMb));
  form.append("combine_audio", combineAudio ? "true" : "false");
  form.append("file", file);
  return upload("/compress", form, opts);
}

export async function listJobs(): Promise<Job[]> {
  const res = await fetch(`${API}/jobs`);
  if (!res.ok) return asError(res);
  return res.json();
}

export async function getInfo(): Promise<ServerInfo> {
  const res = await fetch(`${API}/info`);
  if (!res.ok) return asError(res);
  return res.json();
}

export async function deleteJob(id: string): Promise<void> {
  const res = await fetch(`${API}/jobs/${encodeURIComponent(id)}`, { method: "DELETE" });
  if (!res.ok && res.status !== 404) return asError(res);
}

export function fileUrl(id: string, inline = false): string {
  return `${API}/jobs/${encodeURIComponent(id)}/file${inline ? "?inline=true" : ""}`;
}

export function isAbort(err: unknown): boolean {
  return err instanceof DOMException && err.name === "AbortError";
}

export function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
