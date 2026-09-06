import { create } from "zustand";
import { logNote } from "../api/log";
import { threadHydrate, threadOpened, threadsList, threadView } from "../api/threads";
import { labelsList } from "../api/labels";
import { applyPane, initialPane } from "../pane";
import { seenInPlace } from "./seen";
import {
  live,
  type LabelInfo,
  type Place,
  type ThreadSummary,
  type ThreadView,
} from "../ipc";
import { notify } from "./useToast";

/**
 * An open slower than this is written down. The mirror answers in a few milliseconds and the
 * pane is drawn from the row before the answer comes, so a number here is either a render an
 * older sanitiser left behind or a connection the engine was holding, and the log is where the
 * next "opening an email lags" report finds out which.
 */
const SLOW_OPEN_MS = 250;

/**
 * Where you are and what is in front of you: the place, the account, the page of rows, the row the
 * keyboard is on, and the thread that is open.
 *
 * Rows arrive from `src/api/threads.ts` and from nowhere else. They arrive already grouped and
 * already ordered, so nothing here sorts, and nothing here decides what a group is called.
 */
const PAGE = 50;

type Phase = "idle" | "loading" | "error";

/** A row that a triage action took out of the list, and where it was, so a failure can put it back. */
export interface Taken {
  at: number;
  thread: ThreadSummary;
}

interface MailState {
  place: Place;
  /** Null is All accounts, which is what the query means by an absent account. */
  accountId: string | null;
  /** The provider's label the list is showing, when the place is one. */
  labelId: string | null;
  labelName: string | null;
  /** The provider's labels for this account. They are places, so they belong to this store. */
  labels: LabelInfo[];
  /** An empty label list reads as none only once a read has come back, which is what this says. */
  labelsPhase: "idle" | "loading" | "error";
  threads: ThreadSummary[];
  nextCursor: string | null;
  /** The quiet line at the foot of the list, when the place has one. */
  footer: string | null;
  phase: Phase;
  /**
   * The next page, on its own rather than a value of `phase`. A reload that comes in behind a
   * `store-changed` and a scroll that reaches the foot are two different things happening at
   * once, and while they shared a phase the second was dropped whenever the first was out.
   */
  paging: "idle" | "paging" | "error";
  error: string | null;
  /** The row the keyboard is on. Also the row drawn as selected: there is only one such state. */
  focused: string | null;
  /** The thread in the pane, or in place of the list when the pane is hidden. */
  openKey: string | null;
  thread: ThreadView | null;
  /**
   * The row the pane draws its head from until the view lands: the subject, the sender and the
   * people are all already in memory, so none of them has to wait on a read. Null when the thread
   * was opened from somewhere that has no row for it, which is the only case the pane goes blank.
   */
  opening: ThreadSummary | null;
  threadPhase: "idle" | "loading" | "error";
  /**
   * The bodies the open thread is still short of, by message id, once a fetch has been asked for.
   *
   * `thread_hydrate` answers with a count and not a failure when the provider refuses every body,
   * so a message still `bodyPending` after it has settled is `error` here, which is what turns the
   * skeleton into a line that says so. `loading` is the same slot asked for again from that line.
   * Absent means nobody has settled anything yet, which draws the same skeleton.
   */
  bodyPhase: Record<string, "loading" | "error">;
  pane: boolean;

