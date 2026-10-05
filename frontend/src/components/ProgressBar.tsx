interface Props {
  value: number;
  label: string;
  /// Text under the bar, e.g. "42 % · about 3 min left".
  caption?: string;
  /// Animate an indeterminate bar while the true value is unknown.
  indeterminate?: boolean;
}

export default function ProgressBar({ value, label, caption, indeterminate }: Props) {
  const pct = Math.max(0, Math.min(100, value));
  return (
    <div className="progress-block">
      <div
        className={`progress${indeterminate ? " progress-indeterminate" : ""}`}
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={indeterminate ? undefined : Math.round(pct)}
      >
        <div className="progress-fill" style={{ width: indeterminate ? undefined : `${pct}%` }} />
      </div>
      {caption && <div className="progress-caption">{caption}</div>}
    </div>
  );
}
