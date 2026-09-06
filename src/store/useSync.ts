import { create } from "zustand";
import { syncNow, syncStatus } from "../api/sync";
import { live, type SyncStatus } from "../ipc";
import { notify } from "./useToast";

interface SyncState {
  /** One entry per account. Nothing in the app aggregates these into a single state. */
  statuses: SyncStatus[];
  phase: "idle" | "syncing" | "error";
  error: string | null;
  /** Replaces one account's status, which is what a `sync-progress` event carries. */
  apply: (status: SyncStatus) => void;
  refresh: () => Promise<void>;
  run: (accountId?: string) => Promise<void>;
}

const replace = (statuses: SyncStatus[], next: SyncStatus): SyncStatus[] => {
  const at = statuses.findIndex((s) => s.accountId === next.accountId);
  if (at === -1) return [...statuses, next];
  const out = statuses.slice();
  out[at] = next;
  return out;
};

export const useSync = create<SyncState>((set, get) => ({
  statuses: [],
  phase: "idle",
  error: null,
  apply: (status) => set((s) => ({ statuses: replace(s.statuses, status) })),
  refresh: async () => {
    if (!live()) return;
    try {
      set({ statuses: await syncStatus() });
    } catch (e) {
      set({ error: String(e) });
    }
  },
  run: async (accountId) => {
    if (!live() || get().phase === "syncing") return;
    set({ phase: "syncing", error: null });
    try {
      for (const status of await syncNow(accountId)) get().apply(status);
      set({ phase: "idle" });
    } catch (e) {
      set({ phase: "error", error: String(e) });
      notify(`Sync failed: ${e}`);
    }
  },
}));

/**
 * The one quiet line the header carries while a pass is working, and null while none is.
 *
 * `trouble` says what is wrong; this says what is happening. The two are not the same question and
 * a mailbox that has just been connected spends its first minutes answering the second one: rows
 * arrive before bodies do, and an app that looks finished while it is still filling in reads as an
 * app that has lost half your mail.
 *
 * Whatever the engine chose to call it, in the engine's own words. Nothing here maps a phase to a
 * sentence, because the sentence belongs with the work: the pass that is warming the body cache
 * knows it is caching recent mail and this only has to be able to print it.
 */
export function busy(statuses: SyncStatus[], accountId: string | null): string | null {
  const mine = accountId ? statuses.filter((s) => s.accountId === accountId) : statuses;
  const working = mine.find((s) => s.phase !== "idle" && s.message);
  if (!working?.message) return null;
  if (working.total <= 0) return working.message;
  // Clamped, because a count that has caught up with a total that has not yet been raised by the
  // mail which arrived during the pass would otherwise read as more than all of it.
  const done = Math.min(working.hydrated, working.total);
  return `${working.message} ${done.toLocaleString()} of ${working.total.toLocaleString()}`;
}

/** The phases in which an account has stopped, as opposed to resting between passes. */
const STUCK = new Set<SyncStatus["phase"]>(["offline", "error", "paused"]);

/** What the panel over a newly connected account does next. */
export type Arrival = "working" | "stalled" | "done";

/**
 * Whether an account that has just been connected has arrived.
 *
 * Done is a pass that ended clean: the engine stamps the last sync only at the end of one, and it
 * ends the first one only after the whole crawl and the seed behind it, so a stamped status resting
 * in idle, or already caching bodies, is an Inbox with all of its mail in it. The statuses a pass
 * emits on its way carry no stamp on a new account, and a first pass that failed quietly ends idle
 * without one, which is right: the next poll picks the crawl up and the panel is still the truth.
 * A stopped account is stalled, and says so with the engine's sentence rather than a bar that
 * never fills. No status at all is a first pass that has not reported yet.
 */
export function arrival(status: SyncStatus | null): Arrival {
  if (!status) return "working";
  if (STUCK.has(status.phase)) return "stalled";
  if (status.lastSyncMs !== null && (status.phase === "idle" || status.phase === "caching")) {
    return "done";
  }
  return "working";
}

/** The phases that are a fill: the mirror is short of what the mailbox holds and is catching up. */
const FILLING = new Set<SyncStatus["phase"]>(["syncing", "hydrating", "backfilling"]);

/** What an empty list says while the account behind it is still being brought in. */
export interface Fill {
  accountId: string;
  message: string;
  hydrated: number;
  total: number;
}

const ARRIVING = "Bringing in your mail";

