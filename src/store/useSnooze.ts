import { create } from "zustand";
import { snoozeEvaluate, snoozeSet } from "../api/threads";
import { live, type Place, type SnoozeKind } from "../ipc";
import { acknowledge } from "../screens/triage";
import { useMail } from "./useMail";
import { useSelection } from "./useSelection";
import { useSettings } from "./useSettings";
import { notify } from "./useToast";

/**
 * The snooze picker: what it will act on, what it hangs off, and the one write it makes.
 *
 * The moment is computed here rather than in Rust, because what "this weekend" means belongs to
 * the person looking at the picker: their clock, their zone, and the four times they set once in
 * settings. Rust is handed an instant and stores it.
 */
type Phase = "idle" | "saving";

interface SnoozeState {
  open: boolean;
  /** The threads the choice lands on, taken when the picker opened. */
  keys: string[];
  /** What the popover hangs off: the focused row, or the bar the button is on. */
  anchor: HTMLElement | null;
  phase: Phase;
  show: (keys: string[], anchor: HTMLElement | null) => void;
  hide: () => void;
  choose: (kind: SnoozeKind, returnAtMs: number) => Promise<void>;
  /** On open and on every return to the foreground. Threads that are due come back. */
  evaluate: () => Promise<void>;
}

/** A snoozed thread is not loose, so it leaves the three boxes and nothing else. */
const leaves = (place: Place): boolean =>
  place === "inbox" || place === "feed" || place === "paper-trail";

const reloadList = (): void => void useMail.getState().load();

export const useSnooze = create<SnoozeState>((set, get) => ({
  open: false,
  keys: [],
  anchor: null,
  phase: "idle",

  show: (keys, anchor) => {
    if (keys.length === 0) return;
    // The four times are the person's, not this module's, and the picker cannot name a moment
    // until they are here. Asked for on the way up so the popover has them by the time it paints.
    if (useSettings.getState().settings === null) void useSettings.getState().load();
    set({ open: true, keys, anchor });
  },

  hide: () => set({ open: false, keys: [], anchor: null }),

  choose: async (kind, returnAtMs) => {
    const keys = get().keys;
    if (keys.length === 0) return;
    set({ open: false, keys: [], anchor: null, phase: "saving" });

    const mail = useMail.getState();
    const going = leaves(mail.place);
    const taken = going ? mail.take(keys) : [];
    if (!going) mail.patch(keys, { snoozedUntil: returnAtMs });
    if (going) useSelection.getState().clear();

    try {
      const undo = await snoozeSet(keys, kind, returnAtMs);
      set({ phase: "idle" });
      acknowledge(undo, reloadList);
    } catch (e) {
      if (going) useMail.getState().untake(taken);
      else mail.patch(keys, { snoozedUntil: null });
      set({ phase: "idle" });
      notify(`That did not go through: ${e}`);
    }
  },

  evaluate: async () => {
    if (!live()) return;
    try {
      const back = await snoozeEvaluate();
      // Nothing was due, so nothing moved and nothing is said. A snooze that returns is the Back
      // group appearing, which is the whole of the announcement.
      if (back.length > 0) void useMail.getState().load();
    } catch {
      // A pass that could not run is a pass that runs again on the next foreground, and there is
      // nothing here a person asked for out loud.
    }
  },
}));
