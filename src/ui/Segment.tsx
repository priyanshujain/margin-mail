import { Key } from "./Key";
import "./Segment.css";

export interface SegmentOption {
  id: string;
  label: string;
  /** The number key this box answers to, printed in the segment. */
  keycap?: string;
}

export interface SegmentProps {
  options: SegmentOption[];
  value: string;
  onChange: (id: string) => void;
  label?: string;
  /** While the last choice is still being written, so it cannot be picked over. */
  disabled?: boolean;
}

/** The switch in the middle of the header: the three boxes, each carrying its number. */
export function Segment({ options, value, onChange, label, disabled }: SegmentProps) {
  return (
    <div
      className="segment"
      role="tablist"
      aria-label={label}
      aria-disabled={disabled || undefined}
      data-disabled={disabled ? "" : undefined}
    >
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
          role="tab"
          className="segment-option"
          aria-selected={o.id === value}
          data-active={o.id === value ? "" : undefined}
          disabled={disabled}
          onClick={() => onChange(o.id)}
        >
          {o.keycap ? <Key size="sm">{o.keycap}</Key> : null}
          {o.label}
        </button>
      ))}
    </div>
  );
}