  load: () => Promise<void>;
  loadMore: () => Promise<void>;
  loadLabels: () => Promise<void>;
  goTo: (place: Place) => void;
  /** A label is a place with a name, so it is reached with the label rather than with the id. */
  goToLabel: (label: LabelInfo) => void;
  /** Returns to a place the search took the stage from, keeping whatever is open in the pane. */
  resume: (to: { place: Place; labelId: string | null; labelName: string | null }) => void;
  /** The list is a page of search results now. Search owns the query; this owns the rows. */
  showResults: (threads: ThreadSummary[], cursor: string | null) => void;
  setAccount: (accountId: string | null) => void;
  focus: (key: string | null) => void;
  /** Moves the focus by one row, and picks up the first row when nothing is focused yet. */
  step: (delta: number) => void;
  /** The optimistic half of a triage action: the row changes before the call goes out. */
  patch: (keys: string[], changes: Partial<ThreadSummary>) => void;
  /** Takes rows out of the list, handing back what was taken and where it was. */
  take: (keys: string[]) => Taken[];
  /** Puts back what `take` removed, at the indexes it came from. */
  untake: (taken: Taken[]) => void;
  open: (key?: string) => Promise<void>;
  /**
   * The open thread, read again in place. Nothing else changes: no phase, no clearing, no scroll.
   *
   * This is what a body arriving behind an open thread calls, from the `store-changed` listener in
   * `src/App.tsx` and from the hydration `open` starts, and a page that flashed every time one
   * landed would be worse than the empty paragraph it replaced. Does nothing with no thread open.
   */
  refreshThread: () => Promise<void>;
  /** The open thread's missing bodies, asked for again. What Try again on a failed slot presses. */
  hydrateThread: () => Promise<void>;
  /** The slots belong to the thread that was open, so a new one starts with none. */
  clearBodyPhase: () => void;
  close: () => void;
  togglePane: () => void;
}

