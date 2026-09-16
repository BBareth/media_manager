import { useRef, useState } from "react";

interface Props {
  accept?: string;
  multiple?: boolean;
  fileNames: string[];
  onPick: (files: File[]) => void;
}

/// Styled replacement for the native file input: a clickable drop zone with
/// keyboard support. The real input stays hidden and is cleared after every
/// pick so selecting the same file twice still fires onChange.
export default function FilePicker({ accept, multiple, fileNames, onPick }: Props) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [dragOver, setDragOver] = useState(false);

  function handleFiles(list: FileList | null) {
    const files = list ? Array.from(list) : [];
    onPick(multiple ? files : files.slice(0, 1));
  }

  return (
    <div
      className={`file-picker${dragOver ? " drag-over" : ""}${fileNames.length > 0 ? " has-files" : ""}`}
      role="button"
      tabIndex={0}
      onClick={() => inputRef.current?.click()}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          inputRef.current?.click();
        }
      }}
      onDragOver={(e) => {
        e.preventDefault();
        setDragOver(true);
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setDragOver(false);
        handleFiles(e.dataTransfer.files);
      }}
    >
      <input
        ref={inputRef}
        type="file"
        accept={accept}
        multiple={multiple}
        hidden
        onChange={(e) => {
          handleFiles(e.target.files);
          e.target.value = "";
        }}
      />
      <span className="file-picker-icon" aria-hidden="true">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
          <polyline points="17 8 12 3 7 8" />
          <line x1="12" y1="3" x2="12" y2="15" />
        </svg>
      </span>
      <span className="file-picker-text">
        {fileNames.length === 0 ? (
          <>
            <strong>Choose {multiple ? "files" : "a file"}</strong>
            <span>or drag and drop here</span>
          </>
        ) : (
          <>
            <strong>{fileNames.length === 1 ? fileNames[0] : `${fileNames.length} files selected`}</strong>
            <span>Click to change</span>
          </>
        )}
      </span>
      <span className="file-picker-browse">Browse</span>
    </div>
  );
}
