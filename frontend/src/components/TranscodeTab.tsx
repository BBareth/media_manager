import { useRef, useState } from "react";
import { Job, startTranscode } from "../api";
import JobList from "./JobList";

interface Props {
  jobs: Job[];
  onChanged: () => void;
}

const VIDEO_FORMATS = ["mp4", "mkv", "mov", "webm", "avi"];
const AUDIO_FORMATS = ["mp3", "m4a", "ogg", "opus", "wav", "flac"];

export default function TranscodeTab({ jobs, onChanged }: Props) {
  const [file, setFile] = useState<File | null>(null);
  const [format, setFormat] = useState("mp4");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (!file) {
      setError("Please choose a file to convert.");
      return;
    }
    setBusy(true);
    try {
      await startTranscode(file, format);
      setFile(null);
      if (inputRef.current) inputRef.current.value = "";
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="tab-panel">
      <form className="card form" onSubmit={submit}>
        <label className="field">
          <span>Source file</span>
          <input
            ref={inputRef}
            type="file"
            accept="audio/*,video/*,.avi,.mkv,.mov,.flv,.wmv,.m4v"
            onChange={(e) => setFile(e.target.files?.[0] ?? null)}
          />
        </label>

        <div className="field-row">
          <label className="field">
            <span>Convert to</span>
            <select value={format} onChange={(e) => setFormat(e.target.value)}>
              <optgroup label="Video">
                {VIDEO_FORMATS.map((f) => (
                  <option key={f} value={f}>
                    {f.toUpperCase()}
                  </option>
                ))}
              </optgroup>
              <optgroup label="Audio">
                {AUDIO_FORMATS.map((f) => (
                  <option key={f} value={f}>
                    {f.toUpperCase()}
                  </option>
                ))}
              </optgroup>
            </select>
          </label>

          <button className="btn btn-primary submit" type="submit" disabled={busy}>
            {busy ? "Uploading…" : "Convert"}
          </button>
        </div>

        {file && (
          <p className="hint">
            Selected: <strong>{file.name}</strong>
          </p>
        )}
        {error && <p className="form-error">{error}</p>}
      </form>

      <h2 className="section-title">Conversions</h2>
      <JobList jobs={jobs} onChanged={onChanged} />
    </div>
  );
}
