import { create } from "zustand";

/**
 * Which overlay is on screen. Ported from the calendar's store of the same name, trail included.
 *
 * Nothing here is resident: an overlay is summoned by a key and dismissed with Escape, and only one
 * is open at a time, so the keymap's context stack has exactly one frame to shadow the view's
 * bindings with.
 *
 * `trail` is where you came from. The palette opens Settings, and without it that panel is a dead
 * end whose only way out is closing everything and starting again.
 *
 * The list names every panel the app will have, not only the ones that exist today: `compose`,
 * `settings` and `accounts` belong to later packages and nothing opens them yet.
 */
export type Overlay =
  | "palette"
  | "shortcuts"
  /** The slideshow a newly added account ends on, and the first row of the help menu after that. */
  | "tour"
  /** The guide, which takes the window rather than the stage: it is read over the app, not instead
      of it, and closing it puts back exactly what was underneath. */
  | "guide"
  | "search"
  | "compose"
  | "settings"
  | "accounts"
  | "snooze"
  /** The phone's overflow sheet. A desktop reaches all of this from the header or a key. */
  | "menu";

interface OverlayState {
  open: Overlay | null;
  /** The panels behind this one, outermost first. Empty when this one was opened directly. */
  trail: Overlay[];
  /** Opens one on its own. Anything you were in is gone, not remembered. */
  show: (overlay: Overlay) => void;
  /** Opens one from inside another, so `back` can return to it. */
  push: (overlay: Overlay) => void;
  /** Returns to the panel that opened this one, or closes when there is nowhere to go. */
  back: () => void;
  /**
   * Records where the panel that is already open was reached from.
   *
   * `push` cannot do this job for a panel opened by a command, because a command calls `show` and
   * has no idea anything summoned it.
   */
  reachedFrom: (previous: Overlay) => void;
  toggle: (overlay: Overlay) => void;
  close: () => void;
}

export const useOverlays = create<OverlayState>((set) => ({
  open: null,
  trail: [],
  show: (overlay) => set({ open: overlay, trail: [] }),
  push: (overlay) =>
    set((s) =>
      s.open === null
        ? { open: overlay, trail: [] }
        : { open: overlay, trail: [...s.trail, s.open] },
    ),
  back: () =>
    set((s) => {
      const trail = s.trail.slice();
      const previous = trail.pop() ?? null;
      return { open: previous, trail };
    }),
  // Nothing to go back to when nothing is open, and a panel is not its own way out.
  reachedFrom: (previous) =>
    set((s) => (s.open === null || s.open === previous ? {} : { trail: [previous] })),
  toggle: (overlay) =>
    set((s) => (s.open === overlay ? { open: null, trail: [] } : { open: overlay, trail: [] })),
  close: () => set({ open: null, trail: [] }),
}));
