import { useEffect, useRef, useState } from "react";
import { Job, fileUrl } from "../api";
import { errorSummary, formatBytes, formatDuration, previewKind, shortTitle, timeAgo } from "../format";
import ConfirmButton from "./ConfirmButton";
import Icon, { IconName } from "./Icon";
import ProgressBar from "./ProgressBar";

interface Props {
  job: Job;
  now: number;
  onDelete: (id: string) => void;
}

const STATUS: Record<Job["status"], { label: string; icon: IconName }> = {
  queued: { label: "Queued", icon: "clock" },
  running: { label: "Working", icon: "convert" },
  completed: { label: "Ready", icon: "check" },
  failed: { label: "Failed", icon: "alert" },
};

export default function JobCard({ job, now, onDelete }: Props) {
  const [previewing, setPreviewing] = useState(false);
  const eta = useEta(job, now);
  const kind = job.status === "completed" ? previewKind(job.output_name) : null;
  const status = STATUS[job.status];
  const name = job.output_name ?? shortTitle(job.title);

  return (
    <li className={`job job-${job.status}`}>
      <div className={`job-icon job-icon-${job.status}`} aria-hidden="true">
        {job.status === "running" ? <span className="spinner" /> : <Icon name={status.icon} size={18} />}
      </div>

      <div className="job-body">
        <div className="job-head">
          <span className="job-title" title={job.title}>
            {shortTitle(job.title)}
          </span>
          <span className={`badge badge-${job.status}`}>{status.label}</span>
        </div>
        <div className="job-meta">
          <span>{job.detail}</span>
          {job.output_size ? <span>{formatBytes(job.output_size)}</span> : null}
          <span>{timeAgo(job.created_at, now)}</span>
        </div>

        {job.status === "running" && (
          <ProgressBar
            value={job.progress}
            label={`Progress of ${shortTitle(job.title)}`}
            indeterminate={job.progress <= 0}
            caption={
              job.progress <= 0
                ? "Starting…"
                : `${Math.floor(job.progress)} %${eta != null ? ` · about ${formatDuration(eta)} left` : ""}`
            }
          />
        )}
        {job.status === "queued" && <p className="job-note">Waiting for a free slot — it starts as soon as one opens.</p>}
        {job.status === "failed" && job.error && <JobError error={job.error} />}
        {previewing && kind && <Preview id={job.id} kind={kind} name={name} />}
      </div>

      <div className="job-actions">
        {job.status === "completed" && (
          <>
            <a className="btn btn-primary btn-sm" href={fileUrl(job.id)} download={job.output_name ?? undefined}>
              <Icon name="download" size={16} />
              Download
            </a>
            {kind && (
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                aria-expanded={previewing}
                onClick={() => setPreviewing((p) => !p)}
              >
                <Icon name={kind === "image" ? "eye" : "play"} size={16} />
                {previewing ? "Hide" : "Preview"}
              </button>
            )}
          </>
        )}
        {job.status === "failed" ? (
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => onDelete(job.id)}>
            Dismiss
          </button>
        ) : (
          <ConfirmButton
            className="btn btn-ghost btn-sm"
            confirmLabel={job.status === "completed" ? "Delete file?" : "Stop it?"}
            title={job.status === "completed" ? "Delete this file from the server" : "Stop this job"}
            onConfirm={() => onDelete(job.id)}
          >
            {job.status === "completed" ? (
              <>
                <Icon name="trash" size={16} />
                <span className="btn-label">Delete</span>
              </>
            ) : (
              "Cancel"
            )}
          </ConfirmButton>
        )}
      </div>
    </li>
  );
}

/// Tool output is long; the line that explains the failure is usually near the end.
function JobError({ error }: { error: string }) {
  const summary = errorSummary(error);
  const hasMore = error.trim() !== summary;
  return (
    <div className="job-error">
      <p>{summary}</p>
      {hasMore && (
        <details>
          <summary>Show full output</summary>
          <pre>{error}</pre>
        </details>
      )}
    </div>
  );
}

function Preview({ id, kind, name }: { id: string; kind: "video" | "audio" | "image"; name: string }) {
  const src = fileUrl(id, true);
  return (
    <div className="preview">
      {kind === "video" && <video src={src} controls preload="metadata" aria-label={`Preview of ${name}`} />}
      {kind === "audio" && <audio src={src} controls preload="metadata" aria-label={`Preview of ${name}`} />}
      {kind === "image" && <img src={src} alt={`Preview of ${name}`} />}
    </div>
  );
}

/// Seconds left, extrapolated from how fast the bar has moved since this page first saw the job
/// running. Held back until there is enough movement to be more than a guess.
function useEta(job: Job, now: number): number | null {
  const start = useRef<{ t: number; p: number } | null>(null);
  const running = job.status === "running";

  useEffect(() => {
    if (!running) start.current = null;
    else if (start.current === null && job.progress > 0) start.current = { t: Date.now(), p: job.progress };
  }, [running, job.progress]);

  const s = start.current;
  if (!running || !s) return null;
  const elapsed = (now - s.t) / 1000;
  const moved = job.progress - s.p;
  if (elapsed < 5 || moved < 2) return null;
  return ((100 - job.progress) / moved) * elapsed;
}
