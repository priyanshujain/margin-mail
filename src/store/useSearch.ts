import { create } from "zustand";
import { search, searchProvider } from "../api/search";
import { live, type Place, type ThreadSummary } from "../ipc";
import { useMail } from "./useMail";
import { useSelection } from "./useSelection";
import { notify } from "./useToast";

/**
 * The query, the answer, and the way back.
 *
 * Search is not a place you navigate to: it takes the stage from the place you were in and gives
 * it back on Escape, so this store remembers what it took. The rows themselves go to `useMail`,
 * because a result behaves like any other row and `j`, `x` and `e` must not have to know which
 * list they are walking.
 *
 * The operators are Rust's to parse. Nothing here reads the query beyond trimming it, and
 * `SearchBar` prints what it finds without claiming to understand it.
 */
export type SearchPhase = "off" | "idle" | "searching" | "error";

/** The place search took the stage from, and the selection it put aside. */
interface Origin {
  place: Place;
  labelId: string | null;
  labelName: string | null;
  keys: string[];
  anchor: string | null;
}

interface SearchState {
  /** `off` is the icon in the header. Anything else is the field, open, with a caret in it. */
  phase: SearchPhase;
  query: string;
  /** What the result page says about itself: how short the local answer may be. */
  note: string | null;
  providerSearched: boolean;
  error: string | null;
  from: Origin | null;

  open: () => void;
  setQuery: (query: string) => void;
  run: () => Promise<void>;
  more: () => Promise<void>;
  /** "Search older mail on Gmail" at the foot of the list. */
  askProvider: () => Promise<void>;
  close: () => void;
}

/** A provider's hit for a thread that is already on the device is the same row twice. */
const merge = (had: ThreadSummary[], next: ThreadSummary[]): ThreadSummary[] => {
  const seen = new Set(had.map((t) => t.key));
  return [...had, ...next.filter((t) => !seen.has(t.key))];
};

export const useSearch = create<SearchState>((set, get) => ({
  phase: "off",
  query: "",
  note: null,
  providerSearched: false,
  error: null,
  from: null,

  open: () => {
    if (get().phase !== "off") return;
    const mail = useMail.getState();
    const selection = useSelection.getState();
    set({
      phase: "idle",
      query: "",
      note: null,
      providerSearched: false,
      error: null,
      from: {
        place: mail.place,
        labelId: mail.labelId,
        labelName: mail.labelName,
        keys: selection.keys,
        anchor: selection.anchor,
      },
    });
  },

  setQuery: (query) => set({ query }),

  run: async () => {
    const query = get().query.trim();
    const from = get().from;
    if (!live() || !from) return;

    // An emptied field gives the place back without closing the search: you are still typing.
    if (!query) {
      if (useMail.getState().place === "search") {
        useMail.getState().resume(from);
        set({ note: null, providerSearched: false, phase: "idle" });
      }
      return;
    }

    // Taking the stage puts the selection aside: it was made in another list and a bulk verb here
    // would act on rows nobody can see.
    if (useMail.getState().place !== "search") useSelection.getState().clear();

    const accountId = useMail.getState().accountId;
    set({ phase: "searching", providerSearched: false });
    try {
      const result = await search(accountId, query, null);
      // An older query's answer is not this field's answer.
      if (get().query.trim() !== query) return;
      set({
        phase: "idle",
        note: result.note,
        providerSearched: result.providerSearched,
        error: null,
      });
      useMail.getState().showResults(result.page.threads, result.page.nextCursor);
    } catch (e) {
      set({ phase: "error", error: String(e) });
      notify(`Could not search: ${e}`);
    }
  },

  more: async () => {
    const cursor = useMail.getState().nextCursor;
    const query = get().query.trim();
    if (!live() || !cursor || !query || get().phase === "searching") return;
    if (useMail.getState().place !== "search") return;
    set({ phase: "searching" });
    try {
      const result = await search(useMail.getState().accountId, query, cursor);
      if (get().query.trim() !== query) return;
      set({ phase: "idle", note: result.note });
      const mail = useMail.getState();
      mail.showResults(merge(mail.threads, result.page.threads), result.page.nextCursor);
    } catch (e) {
      set({ phase: "error", error: String(e) });
      // The rows already here are still right; it is the next page that did not come, and a
      // scroll that stops short with no word reads as the end of the answer.
      notify(`Could not load more results: ${e}`);
    }
  },

  askProvider: async () => {
    const accountId = useMail.getState().accountId;
    const query = get().query.trim();
    // Every account at once is every provider at once, and search is per account until the unified
    // view is proven. The button is not offered there, and this is the same answer twice.
    if (!live() || !accountId || !query) return;
    set({ phase: "searching" });
    try {
      const result = await searchProvider(accountId, query);
      if (get().query.trim() !== query) return;
      set({ phase: "idle", providerSearched: true, note: result.note, error: null });
      const mail = useMail.getState();
      mail.showResults(merge(mail.threads, result.page.threads), result.page.nextCursor);
    } catch (e) {
      set({ phase: "error", error: String(e) });
      notify(`Could not search Gmail: ${e}`);
    }
  },

  close: () => {
    const from = get().from;
    const showing = useMail.getState().place === "search";
    set({ phase: "off", query: "", note: null, providerSearched: false, error: null, from: null });
    if (!from || !showing) return;
    useMail.getState().resume(from);
    useSelection.getState().restore(from.keys, from.anchor);
  },
}));
