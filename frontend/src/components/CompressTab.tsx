import { FormEvent, useCallback, useState } from "react";
import { Job, UploadOptions, startCompress } from "../api";
import { formatBytes } from "../format";
import { isBoolean, isString, usePersistentState } from "../hooks/usePersistentState";
import { useUploadQueue } from "../hooks/useUploadQueue";
import FilePicker from "./FilePicker";
import Icon from "./Icon";
import JobList from "./JobList";
import SelectedFiles from "./SelectedFiles";
import UploadQueue from "./UploadQueue";

interface Props {
  jobs: Job[];
  onJob: (job: Job) => void;
  onDelete: (ids: string[]) => void;
}

/// Common upload limits, as one-click targets.
const PRESETS = [8, 10, 25, 50, 100];
const MB = 1024 * 1024;

function parseTarget(value: string): number | null {
  const n = Number(value.replace(",", "."));
  return Number.isFinite(n) && n >= 0.1 && n <= 100000 ? n : null;
}

export default function CompressTab({ jobs, onJob, onDelete }: Props) {
  const [files, setFiles] = useState<File[]>([]);
  const [targetMb, setTargetMb] = usePersistentState("compress.targetMb", "25", isString);
  const [combineAudio, setCombineAudio] = usePersistentState("compress.combineAudio", false, isBoolean);
  const [error, setError] = useState<string | null>(null);

  const target = parseTarget(targetMb);
  const start = useCallback(
    (file: File, opts: UploadOptions) => startCompress(file, target ?? 0, combineAudio, opts),
    [target, combineAudio]
  );
  const queue = useUploadQueue(start, onJob);

  // The server refuses a file that is already small enough; say so before uploading it at all.
  const alreadySmall = (f: File) =>
    target != null && f.size <= target * MB
      ? `Already ${formatBytes(f.size)} — under the ${target} MB target, so it will be skipped.`
      : null;

  async function submit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    if (files.length === 0) {
      setError("Choose at least one video, audio file or image.");
      return;
    }
    if (target == null) {
      setError("Enter a target size between 0.1 and 100000 MB.");
      return;
    }
    if (files.every((f) => alreadySmall(f))) {
      setError(`Every file is already under ${target} MB — nothing to do.`);
      return;
    }
    const picked = files;
    setFiles([]);
    await queue.run(picked.map((file) => ({ file, skip: alreadySmall(file) ?? undefined })));
  }

  return (
    <>
      <form className="card form" onSubmit={submit} noValidate>
        <header className="card-header">
          <h2>Shrink to a size</h2>
          <p>Fit videos, audio and images under an upload limit, keeping as much quality as the size allows.</p>
        </header>

        <FilePicker
          multiple
          disabled={queue.busy}
          label="Choose files to compress"
          hint="Video, audio or images — each is fitted to the same target"
          accept="audio/*,video/*,image/*,.avi,.mkv,.mov,.flv,.wmv,.m4v,.ts,.mts"
          fileNames={files.map((f) => f.name)}
          onPick={(picked) => {
            setFiles(picked);
            setError(null);
          }}
        />
        <SelectedFiles files={files} note={alreadySmall} onRemove={(i) => setFiles((prev) => prev.filter((_, n) => n !== i))} />
        <UploadQueue items={queue.items} onDismiss={queue.dismiss} />

        <fieldset className="field target-field" disabled={queue.busy}>
          <legend className="field-label">Target size per file</legend>
          <div className="target-row">
            <div className="input-group input-suffix">
              <input
                type="text"
                inputMode="decimal"
                aria-label="Target size in megabytes"
                value={targetMb}
                onChange={(e) => setTargetMb(e.target.value)}
                aria-invalid={target == null}
              />
              <span className="suffix">MB</span>
            </div>
            <div className="presets" role="group" aria-label="Common sizes">
              {PRESETS.map((p) => (
                <button
                  key={p}
                  type="button"
                  className={`chip${target === p ? " chip-active" : ""}`}
                  aria-pressed={target === p}
                  onClick={() => setTargetMb(String(p))}
                >
                  {p} MB
                </button>
              ))}
            </div>
          </div>
        </fieldset>

        <label className="checkbox-field">
          <input
            type="checkbox"
            checked={combineAudio}
            disabled={queue.busy}
            onChange={(e) => setCombineAudio(e.target.checked)}
          />
          <span>
            Combine all audio tracks into one
            <span className="muted"> — merges separate game and mic tracks (e.g. ShadowPlay)</span>
          </span>
        </label>

        {error && (
          <p className="form-error" role="alert">
            <Icon name="alert" size={16} />
            {error}
          </p>
        )}

        <div className="form-actions">
          <p className="hint">
            Videos become MP4, scaled down when that looks better than starving the bitrate. Audio becomes AAC. Images shrink
            before their quality drops too far.
          </p>
          {queue.busy ? (
            <button type="button" className="btn btn-secondary" onClick={queue.cancel}>
              Cancel uploads
            </button>
          ) : (
            <button className="btn btn-primary" type="submit">
              <Icon name="compress" size={18} />
              {files.length > 1 ? `Compress ${files.length} files` : "Compress"}
            </button>
          )}
        </div>
      </form>

      <JobList
        title="Compressions"
        jobs={jobs}
        onDelete={onDelete}
        emptyTitle="Nothing compressed yet"
        emptyHint="Choose files above and a size they have to fit."
      />
    </>
  );
}
