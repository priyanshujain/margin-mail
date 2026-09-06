import { create } from "zustand";

/**
 * What the toast offers besides the sentence. Every toast in this app carries one: docs/keyboard.md
 * makes the toast the only acknowledgement a triage key gives, so it is also the only place the
 * action can be taken back from.
 */
export interface ToastAction {
  label: string;
  /** Printed as a cap on the button. Always `z`, because undo has one key everywhere. */
  keycap?: string;
  run: () => void;
}

interface ToastState {
  message: string | null;
  action: ToastAction | null;
  /** Bumped on every notice, so an identical message twice still restarts the dismissal timer. */
  seq: number;
  notice: (message: string, action?: ToastAction) => void;
  dismiss: () => void;
}

export const useToast = create<ToastState>((set) => ({
  message: null,
  action: null,
  seq: 0,
  notice: (message, action) =>
    set((s) => ({ message, action: action ?? null, seq: s.seq + 1 })),
  dismiss: () => set({ message: null, action: null }),
}));

export const notify = (message: string, action?: ToastAction) =>
  useToast.getState().notice(message, action);
