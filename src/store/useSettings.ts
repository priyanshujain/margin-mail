import { create } from "zustand";
import { settingsGet, settingsSet, storageUsed, systemFonts } from "../api/settings";
import { live, type BackupSettings, type Settings, type StorageUsed } from "../ipc";
import { notify } from "./useToast";

type Phase = "idle" | "loading" | "saving" | "error";

interface SettingsState {
  /**
   * Null until the first read comes back. A settings screen drawn from a guess would show the
   * defaults for a frame and then rewrite every control with what is actually on disk.
   */
  settings: Settings | null;
  /** The families installed on this machine. One slow call, made when a picker first asks. */
  fonts: string[];
  /**
   * What each account's mirror is holding. Two sections print it and one of them decides with it,
   * so it is read once here rather than twice from two components.
   */
  storage: StorageUsed[];
  /**
   * Whether settings has the stage. It is a place rather than a panel, so it is not in
   * `useOverlays`: the palette opens over it and Escape gives the stage back.
   */
  open: boolean;
  phase: Phase;
  error: string | null;
  show: () => void;
  close: () => void;
  load: () => Promise<void>;
  loadFonts: () => Promise<void>;
  loadStorage: () => Promise<void>;
  /** A partial patch. Rust merges it into the file; absent fields are unchanged. */
  save: (patch: Partial<Settings>) => Promise<void>;
  /**
   * The backup commands each answer with the whole of `BackupSettings`, so what they hand back is
   * taken rather than read again. They do not go through `settingsSet` and there is no
   * `store-changed` scope the settings screen listens to, so nothing else would notice.
   */
  applyBackup: (backup: BackupSettings) => void;
}

export const useSettings = create<SettingsState>((set, get) => ({
  settings: null,
  fonts: [],
  storage: [],
  open: false,
  phase: "idle",
  error: null,

  show: () => set({ open: true }),
  close: () => set({ open: false }),

  load: async () => {
    if (!live()) return;
    set({ phase: "loading" });
    try {
      set({ settings: await settingsGet(), phase: "idle", error: null });
    } catch (e) {
      set({ phase: "error", error: String(e) });
      notify(`Could not read your settings: ${e}`);
    }
  },

  loadFonts: async () => {
    if (!live() || get().fonts.length > 0) return;
    try {
      set({ fonts: await systemFonts() });
    } catch {
      // A machine that will not list its fonts still has the six bundled ones, which is a picker
      // with less in it rather than a screen that cannot be used.
    }
  },

  loadStorage: async () => {
    if (!live()) return;
    try {
      set({ storage: await storageUsed() });
    } catch (e) {
      notify(`Could not measure what is on this device: ${e}`);
    }
  },

  save: async (patch) => {
    const before = get().settings;
    if (!live() || !before) return;
    // Optimistic, because choosing a face has already repainted the window: a control that snapped
    // back for a frame would read as the app arguing about what you just picked.
    set({ settings: { ...before, ...patch }, phase: "saving" });
    try {
      set({ settings: await settingsSet(patch), phase: "idle", error: null });
    } catch (e) {
      set({ settings: before, phase: "error", error: String(e) });
      notify(`Could not save that setting: ${e}`);
    }
  },

  applyBackup: (backup) => set((s) => (s.settings ? { settings: { ...s.settings, backup } } : {})),
}));
