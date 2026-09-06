// Responsiveness is JS-driven: these write attributes on the root and the stylesheets key off
// them, so there are essentially no media queries outside the token layer.

import { useEffect, useState } from "react";

export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const mq = window.matchMedia(query);
    const onChange = () => setMatches(mq.matches);
    onChange();
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [query]);
  return matches;
}

/** Kept in step with the same two queries in index.html's boot script. */
export const PHONE_QUERY = "(max-width: 640px)";
export const TOUCH_QUERY = "(pointer: coarse)";

/**
 * A phone, meaning a window too narrow for the header's three groups, a 420px list column and a
 * reading pane beside it. The chrome moves to a top bar and a bottom tab bar and the list becomes
 * the screen.
 *
 * Deliberately measured in CSS pixels rather than sniffed off the user agent: a phone in landscape
 * is 850 wide and wants the desktop layout back, a tablet at 768 has always wanted it, and a
 * narrow desktop window is a free way to exercise all of this without a device.
 */
export function usePhone(): boolean {
  const phone = useMediaQuery(PHONE_QUERY);
  useEffect(() => {
    document.documentElement.toggleAttribute("data-phone", phone);
  }, [phone]);
  return phone;
}

/**
 * A coarse pointer, which is a different question from `usePhone`: a tablet is touch and not a
 * phone, and a narrow desktop window is a phone and not touch.
 *
 * This is the one that governs interaction rather than layout. A row's actions cannot wait for a
 * hover here, and the app prints no keycaps on a device with no keys.
 */
export function useTouch(): boolean {
  const touch = useMediaQuery(TOUCH_QUERY);
  useEffect(() => {
    document.documentElement.toggleAttribute("data-touch", touch);
  }, [touch]);
  return touch;
}
