import type { ReactNode } from "react";
import { Icon } from "./Icon";
import { Key } from "./Key";
import "./Pill.css";

export type PillTone = "default" | "wash" | "quiet";

export interface PillProps {
  children: ReactNode;
  /** `wash` is the Screener's reason, `quiet` the quoted-text toggle. */
  tone?: PillTone;
  icon?: string;
  keycap?: string;
  /** Given, the pill is a button; withheld, it is a label. */
  onClick?: () => void;
  title?: string;
}

/** The small rounded label: a Screener reason, a message count, the quoted-text toggle. */
export function Pill({ children, tone = "default", icon, keycap, onClick, title }: PillProps) {
  const inner = (
    <>
      {icon ? <Icon d={icon} size={12} /> : null}
      {children}
      {keycap ? <Key size="sm">{keycap}</Key> : null}
    </>
  );

  if (onClick) {
    return (
      <button type="button" className="pill" data-tone={tone} title={title} onClick={onClick}>
        {inner}
      </button>
    );
  }
  return (
    <span className="pill" data-tone={tone} title={title}>
      {inner}
    </span>
  );
}