/**
 * The fill that is running for the accounts on screen, and null when none is.
 *
 * This is what stands in for "Nothing here" on a mailbox that has not arrived yet, and the case it
 * exists for is the one that shipped: an account connected from Settings sat on an empty Inbox with
 * no sentence about why, because nothing told the list that empty and not-yet-fetched were two
 * different things.
 *
 * Two readings of a status count as a fill. The engine saying so, in its own words, which is a
 * first sync's crawl, a crawl resumed after a quit, or a backfill: a phase from the set above with a
 * sentence attached, since an ordinary poll runs under "syncing" too and carries none. And an
 * account resting between passes that has never finished one: idle, nothing wrong, and no last
 * sync, which is a crawl a tunnel interrupted waiting for the next poll to pick it up. A stopped
 * account is not a fill whatever its history, because the chip has the word for that and a bar
 * that never moves is worse than an empty list that explains itself. Nor is an account with no
 * status at all: the engine reports every account it holds, so no status is an account it does
 * not, and a bar over that would never fill either.
 */
export function filling(statuses: SyncStatus[], accountIds: string[]): Fill | null {
  for (const accountId of accountIds) {
    const status = statuses.find((s) => s.accountId === accountId);
    if (!status) continue;
    if (FILLING.has(status.phase) && status.message) {
      return {
        accountId,
        message: status.message,
        hydrated: Math.min(status.hydrated, status.total),
        total: status.total,
      };
    }
    if (status.phase === "idle" && !status.error && status.lastSyncMs === null) {
      return { accountId, message: ARRIVING, hydrated: 0, total: 0 };
    }
  }
  return null;
}

/**
 * The word the account chip carries when something is wrong, and null when nothing is.
 *
 * The louder of the two lines, and the one that wins the slot: a mailbox that cannot reach the
 * server has nothing useful to say about what it is doing meanwhile.
 *
 * For a stopped account the word is the engine's, because only the engine sees the error's kind and
 * the kind is the whole question. "Signed out" is a token Google refused and nothing else; a label
 * change Gmail answered 400 to used to print the same two words on an account that was signed in
 * perfectly well, because every non-network failure shared one phase and this guessed from it.
 */
export function trouble(statuses: SyncStatus[], accountId: string | null): string | null {
  const mine = accountId ? statuses.filter((s) => s.accountId === accountId) : statuses;
  const stuck = mine.find((s) => STUCK.has(s.phase));
  if (!stuck) return null;
  if (stuck.phase === "offline") return "Offline";
  if (stuck.phase === "paused") return "Paused";
  return stuck.message ?? "Sync trouble";
}

/**
 * The chip words that are also worth a toast, because a person can do something about them and
 * nothing else will: sign the account in again, or grant the permission it is missing. The engine
 * decides the word and this only recognises it, which is the one place the two sides share a
 * string rather than a field, so it is written down here.
 */
const ACTIONABLE = new Set(["Signed out", "Needs permission"]);

/**
 * Whether a status is worth a toast, and what the toast says. `last` is what was last said for
 * this account; the same trouble reported at the end of every pass is said once, and a pass that
 * ends clean forgets it, so the same trouble coming back after a recovery is news again.
 *
 * Almost nothing is. This is Mailspring's rule, and it is why nobody using Mailspring has seen a
 * sync error: a failure that the next poll will answer is the chip's business, not a toast's.
 * Offline, a rate limit and "Sync trouble" are carried by the chip and pass on their own. A pause
 * is news, in the engine's sentence, because it is the one thing a person can do something about
 * by pressing sync; so are a refused token and a missing permission, because nothing else mends
 * them. A write the provider refused arrives mid-pass with the phase still `syncing` and its error
 * already a whole sentence, and is said once because the change did not take.
 */
export function announce(
  status: SyncStatus,
  last: string | null,
): { toast: string | null; last: string | null } {
  if (!status.error) {
    const settled = status.phase === "idle" || status.phase === "caching";
    return { toast: null, last: settled ? null : last };
  }
  let text: string;
  if (status.phase === "paused") {
    text = status.message ?? `Sync paused: ${status.error}`;
  } else if (STUCK.has(status.phase)) {
    if (!status.message || !ACTIONABLE.has(status.message)) return { toast: null, last };
    text = `${status.message}: ${status.error}`;
  } else {
    text = status.error;
  }
  if (text === last) return { toast: null, last };
  return { toast: text, last: text };
}
