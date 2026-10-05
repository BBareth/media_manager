import { QueueItem } from "../hooks/useUploadQueue";
import { formatBytes, formatDuration, formatSpeed } from "../format";
import Icon from "./Icon";
import ProgressBar from "./ProgressBar";

interface Props {
  items: QueueItem[];
  onDismiss: (key: string) => void;
}

/// Files picked for upload, and how each is getting on.
export default function UploadQueue({ items, onDismiss }: Props) {
  if (items.length === 0) return null;
  return (
    <ul className="upload-queue" aria-label="Uploads">
      {items.map(({ key, file, state }) => (
        <li key={key} className={`upload upload-${state.status}`}>
          <div className="upload-row">
            <Icon name="file" size={16} className="upload-icon" />
            <span className="upload-name" title={file.name}>
              {file.name}
            </span>
            <span className="upload-size">{formatBytes(file.size)}</span>
            {(state.status === "failed" || state.status === "skipped") && (
              <button
                type="button"
                className="icon-btn"
                aria-label={`Dismiss ${file.name}`}
                onClick={() => onDismiss(key)}
              >
                <Icon name="x" size={14} />
              </button>
            )}
          </div>
          {state.status === "uploading" && (
            <ProgressBar
              value={(state.loaded / Math.max(1, state.total)) * 100}
              label={`Uploading ${file.name}`}
              caption={uploadCaption(state.loaded, state.total, state.speed)}
            />
          )}
          {state.status === "waiting" && <p className="upload-note">Waiting to upload</p>}
          {state.status === "skipped" && <p className="upload-note upload-warning">{state.reason}</p>}
          {state.status === "failed" && <p className="upload-note upload-error">{state.error}</p>}
        </li>
      ))}
    </ul>
  );
}

function uploadCaption(loaded: number, total: number, speed: number): string {
  const parts = [`${Math.floor((loaded / Math.max(1, total)) * 100)} %`, `${formatBytes(loaded)} of ${formatBytes(total)}`];
  if (speed > 0) {
    parts.push(formatSpeed(speed));
    if (loaded < total) parts.push(`${formatDuration((total - loaded) / speed)} left`);
  }
  return parts.join(" · ");
}
