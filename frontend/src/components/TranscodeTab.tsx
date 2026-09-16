import { useState } from "react";
import { Job, startTranscode } from "../api";
import FilePicker from "./FilePicker";
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
  const [combineAudio, setCombineAudio] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (!file) {
      setError("Please choose a file to convert.");
      return;
    }
    setBusy(true);
    try {
      await startTranscode(file, format, combineAudio);
      setFile(null);
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
        <div className="field">
          <span>Source file</span>
          <FilePicker
            accept="audio/*,video/*,.avi,.mkv,.mov,.flv,.wmv,.m4v"
            fileNames={file ? [file.name] : []}
            onPick={(files) => setFile(files[0] ?? null)}
          />
        </div>

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

          <label className="field">
            <span>Audio tracks</span>
            <select
              value={combineAudio ? "combine" : "keep"}
              onChange={(e) => setCombineAudio(e.target.value === "combine")}
            >
              <option value="keep">Keep tracks separate</option>
              <option value="combine">Combine into one track</option>
            </select>
          </label>

          <button className="btn btn-primary submit" type="submit" disabled={busy}>
            {busy ? "Uploading…" : "Convert"}
          </button>
        </div>

        {!combineAudio && ["mp3", "opus", "wav", "flac"].includes(format) && (
          <p className="hint">
            {format.toUpperCase()} files hold a single audio track. If the source has several,
            choose "Combine into one track" — or convert to MKV, MP4, M4A or OGG to keep them
            separate.
          </p>
        )}
        {error && <p className="form-error">{error}</p>}
      </form>

      <h2 className="section-title">Conversions</h2>
      <JobList jobs={jobs} onChanged={onChanged} />
    </div>
  );
}
