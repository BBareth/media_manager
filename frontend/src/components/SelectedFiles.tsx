import { formatBytes } from "../format";
import Icon from "./Icon";

interface Props {
  files: File[];
  onRemove: (index: number) => void;
  /// A per-file warning, e.g. that it is already under the target.
  note?: (file: File) => string | null;
}

/// The files picked but not yet sent.
export default function SelectedFiles({ files, onRemove, note }: Props) {
  if (files.length === 0) return null;
  const total = files.reduce((sum, f) => sum + f.size, 0);
  return (
    <div className="selected">
      <ul className="file-list" aria-label="Selected files">
        {files.map((f, i) => {
          const warning = note?.(f);
          return (
            <li key={`${f.name}-${f.size}-${f.lastModified}-${i}`} className={warning ? "has-note" : undefined}>
              <div className="upload-row">
                <Icon name="file" size={16} className="upload-icon" />
                <span className="upload-name" title={f.name}>
                  {f.name}
                </span>
                <span className="upload-size">{formatBytes(f.size)}</span>
                <button type="button" className="icon-btn" aria-label={`Remove ${f.name}`} onClick={() => onRemove(i)}>
                  <Icon name="x" size={14} />
                </button>
              </div>
              {warning && <p className="upload-note upload-warning">{warning}</p>}
            </li>
          );
        })}
      </ul>
      {files.length > 1 && (
        <p className="selected-total">
          {files.length} files · {formatBytes(total)}
        </p>
      )}
    </div>
  );
}
