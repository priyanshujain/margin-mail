import "./Key.css";

export interface KeyProps {
  /** The key as it is printed: a letter, a digit, or a real glyph like the return arrow. */
  children: string;
  /** `sm` is the cap that rides inside a button, a pill or a segment; `md` stands on its own. */
  size?: "sm" | "md";
}

/**
 * The key printed on the thing it operates. Every button in the app carries one, which is how the
 * mouse teaches the keyboard, and the shortcut sheet and the palette are made of nothing else.
 */
export function Key({ children, size = "md" }: KeyProps) {
  return (
    <kbd className="key" data-size={size}>
      {children}
    </kbd>
  );
}
