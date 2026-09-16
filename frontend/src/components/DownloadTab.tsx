import { useState } from "react";
import { Job, startDownload } from "../api";
import JobList from "./JobList";

interface Props {
  jobs: Job[];
  onChanged: () => void;
}

const VIDEO_FORMATS = ["mp4", "mkv", "webm"];
const AUDIO_FORMATS = ["mp3", "m4a", "opus", "ogg", "wav", "flac"];

const QUALITIES = [
  { value: "best", label: "Best available" },
  { value: "2160", label: "4K (2160p)" },
  { value: "1440", label: "1440p" },
  { value: "1080", label: "1080p" },
  { value: "720", label: "720p" },
  { value: "480", label: "480p" },
  { value: "360", label: "360p" },
];

export default function DownloadTab({ jobs, onChanged }: Props) {
  const [url, setUrl] = useState("");
  const [format, setFormat] = useState("mp4");
  const [quality, setQuality] = useState("best");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isAudio = AUDIO_FORMATS.includes(format);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (!url.trim()) {
      setError("Please paste a video URL.");
      return;
    }
    setBusy(true);
    try {
      await startDownload({ url: url.trim(), format, quality });
      setUrl("");
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
          <span>Video URL</span>
          <input
            type="text"
            placeholder="https://www.youtube.com/watch?v=..."
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            autoFocus
          />
        </label>

        <div className="field-row">
          <label className="field">
            <span>Format</span>
            <select value={format} onChange={(e) => setFormat(e.target.value)}>
              <optgroup label="Video">
                {VIDEO_FORMATS.map((f) => (
                  <option key={f} value={f}>
                    {f.toUpperCase()}
                  </option>
                ))}
              </optgroup>
              <optgroup label="Audio only">
                {AUDIO_FORMATS.map((f) => (
                  <option key={f} value={f}>
                    {f.toUpperCase()}
                  </option>
                ))}
              </optgroup>
            </select>
          </label>

          <label className="field">
            <span>Quality</span>
            <select
              value={quality}
              onChange={(e) => setQuality(e.target.value)}
              disabled={isAudio}
            >
              {QUALITIES.map((q) => (
                <option key={q.value} value={q.value}>
                  {q.label}
                </option>
              ))}
            </select>
          </label>

          <button className="btn btn-primary submit" type="submit" disabled={busy}>
            {busy ? "Starting…" : "Download"}
          </button>
        </div>

        {isAudio && (
          <p className="hint">Audio-only format selected — quality picks the best source audio.</p>
        )}
        {error && <p className="form-error">{error}</p>}
      </form>

      <h2 className="section-title">Downloads</h2>
      <JobList jobs={jobs} onChanged={onChanged} />
    </div>
  );
}
