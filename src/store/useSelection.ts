import { create } from "zustand";

/**
 * The threads `x` has picked out, and the row the extension is measured from.
 *
 * The anchor is what makes `Shift+J` and `Shift+K` a range rather than a run of toggles: pressing
 * `Shift+J` three times and then `Shift+K` twice has to give back the two rows it just took,
 * which a set alone cannot say.
 *
 * The order is the list's, so extending takes the keys in the order they are on screen. The list
 * hands them in rather than this store reaching for them, because a selection made in one place is
 * not extended by the rows of another.
 */
interface SelectionState {
  keys: string[];
  anchor: string | null;
  toggle: (key: string) => void;
  /** Selects every row between the anchor and `to`, inclusive, leaving the anchor where it was. */
  extend: (ordered: string[], to: string) => void;
  /** `Cmd+A`: everything from the anchor, or from the focused row, to the end of the list. */
  allFrom: (ordered: string[], from: string | null) => void;
  /** Puts back a selection that something else took the stage from, such as a search. */
  restore: (keys: string[], anchor: string | null) => void;
  clear: () => void;
}

const between = (ordered: string[], a: string, b: string): string[] => {
  const from = ordered.indexOf(a);
  const to = ordered.indexOf(b);
  if (from === -1 || to === -1) return [];
  return ordered.slice(Math.min(from, to), Math.max(from, to) + 1);
};

export const useSelection = create<SelectionState>((set) => ({
  keys: [],
  anchor: null,
  toggle: (key) =>
    set((s) => {
      const has = s.keys.includes(key);
      const keys = has ? s.keys.filter((k) => k !== key) : [...s.keys, key];
      return { keys, anchor: keys.length === 0 ? null : has ? s.anchor : key };
    }),
  extend: (ordered, to) =>
    set((s) => {
      const anchor = s.anchor ?? to;
      return { keys: between(ordered, anchor, to), anchor };
    }),
  allFrom: (ordered, from) =>
    set((s) => {
      const start = from ?? s.anchor ?? ordered[0];
      if (!start) return {};
      const at = ordered.indexOf(start);
      if (at === -1) return {};
      return { keys: ordered.slice(at), anchor: start };
    }),
  restore: (keys, anchor) => set({ keys, anchor: keys.length > 0 ? anchor : null }),
  clear: () => set({ keys: [], anchor: null }),
}));
