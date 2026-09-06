import type { ReactNode } from "react";
import { Icon } from "./Icon";
import "./Banner.css";

export interface BannerProps {
  children: ReactNode;
  icon?: string;
  /** `muted` is the note surface, for a thing you did (a merge, an ignore) rather than a notice. */
  tone?: "default" | "muted";
  action?: {
    label: string;
    onClick: () => void;
    /**
     * The verb has been pressed and the answer is still coming. The button says so in its own
     * words (`busyLabel`, or the label with an ellipsis) and refuses a second press, because a
     * control that looks the same before and after being pressed is a control that looks stuck.
     */
    busy?: boolean;
    busyLabel?: string;
  };
}

/** The strip that says what the app did before you read the message: stripped a tracker, merged
 *  two threads, held a sender out of the list. Always with the way to undo it. */
export function Banner({ children, icon, tone = "default", action }: BannerProps) {
  return (
    <div className="banner" data-tone={tone}>
      {icon ? <Icon d={icon} size={14} /> : null}
      <span className="banner-text">{children}</span>
      {action ? (
        <button
          type="button"
          className="banner-action"
          data-busy={action.busy ? "" : undefined}
          aria-busy={action.busy ? "true" : undefined}
          disabled={action.busy}
          onClick={action.onClick}
        >
          {action.busy ? (action.busyLabel ?? `${action.label}…`) : action.label}
        </button>
      ) : null}
    </div>
  );
}
