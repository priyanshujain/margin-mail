// Which whole-stage screen is up, when it is not one of the thread places.
//
// `Place` in the frozen contract is a query over threads, and three of the palette's entries are
// not that: Contacts lists people, Clips lists passages, All files lists attachments. Focus & Reply
// is a fourth, a page over the Reply later pile rather than a place you can be in. None of them
// belongs in `Place`, and none of them is an overlay either, because an overlay is something you
// dismiss to get back to what you were doing and these are somewhere you go.
//
// So they are a stage of their own, and the rule is that a stage wins over a place: opening
// Contacts leaves the Inbox where it was, and Escape puts you back on it.

import { create } from "zustand";

export type Stage = "contacts" | "clips" | "files" | "focus-reply";

interface StageState {
  open: Stage | null;
  show: (stage: Stage) => void;
  close: () => void;
  toggle: (stage: Stage) => void;
}

export const useStage = create<StageState>((set) => ({
  open: null,
  show: (stage) => set({ open: stage }),
  close: () => set({ open: null }),
  toggle: (stage) => set((s) => (s.open === stage ? { open: null } : { open: stage })),
}));
