import { create } from "zustand";

/** The Move to picker: which threads it was opened on. Empty is closed. */
interface MoveState {
  keys: string[];
  show: (keys: string[]) => void;
  hide: () => void;
}

export const useMove = create<MoveState>((set) => ({
  keys: [],
  show: (keys) => set({ keys }),
  hide: () => set({ keys: [] }),
}));
