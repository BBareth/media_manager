import { Job, deleteJob, fileUrl } from "../api";
import { formatBytes, timeAgo } from "../format";

interface Props {
  jobs: Job[];
  onChanged: () => void;
}

const STATUS_LABEL: Record<Job["status"], string> = {
  queued: "Queued",
  running: "Working",
  completed: "Ready",
  failed: "Failed",
};

export default function JobList({ jobs, onChanged }: Props) {
  if (jobs.length === 0) {
    return <p className="empty">No jobs yet.</p>;
  }

  async function handleDelete(id: string) {
    await deleteJob(id);
    onChanged();
  }

  function handleDownload(job: Job) {
    // Trigger a browser download of the finished file.
    const a = document.createElement("a");
    a.href = fileUrl(job.id);
    a.download = job.output_name ?? "download";
    document.body.appendChild(a);
    a.click();
    a.remove();
  }

  return (
    <ul className="jobs">
      {jobs.map((job) => (
        <li key={job.id} className={`job job-${job.status}`}>
          <div className="job-main">
            <div className="job-title" title={job.title}>
              {job.title}
            </div>
            <div className="job-meta">
              <span className={`badge badge-${job.status}`}>{STATUS_LABEL[job.status]}</span>
              <span>{job.detail}</span>
              {job.output_size ? <span>{formatBytes(job.output_size)}</span> : null}
              <span className="muted">{timeAgo(job.created_at)}</span>
            </div>

            {(job.status === "running" || job.status === "queued") && (
              <div className="progress">
                <div className="progress-bar" style={{ width: `${job.progress}%` }} />
                <span className="progress-text">{Math.round(job.progress)}%</span>
              </div>
            )}

            {job.status === "failed" && job.error && (
              <pre className="job-error">{job.error}</pre>
            )}
          </div>

          <div className="job-actions">
            {job.status === "completed" && (
              <button className="btn btn-primary" onClick={() => handleDownload(job)}>
                Download
              </button>
            )}
            <button className="btn btn-ghost" onClick={() => handleDelete(job.id)}>
              Delete
            </button>
          </div>
        </li>
      ))}
    </ul>
  );
}
