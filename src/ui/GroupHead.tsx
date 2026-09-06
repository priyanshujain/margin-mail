import type { ReactNode } from "react";
import "./GroupHead.css";

export interface GroupHeadProps {
  children: ReactNode;
  /** A small verb at the right of the label. Nothing here ever carries a count. */
  action?: { label: string; onClick: () => void };
  /** A hairline across the rest of the line, for a marker in a stream rather than a heading. */
  rule?: boolean;
}

/** The small uppercase label above a group of rows, and the only heading a list has. */
export function GroupHead({ children, action, rule }: GroupHeadProps) {
  return (
    <div className="group-head" data-rule={rule ? "" : undefined}>
      <span className="group-head-label">{children}</span>
      {rule ? <span className="group-head-rule" /> : null}
      {action ? (
        <button type="button" className="group-head-action" onClick={action.onClick}>
          {action.label}
        </button>
      ) : null}
    </div>
  );
}
