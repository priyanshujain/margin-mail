import { useEffect, useRef, type ReactNode } from "react";
import { useEscapeLayer } from "../escape";
import { Button } from "./Button";
import { CHEVRON_LEFT, CLOSE } from "./icons";
import "./Sheet.css";

export interface SheetProps {
  open: boolean;
  title: string;
  onClose: () => void;
  /** `mini` is a picker, `wide` the editor, `full` the guide, which is a library and not a form. */
  size?: "mini" | "default" | "wide" | "full";
  foot?: ReactNode;
  /** Set when this panel was opened from another one, so Escape retraces the way in. */
  onBack?: () => void;
  /** What the back control says it goes back to: "the command palette", "settings". */
  backLabel?: string;
  /**
   * Something the panel started is still running. Nothing closes it until that finishes: not the
   * close control, not the scrim, not Escape. A sheet that goes away under a command that is still
   * holding the database lock leaves the person guessing whether it ran.
   */
  busy?: boolean;
  children: ReactNode;
}

/**
 * Every panel in the app, and on a phone every bottom sheet: they are the same box, docked
 * differently, which is a rule in app.css rather than a second component here.
 *
 * Nothing is resident. A closed sheet renders nothing at all and its children mount fresh on the
 * next open, which is what keeps the form state in each panel from having to be reset by hand.
 *
 * The back trail is two props rather than a store. The calendar reads its trail from useOverlays,
 * but the store that knows what opened what is not this layer's business, and a primitive that
 * imports one cannot be rendered on a Kit page.
 */
export function Sheet({
  open,
  title,
  onClose,
  size = "default",
  foot,
  onBack,
  backLabel = "the last panel",
  busy,
  children,
}: SheetProps) {
  // Escape retraces the way in rather than throwing away every panel at once. With nowhere to go
  // back to it closes, which is what it always did. While busy the layer stays up and does nothing,
  // so the key does not fall through to whatever is underneath.
  useEscapeLayer(open, busy ? () => {} : (onBack ?? onClose));
  const panel = useRef<HTMLDivElement | null>(null);

  // Focus has to leave the page or the first keystroke goes to the keymap instead of the panel.
  useEffect(() => {
    if (!open) return;
    const el = panel.current;
    if (!el) return;
    (el.querySelector<HTMLElement>("[data-autofocus]") ?? el).focus();
  }, [open]);

  if (!open) return null;

  return (
    <div
      className="overlay"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget && !busy) onClose();
      }}
    >
      <div
        className="panel sheet"
        data-size={size}
        data-busy={busy ? "" : undefined}
        aria-busy={busy || undefined}
        ref={panel}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={title}
      >
        <div className="panel-head">
          {onBack ? (
            <Button
              variant="ghost"
              iconOnly
              icon={CHEVRON_LEFT}
              title={`Back to ${backLabel} (⎋)`}
              label={`Back to ${backLabel}`}
              disabled={busy}
              onClick={onBack}
            />
          ) : null}
          <h2>{title}</h2>
          <Button
            variant="ghost"
            iconOnly
            icon={CLOSE}
            title="Close (⎋)"
            label="Close"
            disabled={busy}
            onClick={onClose}
          />
        </div>
        <div className="panel-body">{children}</div>
        {foot ? <div className="panel-foot">{foot}</div> : null}
      </div>
    </div>
  );
}

export interface ConfirmProps {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  busy?: boolean;
  /** What the button says while `busy`: "Clearing", "Removing". The label with an ellipsis if not. */
  busyLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * A destructive step in front of the panel it belongs to. The layer above unwinds first.
 *
 * Once confirmed there is no way out until the command answers. Cancel cannot take back a delete
 * that is already running, and a confirmation that closes on Escape halfway through reads as one
 * that never ran.
 */
export function Confirm({
  title,
  body,
  confirmLabel,
  busy,
  busyLabel,
  onConfirm,
  onCancel,
}: ConfirmProps) {
  useEscapeLayer(true, busy ? () => {} : onCancel);
  const cancel = useRef<HTMLDivElement | null>(null);

  // The safe option takes the focus, so a stray Enter does nothing destructive.
  useEffect(() => {
    cancel.current?.querySelector("button")?.focus();
  }, []);

  return (
    <div className="confirm" data-busy={busy ? "" : undefined}>
      <h3 className="confirm-title">{title}</h3>
      <div className="confirm-body">{body}</div>
      <div className="confirm-actions" ref={cancel}>
        <Button disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
        <Button variant="danger" disabled={busy} onClick={onConfirm}>
          {busy ? (busyLabel ?? `${confirmLabel}…`) : confirmLabel}
        </Button>
      </div>
    </div>
  );
}
