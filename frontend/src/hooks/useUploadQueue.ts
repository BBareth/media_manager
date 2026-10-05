import { useCallback, useRef, useState } from "react";
import { Job, UploadOptions, errorMessage, isAbort } from "../api";

export type UploadState =
  | { status: "waiting" }
  | { status: "uploading"; loaded: number; total: number; speed: number }
  | { status: "done" }
  | { status: "failed"; error: string }
  | { status: "cancelled" }
  | { status: "skipped"; reason: string };

export interface QueueItem {
  key: string;
  file: File;
  state: UploadState;
}

export interface QueueEntry {
  file: File;
  /// Set to keep the file in the list without uploading it, saying why.
  skip?: string;
}

/// Upload files one after another, each becoming a job as soon as it lands.
///
/// One at a time on purpose: parallel uploads share the same connection, so they all finish
/// late instead of the first one starting to process early.
export function useUploadQueue(
  start: (file: File, opts: UploadOptions) => Promise<Job>,
  onJob: (job: Job) => void
) {
  const [items, setItems] = useState<QueueItem[]>([]);
  const [busy, setBusy] = useState(false);
  const controller = useRef<AbortController | null>(null);

  const update = useCallback((key: string, state: UploadState) => {
    setItems((prev) => prev.map((i) => (i.key === key ? { ...i, state } : i)));
  }, []);

  const run = useCallback(
    async (entries: QueueEntry[]) => {
      const stamp = Date.now();
      const queue: QueueItem[] = entries.map((e, n) => ({
        key: `${stamp}-${n}`,
        file: e.file,
        state: e.skip ? { status: "skipped", reason: e.skip } : { status: "waiting" },
      }));
      setItems(queue);
      setBusy(true);
      const ctrl = new AbortController();
      controller.current = ctrl;

      for (const item of queue) {
        if (item.state.status === "skipped") continue;
        if (ctrl.signal.aborted) {
          update(item.key, { status: "cancelled" });
          continue;
        }
        const startedAt = performance.now();
        update(item.key, { status: "uploading", loaded: 0, total: item.file.size, speed: 0 });
        try {
          const job = await start(item.file, {
            signal: ctrl.signal,
            onProgress: ({ loaded, total }) => {
              const elapsed = (performance.now() - startedAt) / 1000;
              update(item.key, {
                status: "uploading",
                loaded,
                total,
                speed: elapsed > 0.5 ? loaded / elapsed : 0,
              });
            },
          });
          onJob(job);
          update(item.key, { status: "done" });
        } catch (err) {
          update(item.key, isAbort(err) ? { status: "cancelled" } : { status: "failed", error: errorMessage(err) });
        }
      }

      controller.current = null;
      setBusy(false);
      // Uploaded files now show up as jobs; only what still needs attention stays here.
      setItems((prev) => prev.filter((i) => i.state.status === "failed" || i.state.status === "skipped"));
    },
    [start, onJob, update]
  );

  const cancel = useCallback(() => controller.current?.abort(), []);
  const dismiss = useCallback((key: string) => setItems((prev) => prev.filter((i) => i.key !== key)), []);

  return { items, busy, run, cancel, dismiss };
}
