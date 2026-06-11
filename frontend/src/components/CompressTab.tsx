import { useRef, useState } from "react";
import { Job, startCompress } from "../api";
import { formatBytes } from "../format";
import JobList from "./JobList";

interface Props {
  jobs: Job[];
  onChanged: () => void;
}

export default function CompressTab({ jobs, onChanged }: Props) {
  const [files, setFiles] = useState<File[]>([]);
  const [targetMb, setTargetMb] = useState("50");
  const [busy, setBusy] = useState(false);
  const [uploadIndex, setUploadIndex] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  function onPick(list: FileList | null) {
    setFiles(list ? Array.from(list) : []);
    setError(null);
  }

  function removeFile(index: number) {
    setFiles((prev) => prev.filter((_, i) => i !== index));
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (files.length === 0) {
      setError("Please choose at least one video or image.");
      return;
    }
    const target = Number(targetMb);
    if (!Number.isFinite(target) || target <= 0) {
      setError("Please enter a target size in MB (e.g. 25).");
      return;
    }

    // Queue every file with the same target. Uploads run one at a time so a
    // pile of large videos doesn't saturate the connection; each file becomes
    // its own job as soon as its upload finishes.
    setBusy(true);
    const failures: string[] = [];
    for (let i = 0; i < files.length; i++) {
      setUploadIndex(i + 1);
      try {
        await startCompress(files[i], target);
        onChanged();
      } catch (err) {
        failures.push(`${files[i].name}: ${err instanceof Error ? err.message : String(err)}`);
      }
    }
    setBusy(false);
    setUploadIndex(0);
    setFiles([]);
    if (inputRef.current) inputRef.current.value = "";
    if (failures.length > 0) setError(failures.join("\n"));
  }

  return (
    <div className="tab-panel">
      <form className="card form" onSubmit={submit}>
        <label className="field">
          <span>Videos or images (select multiple to queue them)</span>
          <input
            ref={inputRef}
            type="file"
            multiple
            accept="audio/*,video/*,image/*,.avi,.mkv,.mov,.flv,.wmv,.m4v"
            onChange={(e) => onPick(e.target.files)}
          />
        </label>

        <div className="field-row">
          <label className="field">
            <span>Target size per file (MB)</span>
            <input
              type="number"
              min="0.1"
              step="any"
              value={targetMb}
              onChange={(e) => setTargetMb(e.target.value)}
            />
          </label>

          <button className="btn btn-primary submit" type="submit" disabled={busy}>
            {busy
              ? `Uploading ${uploadIndex}/${files.length}…`
              : files.length > 1
                ? `Compress ${files.length} files`
                : "Compress"}
          </button>
        </div>

        {files.length > 0 && (
          <ul className="file-queue">
            {files.map((f, i) => (
              <li key={`${f.name}-${i}`}>
                <span className="file-queue-name">{f.name}</span>
                <span className="muted">{formatBytes(f.size)}</span>
                {!busy && (
                  <button
                    type="button"
                    className="btn-remove"
                    title="Remove from queue"
                    onClick={() => removeFile(i)}
                  >
                    ✕
                  </button>
                )}
              </li>
            ))}
          </ul>
        )}

        <p className="hint">
          Videos are re-encoded to MP4 with a bitrate chosen to land under the target. Images step
          down in quality (and resolution if needed) until they fit.
        </p>
        {error && <p className="form-error" style={{ whiteSpace: "pre-wrap" }}>{error}</p>}
      </form>

      <h2 className="section-title">Compressions</h2>
      <JobList jobs={jobs} onChanged={onChanged} />
    </div>
  );
}
