import { FormEvent, useState } from "react";
import { Job, errorMessage, startDownload } from "../api";
import { formatLabel } from "../format";
import { isString, oneOf, usePersistentState } from "../hooks/usePersistentState";
import ChoiceGroup, { Choice } from "./ChoiceGroup";
import Icon from "./Icon";
import JobList from "./JobList";

interface Props {
  jobs: Job[];
  onJob: (job: Job) => void;
  onDelete: (ids: string[]) => void;
}

type Mode = "video" | "audio";
const MODES: Choice<Mode>[] = [
  { value: "video", label: "Video" },
  { value: "audio", label: "Audio only" },
];

const VIDEO_FORMATS = ["mp4", "mkv", "webm"] as const;
const AUDIO_FORMATS = ["mp3", "m4a", "opus", "ogg", "wav", "flac"] as const;
const FORMAT_HINTS: Record<string, string> = {
  mp4: "Plays everywhere",
  mkv: "Keeps every stream, plays in VLC and most desktop players",
  webm: "Open format, best in browsers",
  mp3: "Plays everywhere",
  m4a: "AAC, smaller than MP3 at the same quality",
  opus: "Smallest files for the quality",
  ogg: "Vorbis",
  wav: "Uncompressed",
  flac: "Lossless",
};

const QUALITIES = [
  { value: "best", label: "Best available" },
  { value: "2160", label: "4K (2160p)" },
  { value: "1440", label: "1440p" },
  { value: "1080", label: "1080p" },
  { value: "720", label: "720p" },
  { value: "480", label: "480p" },
  { value: "360", label: "360p" },
] as const;

const choices = <T extends string>(formats: readonly T[]): Choice<T>[] =>
  formats.map((f) => ({ value: f, label: formatLabel(f), hint: FORMAT_HINTS[f] }));

const canPaste = typeof navigator !== "undefined" && !!navigator.clipboard?.readText && window.isSecureContext;

export default function DownloadTab({ jobs, onJob, onDelete }: Props) {
  const [url, setUrl] = useState("");
  const [mode, setMode] = usePersistentState<Mode>("download.mode", "video", oneOf(["video", "audio"]));
  const [videoFormat, setVideoFormat] = usePersistentState("download.videoFormat", "mp4", oneOf(VIDEO_FORMATS));
  const [audioFormat, setAudioFormat] = usePersistentState("download.audioFormat", "mp3", oneOf(AUDIO_FORMATS));
  const [quality, setQuality] = usePersistentState(
    "download.quality",
    "best",
    (v): v is string => isString(v) && QUALITIES.some((q) => q.value === v)
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const audio = mode === "audio";
  const format = audio ? audioFormat : videoFormat;

  async function submit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    const trimmed = url.trim();
    if (!trimmed) {
      setError("Paste a link to a video or a page that has one.");
      return;
    }
    if (!/^https?:\/\//i.test(trimmed)) {
      setError("The link has to start with http:// or https://.");
      return;
    }
    setBusy(true);
    try {
      onJob(await startDownload({ url: trimmed, format, quality: audio ? "" : quality }));
      setUrl("");
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function paste() {
    try {
      const text = (await navigator.clipboard.readText()).trim();
      if (text) {
        setUrl(text);
        setError(null);
      }
    } catch {
      setError("The browser did not allow reading the clipboard — paste with Ctrl+V instead.");
    }
  }

  return (
    <>
      <form className="card form" onSubmit={submit} noValidate>
        <header className="card-header">
          <h2>Download from a link</h2>
          <p>Video or audio from YouTube and the thousands of other sites yt-dlp supports.</p>
        </header>

        <div className="field">
          <label className="field-label" htmlFor="download-url">
            Link
          </label>
          <div className="input-group">
            <Icon name="link" size={18} className="input-icon" />
            <input
              id="download-url"
              type="url"
              inputMode="url"
              autoComplete="off"
              spellCheck={false}
              placeholder="https://www.youtube.com/watch?v=…"
              value={url}
              onChange={(e) => {
                setUrl(e.target.value);
                if (error) setError(null);
              }}
              aria-invalid={!!error}
              aria-describedby={error ? "download-error" : undefined}
              autoFocus
            />
            {canPaste && !url && (
              <button type="button" className="input-action" onClick={paste}>
                <Icon name="clipboard" size={16} />
                Paste
              </button>
            )}
            {url && (
              <button type="button" className="input-action" aria-label="Clear link" onClick={() => setUrl("")}>
                <Icon name="x" size={16} />
              </button>
            )}
          </div>
        </div>

        <div className="options">
          <ChoiceGroup legend="Get" value={mode} options={MODES} onChange={setMode} variant="segmented" />
          {audio ? (
            <ChoiceGroup legend="Audio format" value={audioFormat} options={choices(AUDIO_FORMATS)} onChange={setAudioFormat} />
          ) : (
            <ChoiceGroup legend="Video format" value={videoFormat} options={choices(VIDEO_FORMATS)} onChange={setVideoFormat} />
          )}
          {!audio && (
            <div className="field field-quality">
              <label className="field-label" htmlFor="download-quality">
                Quality
              </label>
              <select id="download-quality" value={quality} onChange={(e) => setQuality(e.target.value)}>
                {QUALITIES.map((q) => (
                  <option key={q.value} value={q.value}>
                    {q.label}
                  </option>
                ))}
              </select>
            </div>
          )}
        </div>

        {error && (
          <p className="form-error" id="download-error" role="alert">
            <Icon name="alert" size={16} />
            {error}
          </p>
        )}

        <div className="form-actions">
          <p className="hint">
            {audio
              ? "Takes the best audio the site offers and converts it."
              : quality === "best"
                ? "Takes the highest resolution the site offers."
                : "Takes the highest resolution up to your limit, or the closest above it if there is nothing smaller."}
          </p>
          <button className="btn btn-primary" type="submit" disabled={busy}>
            <Icon name="download" size={18} />
            {busy ? "Starting…" : "Download"}
          </button>
        </div>
      </form>

      <JobList
        title="Downloads"
        jobs={jobs}
        onDelete={onDelete}
        emptyTitle="No downloads yet"
        emptyHint="Paste a link above — finished files wait here for you to save them."
      />
    </>
  );
}
