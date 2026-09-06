import type { ReactNode } from "react";
import "./EmptyState.css";

export interface EmptyStateProps {
  children: ReactNode;
}

/**
 * One quiet line in the text face and nothing else.
 *
 * No illustration, no photograph, no streak, no button suggesting you go and make some mail. An
 * empty list is a fine thing for a list to be, and the app has no opinion about it.
 */
export function EmptyState({ children }: EmptyStateProps) {
  return <p className="empty-state">{children}</p>;
}