export const useMail = create<MailState>((set, get) => ({
  place: "inbox",
  accountId: null,
  labelId: null,
  labelName: null,
  labels: [],
  labelsPhase: "idle",
  threads: [],
  nextCursor: null,
  footer: null,
  phase: "idle",
  paging: "idle",
  error: null,
  focused: null,
  openKey: null,
  thread: null,
  opening: null,
  threadPhase: "idle",
  bodyPhase: {},
  pane: initialPane(),

  load: async () => {
    if (!live()) return;
    const { place, accountId, labelId } = get();
    // Search results are the search store's list: it holds the query, and asking `threads_list`
    // for a place called search without one would answer with the whole mailbox.
    if (place === "search") return;
    set({ phase: "loading" });
    try {
      const page = await threadsList({ accountId, place, labelId, limit: PAGE, cursor: null });
      // A place that came back while you were already somewhere else is not this list's answer.
      if (get().place !== place || get().accountId !== accountId || get().labelId !== labelId) return;
      set({
        threads: page.threads,
        nextCursor: page.nextCursor,
        footer: page.footer,
        phase: "idle",
        error: null,
        // A row that is no longer in the list cannot keep the focus, and one that is keeps it, so
        // a refresh after a sync does not throw you back to the top.
        focused: page.threads.some((t) => t.key === get().focused) ? get().focused : null,
      });
    } catch (e) {
      set({ phase: "error", error: String(e) });
      notify(`Could not load the list: ${e}`);
    }
  },

  loadMore: async () => {
    const { place, accountId, labelId, nextCursor, paging } = get();
    if (!live() || !nextCursor || paging === "paging") return;
    if (place === "search") return;
    set({ paging: "paging" });
    try {
      const page = await threadsList({ accountId, place, labelId, limit: PAGE, cursor: nextCursor });
      // A page that came back for a list you have already left is not this list's next page.
      if (get().place !== place || get().accountId !== accountId || get().labelId !== labelId) {
        set({ paging: "idle" });
        return;
      }
      set((s) => ({
        threads: [...s.threads, ...page.threads],
        nextCursor: page.nextCursor,
        footer: page.footer,
        paging: "idle",
      }));
    } catch {
      // The foot of the list says so and offers the page again, and `call` has written the error
      // down. The list above it is still the list, so nothing else changes.
      set({ paging: "error" });
    }
  },

  loadLabels: async () => {
    if (!live()) return;
    const accountId = get().accountId;
    set({ labelsPhase: "loading" });
    try {
      const labels = await labelsList(accountId);
      if (get().accountId !== accountId) return;
      set({ labels, labelsPhase: "idle" });
    } catch {
      // Not worth a toast: nothing asked for them out loud. The picker reads the phase, so a list
      // that failed no longer reads as a mailbox with no labels on it.
      set({ labelsPhase: "error" });
    }
  },

  goTo: (place) => {
    if (get().place === place && place !== "label") return;
    set({
      place,
      labelId: null,
      labelName: null,
      threads: [],
      nextCursor: null,
      footer: null,
      focused: null,
      openKey: null,
      thread: null,
      opening: null,
    });
    void get().load();
  },

  goToLabel: (label) => {
    if (get().place === "label" && get().labelId === label.id) return;
    set({
      place: "label",
      labelId: label.id,
      labelName: label.name,
      threads: [],
      nextCursor: null,
      footer: null,
      focused: null,
      openKey: null,
      thread: null,
      opening: null,
    });
    void get().load();
  },

  resume: (to) => {
    set({
      place: to.place,
      labelId: to.labelId,
      labelName: to.labelName,
      threads: [],
      nextCursor: null,
      footer: null,
      focused: null,
    });
    void get().load();
  },

  showResults: (threads, cursor) =>
    set({
      place: "search",
      labelId: null,
      labelName: null,
      threads,
      nextCursor: cursor,
      footer: null,
      phase: "idle",
      error: null,
      focused: threads.some((t) => t.key === get().focused) ? get().focused : null,
    }),

  setAccount: (accountId) => {
    if (get().accountId === accountId) return;
    set({
      accountId,
      labelId: null,
      labelName: null,
      labels: [],
      threads: [],
      nextCursor: null,
      footer: null,
      focused: null,
      openKey: null,
      thread: null,
      opening: null,
    });
    void get().load();
  },

  focus: (key) => set({ focused: key }),

  step: (delta) => {
    const { threads, focused } = get();
    if (threads.length === 0) return;
    const at = threads.findIndex((t) => t.key === focused);
    // From nowhere, `j` takes the first row and `k` takes the last.
    const next = at === -1 ? (delta > 0 ? 0 : threads.length - 1) : at + delta;
    if (next < 0 || next >= threads.length) return;
    set({ focused: threads[next].key });
  },

  patch: (keys, changes) =>
    set((s) => {
      const wanted = new Set(keys);
      if (!s.threads.some((t) => wanted.has(t.key))) return {};
      return { threads: s.threads.map((t) => (wanted.has(t.key) ? { ...t, ...changes } : t)) };
    }),

  take: (keys) => {
    const wanted = new Set(keys);
    const taken: Taken[] = [];
    const { threads, focused } = get();
    threads.forEach((thread, at) => {
      if (wanted.has(thread.key)) taken.push({ at, thread });
    });
    if (taken.length === 0) return taken;
    const left = threads.filter((t) => !wanted.has(t.key));
    // The hand is already on the next thread, so the focus goes to whatever took the row's place
    // rather than back to the top of the list.
    const next =
      focused && wanted.has(focused)
        ? (left[Math.min(taken[0].at, left.length - 1)]?.key ?? null)
        : focused;
    set({ threads: left, focused: next });
    return taken;
  },

  untake: (taken) =>
    set((s) => {
      if (taken.length === 0) return {};
      const threads = s.threads.slice();
      // Ascending, so each row lands on the index it was taken from rather than one short of it.
      for (const { at, thread } of [...taken].sort((a, b) => a.at - b.at)) {
        if (threads.some((t) => t.key === thread.key)) continue;
        threads.splice(Math.min(at, threads.length), 0, thread);
      }
      return { threads };
    }),

  open: async (key) => {
    const target = key ?? get().focused;
    if (!target || !live()) return;
    // Asking for the thread that is already open is a re-read after a note or a rename, so it keeps
    // what is on screen. A different thread does not: leaving the last one drawn under the row you
    // just clicked is what "opening an email takes seconds" was, and the row already carries enough
    // to draw the head of the new one on this frame.
    const again = get().openKey === target;
    set({
      openKey: target,
      focused: target,
      threadPhase: "loading",
      thread: again ? get().thread : null,
      opening: get().threads.find((t) => t.key === target) ?? null,
    });
    try {
      const began = performance.now();
      const view = await threadView(target);
      const took = performance.now() - began;
      if (took > SLOW_OPEN_MS) void logNote("open", `thread_view took ${Math.round(took)} ms for ${target}`);
      if (get().openKey !== target) return;
      set({ thread: view, opening: null, threadPhase: "idle" });
      // Opening marks the thread seen: the row first, so the dot goes on this frame, and then the
      // mirror behind it. A re-read of the thread already open is not a second opening.
      const wasUnseen = get().threads.some((t) => t.key === target && t.unseen);
      set((s) => ({ threads: seenInPlace(s.threads, target) }));
      if (!again) void markSeen(target, wasUnseen);
      // The view is what the mirror already had. Anything it was short of is fetched behind the
      // open thread and read again when it lands, so the pane is never waiting on the network.
      if (view.messages.some((m) => m.bodyPending)) void hydrate(target);
    } catch (e) {
      set({ threadPhase: "error" });
      notify(`Could not open the thread: ${e}`);
    }
  },

  refreshThread: async () => {
    const key = get().openKey;
    if (!key || !live()) return;
    try {
      const view = await threadView(key);
      if (get().openKey !== key) return;
      set({ thread: view, opening: null });
    } catch {
      // Nothing asked for this out loud, and the thread on screen is still the thread on screen.
      // Whatever failed will be said by whichever call the person actually made.
    }
  },

  hydrateThread: async () => {
    const key = get().openKey;
    if (key) await hydrate(key);
  },

  clearBodyPhase: () =>
    set((s) => (Object.keys(s.bodyPhase).length === 0 ? {} : { bodyPhase: {} })),

  close: () => set({ openKey: null, thread: null, opening: null, threadPhase: "idle" }),

  togglePane: () => {
    const pane = !get().pane;
    applyPane(pane);
    set({ pane });
  },
}));

