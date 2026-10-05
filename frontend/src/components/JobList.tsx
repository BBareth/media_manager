import { Job } from "../api";
import { useNow } from "../hooks/useNow";
import ConfirmButton from "./ConfirmButton";
import Icon from "./Icon";
import JobCard from "./JobCard";

interface Props {
  title: string;
  jobs: Job[];
  emptyTitle: string;
  emptyHint: string;
  onDelete: (ids: string[]) => void;
}

const ORDER: Record<Job["status"], number> = { running: 0, queued: 1, completed: 2, failed: 2 };

export default function JobList({ title, jobs, emptyTitle, emptyHint, onDelete }: Props) {
  const active = jobs.some((j) => j.status === "running" || j.status === "queued");
  // Tick fast while something is moving, so the estimate counts down; slowly otherwise, just
  // enough to keep "3 min ago" honest.
  const now = useNow(active ? 1000 : 30000);
  const finished = jobs.filter((j) => j.status === "completed" || j.status === "failed");
  // Work in progress first, then the rest newest first (the server's order).
  const sorted = [...jobs].sort((a, b) => ORDER[a.status] - ORDER[b.status]);

  return (
    <section className="jobs-section" aria-label={title}>
      <div className="section-head">
        <h2 className="section-title">
          {title}
          {jobs.length > 0 && <span className="section-count">{jobs.length}</span>}
        </h2>
        {finished.length > 0 && (
          <ConfirmButton
            className="btn btn-ghost btn-sm"
            confirmLabel={`Delete ${finished.length} finished?`}
            title="Remove every finished and failed job here, and their files"
            onConfirm={() => onDelete(finished.map((j) => j.id))}
          >
            Clear finished
          </ConfirmButton>
        )}
      </div>

      {jobs.length === 0 ? (
        <div className="empty">
          <Icon name="inbox" size={28} />
          <p className="empty-title">{emptyTitle}</p>
          <p className="empty-hint">{emptyHint}</p>
        </div>
      ) : (
        <ul className="jobs">
          {sorted.map((job) => (
            <JobCard key={job.id} job={job} now={now} onDelete={(id) => onDelete([id])} />
          ))}
        </ul>
      )}
    </section>
  );
}
