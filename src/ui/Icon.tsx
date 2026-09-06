import type { ReactNode } from "react";
import "./Icon.css";

export interface IconProps {
  /** A 24x24 stroke path from icons.ts. */
  d?: string;
  size?: number;
  /** For the rare glyph that is not one path. Nothing in mail needs it yet. */
  children?: ReactNode;
}

export function Icon({ d, size = 16, children }: IconProps) {
  return (
    <svg
      className="icon"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children ?? <path d={d} />}
    </svg>
  );
}
