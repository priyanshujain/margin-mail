import type { ReactNode, Ref } from "react";
import { Icon } from "./Icon";
import { Key } from "./Key";
import "./Button.css";

export type ButtonVariant = "default" | "primary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps {
  children?: ReactNode;
  onClick?: () => void;
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** A path from icons.ts, drawn before the label. */
  icon?: string;
  /** The key this button's verb answers to, printed on it. */
  keycap?: string;
  /** A square control with the icon and nothing else. Always give it a title. */
  iconOnly?: boolean;
  /** For a control that is currently the one in effect, such as an open panel's button. */
  active?: boolean;
  disabled?: boolean;
  /** Written out with the shortcut in real glyphs, because an icon alone says nothing. */
  title?: string;
  label?: string;
  type?: "button" | "submit";
  /** The element itself, for a popover that has to hang off this button. */
  ref?: Ref<HTMLButtonElement>;
}

export function Button({
  children,
  onClick,
  variant = "default",
  size = "md",
  icon,
  keycap,
  iconOnly,
  active,
  disabled,
  title,
  label,
  type = "button",
  ref,
}: ButtonProps) {
  return (
    <button
      ref={ref}
      type={type}
      className="button"
      data-variant={variant}
      data-size={size}
      data-icon-only={iconOnly ? "" : undefined}
      data-active={active ? "" : undefined}
      disabled={disabled}
      title={title}
      aria-label={label ?? (iconOnly ? title : undefined)}
      onClick={onClick}
    >
      {icon ? <Icon d={icon} size={iconOnly ? 16 : 14} /> : null}
      {iconOnly ? null : children}
      {keycap && !iconOnly ? <Key size="sm">{keycap}</Key> : null}
    </button>
  );
}
