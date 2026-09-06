import { create } from "zustand";
import { clipDelete, clipsList, filesList } from "../api/notes";
import { undoToken } from "../api/undo";
import { live, type Clip, type FileCard, type Undo } from "../ipc";
import { useMail } from "./useMail";
import { notify } from "./useToast";

/**
 * The two libraries: every passage saved out of a message, and every attachment in the mirror.
 *
 * One store rather than two because they are one idea, which is the mailbox read sideways: not
 * threads in date order but the things inside them. Neither fetches anything from the provider.
 * All files is the local index, and a card is opened before anything is downloaded.
 */
type Phase = "idle" | "loading" | "error";

/** The filter row, in the order it prints. The empty id is every type at once. */
export const CATEGORIES: { id: string; label: string }[] = [
  { id: "", label: "Everything" },
  { id: "images", label: "Images" },
  { id: "pdfs", label: "PDFs" },
  { id: "documents", label: "Documents" },
  { id: "spreadsheets", label: "Spreadsheets" },
  { id: "invites", label: "Invites" },
  { id: "other", label: "Other" },
];

interface LibraryState {
  clips: Clip[];
  clipsPhase: Phase;

  files: FileCard[];
  filesPhase: Phase;
  /** One of `CATEGORIES`, or the empty string for every type. */
  category: string;
  /** An address, or the empty string for everyone. */
  sender: string;

  loadClips: () => Promise<void>;
  loadFiles: () => Promise<void>;
  setCategory: (category: string) => void;
  setSender: (sender: string) => void;
  removeClip: (id: string) => Promise<void>;
}

export const useLibrary = create<LibraryState>((set, get) => ({
  clips: [],
  clipsPhase: "idle",

  files: [],
  filesPhase: "idle",
  category: "",
  sender: "",

  loadClips: async () => {
    if (!live()) return;
    const accountId = useMail.getState().accountId;
    set({ clipsPhase: "loading" });
    try {
      const clips = await clipsList(accountId);
      if (useMail.getState().accountId !== accountId) return;
      set({ clips, clipsPhase: "idle" });
    } catch (e) {
      set({ clipsPhase: "error" });
      notify(`Could not read your clips: ${e}`);
    }
  },

  loadFiles: async () => {
    if (!live()) return;
    const accountId = useMail.getState().accountId;
    const { category, sender } = get();
    set({ filesPhase: "loading" });
    try {
      const files = await filesList(accountId, category, sender);
      // A filter that has changed under the answer is a different question, and the answer to the
      // one being asked now is already on its way.
      if (get().category !== category || get().sender !== sender) return;
      set({ files, filesPhase: "idle" });
    } catch (e) {
      set({ filesPhase: "error" });
      notify(`Could not read your files: ${e}`);
    }
  },

  setCategory: (category) => {
    set({ category });
    void get().loadFiles();
  },

  setSender: (sender) => {
    set({ sender });
    void get().loadFiles();
  },

  removeClip: async (id) => {
    const before = get().clips;
    set({ clips: before.filter((clip) => clip.id !== id) });
    try {
      acknowledge(await clipDelete(id));
    } catch (e) {
      set({ clips: before });
      notify(`That did not go through: ${e}`);
    }
  },
}));

function acknowledge(undo: Undo): void {
  notify(undo.label, {
    label: "Undo",
    keycap: "z",
    run: () => {
      void undoToken(undo.token)
        .then(() => void useLibrary.getState().loadClips())
        .catch((e) => notify(`Could not undo that: ${e}`));
    },
  });
}
