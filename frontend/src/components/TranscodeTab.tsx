import { FormEvent, useCallback, useState } from "react";
import { Job, UploadOptions, startTranscode } from "../api";
import { formatLabel } from "../format";
import { isBoolean, oneOf, usePersistentState } from "../hooks/usePersistentState";
import { useUploadQueue } from "../hooks/useUploadQueue";
import ChoiceGroup, { Choice } from "./ChoiceGroup";
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

type Kind = "video" | "audio";
const KINDS: Choice<Kind>[] = [
  { value: "video", label: "Video" },
  { value: "audio", label: "Audio only" },
];
const VIDEO_FORMATS = ["mp4", "mkv", "mov", "webm", "avi"] as const;
const AUDIO_FORMATS = ["mp3", "m4a", "ogg", "opus", "wav", "flac"] as const;
const HINTS: Record<string, string> = {
  mp4: "H.264 — plays everywhere",
  mkv: "H.264, keeps every track",
  mov: "H.264 for Apple software",
  webm: "VP9 — open format, best in browsers",
  avi: "XviD for old hardware players",
  mp3: "Plays everywhere",
  m4a: "AAC, smaller than MP3 at the same quality",
  ogg: "Vorbis",
  opus: "Smallest files for the quality",
  wav: "Uncompressed",
  flac: "Lossless",
};
/// Containers that hold a single audio track.
const SINGLE_TRACK = ["mp3", "opus", "wav", "flac"];

const choices = <T extends string>(formats: readonly T[]): Choice<T>[] =>
  formats.map((f) => ({ value: f, label: formatLabel(f), hint: HINTS[f] }));

export default function TranscodeTab({ jobs, onJob, onDelete }: Props) {
  const [files, setFiles] = useState<File[]>([]);
  const [kind, setKind] = usePersistentState<Kind>("transcode.kind", "video", oneOf(["video", "audio"]));
  const [videoFormat, setVideoFormat] = usePersistentState("transcode.videoFormat", "mp4", oneOf(VIDEO_FORMATS));
  const [audioFormat, setAudioFormat] = usePersistentState("transcode.audioFormat", "mp3", oneOf(AUDIO_FORMATS));
  const [combineAudio, setCombineAudio] = usePersistentState("transcode.combineAudio", false, isBoolean);
  const [error, setError] = useState<string | null>(null);

  const format = kind === "audio" ? audioFormat : videoFormat;
  const start = useCallback(
    (file: File, opts: UploadOptions) => startTranscode(file, format, combineAudio, opts),
    [format, combineAudio]
  );
  const queue = useUploadQueue(start, onJob);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    if (files.length === 0) {
      setError("Choose at least one file to convert.");
      return;
    }
    const picked = files;
    setFiles([]);
    await queue.run(picked.map((file) => ({ file })));
  }

  return (
    <>
      <form className="card form" onSubmit={submit} noValidate>
        <header className="card-header">
          <h2>Convert a file</h2>
          <p>Change the format of a video or audio file, or pull the audio out of a video.</p>
        </header>

        <FilePicker
          multiple
          disabled={queue.busy}
          label="Choose files to convert"
          hint="Video or audio — pick several to convert them all the same way"
          accept="audio/*,video/*,.avi,.mkv,.mov,.flv,.wmv,.m4v,.ts,.mts"
          fileNames={files.map((f) => f.name)}
          onPick={(picked) => {
            setFiles(picked);
            setError(null);
          }}
        />
        <SelectedFiles files={files} onRemove={(i) => setFiles((prev) => prev.filter((_, n) => n !== i))} />
        <UploadQueue items={queue.items} onDismiss={queue.dismiss} />

        <div className="options">
          <ChoiceGroup legend="Convert to" value={kind} options={KINDS} onChange={setKind} variant="segmented" disabled={queue.busy} />
          {kind === "audio" ? (
            <ChoiceGroup legend="Audio format" value={audioFormat} options={choices(AUDIO_FORMATS)} onChange={setAudioFormat} disabled={queue.busy} />
          ) : (
            <ChoiceGroup legend="Video format" value={videoFormat} options={choices(VIDEO_FORMATS)} onChange={setVideoFormat} disabled={queue.busy} />
          )}
        </div>

        <label className="checkbox-field">
          <input
            type="checkbox"
            checked={combineAudio}
            disabled={queue.busy}
            onChange={(e) => setCombineAudio(e.target.checked)}
          />
          <span>
            Combine all audio tracks into one
            <span className="muted"> — for recordings with separate game and mic tracks</span>
          </span>
        </label>

        {!combineAudio && SINGLE_TRACK.includes(format) && (
          <p className="hint">
            {formatLabel(format)} holds a single audio track. If a file has several, tick "Combine" — or pick MKV, MP4, M4A or OGG
            to keep them separate.
          </p>
        )}
        {error && (
          <p className="form-error" role="alert">
            <Icon name="alert" size={16} />
            {error}
          </p>
        )}

        <div className="form-actions">
          <p className="hint">If a file's streams already suit the new format they are copied, not re-encoded — instant and lossless.</p>
          {queue.busy ? (
            <button type="button" className="btn btn-secondary" onClick={queue.cancel}>
              Cancel uploads
            </button>
          ) : (
            <button className="btn btn-primary" type="submit">
              <Icon name="convert" size={18} />
              {files.length > 1 ? `Convert ${files.length} files` : "Convert"}
            </button>
          )}
        </div>
      </form>

      <JobList
        title="Conversions"
        jobs={jobs}
        onDelete={onDelete}
        emptyTitle="Nothing converted yet"
        emptyHint="Choose a file above and pick the format you need."
      />
    </>
  );
}
