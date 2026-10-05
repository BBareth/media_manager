import { useId } from "react";

export interface Choice<T extends string> {
  value: T;
  label: string;
  hint?: string;
}

interface Props<T extends string> {
  legend: string;
  value: T;
  options: readonly Choice<T>[];
  onChange: (value: T) => void;
  disabled?: boolean;
  /// "segmented" fills the row with equal parts; "chips" wraps like tags.
  variant?: "segmented" | "chips";
}

/// A set of mutually exclusive options shown as buttons. Built on real radio inputs, so arrow
/// keys, screen readers and form semantics come for free.
export default function ChoiceGroup<T extends string>({
  legend,
  value,
  options,
  onChange,
  disabled,
  variant = "chips",
}: Props<T>) {
  const name = useId();
  return (
    <fieldset className={`choice-group choice-${variant}`} disabled={disabled}>
      <legend className="field-label">{legend}</legend>
      <div className="choice-options">
        {options.map((o) => (
          <label key={o.value} className="choice" title={o.hint}>
            <input
              type="radio"
              name={name}
              value={o.value}
              checked={value === o.value}
              onChange={() => onChange(o.value)}
            />
            <span>{o.label}</span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}
