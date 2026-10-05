import { ReactNode, useEffect, useState } from "react";

interface Props {
  onConfirm: () => void;
  children: ReactNode;
  /// Label while armed, e.g. "Delete file?".
  confirmLabel: string;
  className?: string;
  title?: string;
}

/// A destructive button that needs a second click. Cheaper than a modal, harder to hit by
/// accident than a plain button, and it disarms itself if the second click never comes.
export default function ConfirmButton({ onConfirm, children, confirmLabel, className = "", title }: Props) {
  const [armed, setArmed] = useState(false);

  useEffect(() => {
    if (!armed) return;
    const id = window.setTimeout(() => setArmed(false), 4000);
    return () => window.clearTimeout(id);
  }, [armed]);

  return (
    <button
      type="button"
      className={`${className}${armed ? " btn-armed" : ""}`}
      title={title}
      aria-live="polite"
      onClick={() => {
        if (armed) {
          setArmed(false);
          onConfirm();
        } else {
          setArmed(true);
        }
      }}
      onBlur={() => setArmed(false)}
    >
      {armed ? confirmLabel : children}
    </button>
  );
}
