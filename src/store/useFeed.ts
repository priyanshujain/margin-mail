import { create } from "zustand";
import { threadHydrate, threadView } from "../api/threads";
import { live, type ThreadView } from "../ipc";

/**
 * The Feed's cards: which one the keyboard is on, which are open, and the body of each.
 *
 * The rows themselves are `useMail`'s, because the Feed is a place and a place's page comes from
 * `threads_list` like every other. What is the Feed's own is that a row here is not a row: it is an
 * open message, and an open message needs a `thread_view`.
 *
 * They are fetched one card at a time as the column reaches them rather than all at once, and that
 * is the whole reason this store exists. `thread_view` is a read of the mirror and never waits on
 * the network, so a body the mirror has not cached comes back empty and `bodyPending`, and it is
 * `thread_hydrate` that goes to the provider for it. A Feed of two hundred newsletters is two
 * hundred of those, so asking for the page would be two hundred round trips to Gmail for the four
 * cards that fit on a screen.
 *
 * There is no read state here and no counts. Time is the only order, and the only thing the Feed
 * remembers between visits is the hairline, which is `src/leftOff.ts` and not this.
 */
interface FeedState {
  /** Only bodies that arrived. A view still short of one is held as a failure, never as a view. */
  views: Record<string, ThreadView>;
  phase: Record<string, "loading" | "error">;
  /** The cards showing their whole body. Everything else clips at `--feed-clip`. */
  expanded: string[];
  focused: string | null;
  /**
   * The instant the newest card on screen carried when this place was last left, read once when
   * the column arrives. Held rather than watched, because a marker that moved while you read would
   * be a hairline that walks down the page.
   */
  marker: number | null;

  /** Called when the column mounts, with what `src/leftOff.ts` had. */
  arrive: (marker: number | null) => void;
  /** Asks for a card's body, once. The column calls this as a card comes near the viewport. */
  want: (key: string) => void;
  /** Asks again, from the line on a card that said its body did not arrive. */
  retry: (key: string) => void;
  focus: (key: string | null) => void;
  step: (keys: string[], delta: number) => void;
  toggle: (key: string) => void;
}

/**
 * A card's body: what the mirror has, and when that is a body it has not fetched yet, the fetch
 * and a second read behind it. Without the second half a card whose body was not cached was an
 * empty frame for the whole session, because the view was held and nothing ever asked again. A
 * body still missing after the fetch is a failure the card can say and retry from, not a view.
 */
async function loadCard(key: string): Promise<void> {
  useFeed.setState((s) => ({ phase: { ...s.phase, [key]: "loading" } }));
  try {
    let view = await threadView(key);
    if (view.messages.some((m) => m.bodyPending)) {
      await threadHydrate(key);
      view = await threadView(key);
    }
    const still = view.messages.some((m) => m.bodyPending);
    useFeed.setState((s) => {
      const phase = { ...s.phase };
      if (still) return { phase: { ...phase, [key]: "error" } };
      delete phase[key];
      return { views: { ...s.views, [key]: view }, phase };
    });
  } catch {
    // The card draws the failure and the way to ask again, and `call` has already written the
    // error down. A Feed of two hundred cards offline would otherwise be two hundred toasts.
    useFeed.setState((s) => ({ phase: { ...s.phase, [key]: "error" } }));
  }
}

export const useFeed = create<FeedState>((set, get) => ({
  views: {},
  phase: {},
  expanded: [],
  focused: null,
  marker: null,

  arrive: (marker) => set({ marker }),

  want: (key) => {
    if (!live() || get().views[key] || get().phase[key]) return;
    void loadCard(key);
  },

  retry: (key) => {
    if (!live() || get().phase[key] === "loading") return;
    void loadCard(key);
  },

  focus: (key) => set({ focused: key }),

  step: (keys, delta) => {
    if (keys.length === 0) return;
    const at = keys.indexOf(get().focused ?? "");
    // From nowhere, `j` takes the newest card and `k` takes the oldest.
    const next = at === -1 ? (delta > 0 ? 0 : keys.length - 1) : at + delta;
    if (next < 0 || next >= keys.length) return;
    set({ focused: keys[next] });
  },

  toggle: (key) =>
    set((s) => ({
      focused: key,
      expanded: s.expanded.includes(key)
        ? s.expanded.filter((k) => k !== key)
        : [...s.expanded, key],
    })),
}));
