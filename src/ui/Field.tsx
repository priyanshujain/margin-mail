import { useId } from "react";
import { NO_AUTOFILL } from "./autofill";
import "./Field.css";

export interface FieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  /** A textarea rather than an input: a note, a signature, a screening reason. */
  multiline?: boolean;
  rows?: number;
  /** One quiet line under the field. `tone="error"` says it in the danger ink. */
  hint?: string;
  tone?: "default" | "error";
  type?: "text" | "email" | "search" | "password";
  /**
   * What the platform may offer to fill in. Off unless said otherwise, because a field that keeps
   * what was typed and offers it back later looks like a suggestion this app is making. An address
   * is the one place a suggestion is wanted: the person's own, from their contact card. It also
   * settles what the field is, which matters on a Mac, where a text field that says nothing about
   * itself gets offered whatever code just arrived in Messages.
   */
  autoComplete?: "email" | "name" | "username" | "current-password" | "new-password";
  disabled?: boolean;
  autoFocus?: boolean;
}

export function Field({
  label,
  value,
  onChange,
  placeholder,
  multiline,
  rows = 4,
  hint,
  tone = "default",
  type = "text",
  autoComplete,
  disabled,
  autoFocus,
}: FieldProps) {
  const id = useId();
  return (
    <div className="field">
      <label className="field-label" htmlFor={id}>
        {label}
      </label>
      {multiline ? (
        <textarea
          id={id}
          className="field-textarea"
          rows={rows}
          value={value}
          placeholder={placeholder}
          disabled={disabled}
          autoFocus={autoFocus}
          data-autofocus={autoFocus ? "" : undefined}
          autoComplete="off"
          onChange={(e) => onChange(e.target.value)}
        />
      ) : (
        <input
          id={id}
          className="field-input"
          type={type}
          value={value}
          placeholder={placeholder}
          disabled={disabled}
          autoFocus={autoFocus}
          data-autofocus={autoFocus ? "" : undefined}
          {...NO_AUTOFILL}
          autoComplete={autoComplete ?? NO_AUTOFILL.autoComplete}
          onChange={(e) => onChange(e.target.value)}
        />
      )}
      {hint ? (
        <span className="field-hint" data-tone={tone}>
          {hint}
        </span>
      ) : null}
    </div>
  );
}
