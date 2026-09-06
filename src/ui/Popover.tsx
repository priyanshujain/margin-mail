import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { useEscapeLayer } from "../escape";
import "./Popover.css";

export type PopoverPlacement = "bottom-start" | "bottom-end" | "top-end";

export interface PopoverProps {
  open: boolean;
  /** The element it hangs from: a sender's name, a snooze button. */
  anchor: HTMLElement | null;
  onClose: () => void;
  placement?: PopoverPlacement;
  width?: number;
  label?: string;
  children: ReactNode;
}

const EDGE = 8;

/**
 * A panel hanging off something on the page: the contact card, the snooze picker.
 *
 * Positioned in fixed coordinates from the anchor's rect rather than by a containing block, so it
 * can hang off a name inside a scrolling thread without that thread having to become a positioning
 * context. It closes on Escape through the shared layer stack and on a pointer down anywhere else,
 * which is what makes it a popover rather than a small modal.
 *
 * The three numbers go out as custom properties rather than as left/top/width, so the phone rule
 * in the stylesheet can take the width back without fighting an inline style.
 *
 * `top-end` hangs above the anchor instead, for a control in the bottom corner whose menu would
 * otherwise open off the bottom of the window. Its top is the anchor's top and the stylesheet pulls
 * the panel up by its own height, so the placement costs no measurement of the panel and no second
 * render pass: the browser knows how tall the thing is and this code does not have to.
 */
export function Popover({
  open,
  anchor,
  onClose,
  placement = "bottom-start",
  width = 320,
  label,
  children,
}: PopoverProps) {
  const panel = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useEscapeLayer(open, onClose);

  useLayoutEffect(() => {
    if (!open || !anchor) {
      setPos(null);
      return;
    }
    const place = () => {
      const r = anchor.getBoundingClientRect();
      const wanted = placement === "bottom-start" ? r.left : r.right - width;
      const left = Math.min(Math.max(EDGE, wanted), window.innerWidth - width - EDGE);
      const top = placement === "top-end" ? r.top - 6 : r.bottom + 6;
      setPos((was) => (was && was.left === left && was.top === top ? was : { left, top }));
    };
    place();
    window.addEventListener("scroll", place, true);
    window.addEventListener("resize", place);
    // A page can move the anchor without scrolling or resizing anything: a font arrives, a
    // collapsed message opens, a banner appears above the thread. Measuring once leaves the card
    // hanging off nothing.
    const ro = new ResizeObserver(place);
    ro.observe(document.body);
    return () => {
      window.removeEventListener("scroll", place, true);
      window.removeEventListener("resize", place);
      ro.disconnect();
    };
  }, [open, anchor, placement, width]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (panel.current?.contains(t) || anchor?.contains(t)) return;
      onClose();
    };
    window.addEventListener("pointerdown", onDown, true);
    return () => window.removeEventListener("pointerdown", onDown, true);
  }, [open, anchor, onClose]);

  if (!open || !pos) return null;

  const place = {
    "--pop-left": `${pos.left}px`,
    "--pop-top": `${pos.top}px`,
    "--pop-w": `${width}px`,
  } as CSSProperties;

  return (
    <div
      className="popover"
      ref={panel}
      role="dialog"
      aria-label={label}
      data-placement={placement}
      style={place}
    >
      {children}
    </div>
  );
}