/**
 * The mirror's half of opening: every message seen, the provider told, the badge moved. Not a verb
 * and not undoable, so nothing comes back and nothing is said. A failure puts the dot back where
 * there was one, because a row without it over a thread the mirror still holds unseen is exactly
 * the disagreement this exists to end.
 */
async function markSeen(key: string, wasUnseen: boolean): Promise<void> {
  try {
    await threadOpened(key);
  } catch {
    if (wasUnseen) useMail.getState().patch([key], { unseen: true });
  }
}

/**
 * The bodies an open thread was short of, fetched behind it.
 *
 * The command's answer is a count rather than the bodies, so the thread is read again instead of
 * patched: what to draw is the mirror's to say, not this call's. The backend also emits
 * `store-changed` when they land, and both paths end in the same silent re-read, so a body that
 * arrives while another one is still coming is not a race.
 *
 * The re-read happens whatever the count was. Zero is what comes back when the provider refused
 * every body and when there is no provider attached at all, neither of which is an error to the
 * command, and a skeleton that breathes for ever was what a zero used to leave behind. Whatever is
 * still pending after the re-read is marked as failed so the pane can say so.
 */
async function hydrate(key: string): Promise<void> {
  // Only a slot already marked failed changes here: a first pass draws the same skeleton with or
  // without an entry, and this is what takes a pressed Try again out of its failed line.
  useMail.setState((s) => {
    const retried = (s.thread?.messages ?? [])
      .filter((m) => m.bodyPending && s.bodyPhase[m.id] === "error")
      .map((m) => [m.id, "loading" as const]);
    return retried.length === 0 ? {} : { bodyPhase: { ...s.bodyPhase, ...Object.fromEntries(retried) } };
  });
  try {
    await threadHydrate(key);
  } catch (e) {
    // Worth saying out loud, unlike the silent refresh: what is on screen is a thread with a
    // paragraph missing from it, and nothing else will explain the gap.
    if (useMail.getState().openKey === key) notify(`Could not fetch the rest of this thread: ${e}`);
  }
  if (useMail.getState().openKey !== key) return;
  await useMail.getState().refreshThread();
  if (useMail.getState().openKey !== key) return;
  const still = (useMail.getState().thread?.messages ?? []).filter((m) => m.bodyPending);
  useMail.setState({ bodyPhase: Object.fromEntries(still.map((m) => [m.id, "error" as const])) });
}
