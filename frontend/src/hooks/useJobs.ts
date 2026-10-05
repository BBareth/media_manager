import { useCallback, useEffect, useState } from "react";
import { EVENTS_URL, Job, deleteJob, listJobs } from "../api";

export type Connection = "connecting" | "live" | "offline";

/// The job list, kept current by server-sent events instead of polling.
export function useJobs() {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [connection, setConnection] = useState<Connection>("connecting");

  useEffect(() => {
    let source: EventSource | null = null;
    let retry: number | undefined;
    let disposed = false;

    const connect = () => {
      source = new EventSource(EVENTS_URL);
      source.addEventListener("jobs", (e) => {
        setJobs(JSON.parse((e as MessageEvent<string>).data) as Job[]);
        setConnection("live");
      });
      source.onerror = () => {
        setConnection("offline");
        // EventSource retries on its own after a dropped connection, but gives up for good if
        // a reconnect attempt gets an HTTP error — a proxy's 502 while the server restarts.
        if (source?.readyState === EventSource.CLOSED && !disposed) {
          retry = window.setTimeout(connect, 3000);
        }
      };
    };

    connect();
    return () => {
      disposed = true;
      source?.close();
      window.clearTimeout(retry);
    };
  }, []);

  /// Show a job the moment the server accepts it, ahead of the event that will confirm it.
  const addJob = useCallback((job: Job) => {
    setJobs((prev) => (prev.some((j) => j.id === job.id) ? prev : [job, ...prev]));
  }, []);

  const removeJobs = useCallback(async (ids: string[]) => {
    const gone = new Set(ids);
    setJobs((prev) => prev.filter((j) => !gone.has(j.id)));
    const results = await Promise.allSettled(ids.map(deleteJob));
    if (results.some((r) => r.status === "rejected")) {
      // Put back whatever the server still has.
      listJobs().then(setJobs, () => {});
    }
  }, []);

  return { jobs, connection, addJob, removeJobs };
}
