import {
  call,
  type FlagPatch,
  type Pile,
  type SnoozeKind,
  type ThreadPage,
  type ThreadQuery,
  type ThreadView,
  type Undo,
} from "../ipc";

export const threadsList = (query: ThreadQuery) => call<ThreadPage>("threads_list", { query });

export const threadView = (key: string) => call<ThreadView>("thread_view", { key });

/**
 * Fetches the bodies `thread_view` came back without, and answers with how many arrived. The view
 * is read again rather than patched from this, because what a message renders as is the mirror's
 * answer and not this call's.
 */
export const threadHydrate = (key: string) => call<number>("thread_hydrate", { key });

/**
 * Opening is what marks a thread seen, and this is the mirror being told. Nothing comes back: it
 * is not a verb, it is not on the undo stack, and a thread already seen is a no-op on the far side.
 */
export const threadOpened = (key: string) => call<void>("thread_opened", { key });

/** Seen, starred, archived, trashed and spam are the provider's flags, so these go through it. */
export const flagsSet = (keys: string[], patch: FlagPatch) =>
  call<Undo>("flags_set", { keys, patch });

export const markAllSeen = (accountId: string | null, place: string) =>
  call<Undo>("mark_all_seen", { accountId, place });

/**
 * The only bulk write to the provider the app ever proposes: mark everything older than a date as
 * seen. Reversible for seven days, which is what the token is for.
 */
export const startFresh = (accountId: string, olderThanMs: number) =>
  call<Undo>("start_fresh", { accountId, olderThanMs });

export const pileToggle = (keys: string[], pile: Pile) => call<Undo>("pile_toggle", { keys, pile });

export const snoozeSet = (keys: string[], kind: SnoozeKind, returnAtMs: number) =>
  call<Undo>("snooze_set", { keys, kind, returnAtMs });

export const snoozeClear = (keys: string[]) => call<Undo>("snooze_clear", { keys });

/** Runs on open, on foreground and on wake. Returns the threads that came back. */
export const snoozeEvaluate = () => call<string[]>("snooze_evaluate");

export const threadRename = (key: string, name: string | null) =>
  call<Undo>("thread_rename", { key, name });

/** The merged thread takes the first key in the list, which is also the one Unmerge undoes from. */
export const threadMerge = (keys: string[], name: string | null) =>
  call<Undo>("thread_merge", { keys, name });

export const threadUnmerge = (key: string) => call<Undo>("thread_unmerge", { key });

export const threadIgnore = (keys: string[], on: boolean) =>
  call<Undo>("thread_ignore", { keys, on });

export const threadNotify = (keys: string[], on: boolean) =>
  call<Undo>("thread_notify", { keys, on });
