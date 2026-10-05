import { useRef, useState } from "react";
import Icon from "./Icon";

interface Props {
  accept?: string;
  multiple?: boolean;
  disabled?: boolean;
  /// What the zone is for, read out by screen readers.
  label: string;
  /// Short line under the call to action, e.g. "Video, audio or images".
  hint?: string;
  fileNames: string[];
  onPick: (files: File[]) => void;
}

/// Styled replacement for the native file input: a clickable drop zone with
/// keyboard support. The real input stays hidden and is cleared after every
/// pick so selecting the same file twice still fires onChange.
export default function FilePicker({ accept, multiple, disabled, label, hint, fileNames, onPick }: Props) {
  const inputRef = useRef<HTMLInputElement>(null);
  // dragenter/dragleave also fire for every child element crossed; count them so the
  // highlight does not flicker while the pointer moves over the text.
  const depth = useRef(0);
  const [dragOver, setDragOver] = useState(false);

  function handleFiles(list: FileList | null) {
    if (disabled) return;
    const files = list ? Array.from(list) : [];
    if (files.length > 0) onPick(multiple ? files : files.slice(0, 1));
  }

  const open = () => !disabled && inputRef.current?.click();

  return (
    <div
      className={`file-picker${dragOver ? " drag-over" : ""}${fileNames.length > 0 ? " has-files" : ""}${disabled ? " disabled" : ""}`}
      role="button"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-disabled={disabled}
      onClick={open}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          open();
        }
      }}
      onDragEnter={(e) => {
        e.preventDefault();
        depth.current += 1;
        if (!disabled) setDragOver(true);
      }}
      onDragOver={(e) => e.preventDefault()}
      onDragLeave={() => {
        depth.current = Math.max(0, depth.current - 1);
        if (depth.current === 0) setDragOver(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        depth.current = 0;
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
      <span className="file-picker-icon">
        <Icon name="upload" size={20} />
      </span>
      <span className="file-picker-text">
        {fileNames.length === 0 ? (
          <>
            <strong>{dragOver ? "Drop to add" : `Choose ${multiple ? "files" : "a file"} or drag ${multiple ? "them" : "it"} here`}</strong>
            {hint && <span>{hint}</span>}
          </>
        ) : (
          <>
            <strong>{fileNames.length === 1 ? fileNames[0] : `${fileNames.length} files selected`}</strong>
            <span>Click or drop to replace</span>
          </>
        )}
      </span>
      <span className="file-picker-browse">Browse</span>
    </div>
  );
}
