import { create } from "zustand";
import { placeCounts, placeVisited } from "../api/counts";
import type { Place, PlaceCounts } from "../ipc";

interface CountsState {
  counts: PlaceCounts[];
  load: () => Promise<void>;
  visited: (accountId: string | null, place: Place) => Promise<void>;
}

export const useCounts = create<CountsState>((set, get) => ({
  counts: [],

  load: async () => {
    try {
      set({ counts: await placeCounts() });
    } catch {
      // The dots are a hint. A failed count leaves the last one standing rather than clearing it.
    }
  },

  visited: async (accountId, place) => {
    if (place !== "feed" && place !== "paper-trail") return;
    const field = place === "feed" ? "feed" : "paperTrail";
    set({
      counts: get().counts.map((c) =>
        accountId === null || c.accountId === accountId ? { ...c, [field]: false } : c,
      ),
    });
    try {
      await placeVisited(accountId, place);
    } catch {
      void get().load();
    }
  },
}));

/** The counts for one account, or every account summed when `accountId` is null. */
export function countsFor(counts: PlaceCounts[], accountId: string | null) {
  const mine = accountId === null ? counts : counts.filter((c) => c.accountId === accountId);
  return {
    inbox: mine.reduce((n, c) => n + c.inbox, 0),
    screener: mine.reduce((n, c) => n + c.screener, 0),
    feed: mine.some((c) => c.feed),
    paperTrail: mine.some((c) => c.paperTrail),
  };
}
