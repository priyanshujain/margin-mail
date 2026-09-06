import type { ReactNode } from "react";
import { Key } from "./Key";
import "./Toast.css";

export interface ToastProps {
  children: ReactNode;
  /** Every destructive or hidden action offers one, and it is always the same key. */
  action?: { label: string; keycap?: string; onClick: () => void };
}

/**
 * The acknowledgement at the foot of the window: what just happened and how to take it back.
 *
 * This is the only feedback a triage key gives. Rows do not animate when they leave for a pile or
 * an archive, because a list that rearranges itself under the hand is a list you stop trusting.
 */
export function Toast({ children, action }: ToastProps) {
  return (
    <div className="toast" role="status">
      <span className="toast-text">{children}</span>
      {action ? (
        <button type="button" className="toast-action" onClick={action.onClick}>
          {action.label}
          {action.keycap ? <Key size="sm">{action.keycap}</Key> : null}
        </button>
      ) : null}
    </div>
  );
}
