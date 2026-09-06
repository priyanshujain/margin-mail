import { create } from "zustand";
import { screenerClearAll, screenerDecide, screenerList } from "../api/screener";
import { threadHydrate, threadView } from "../api/threads";
import { undoLast } from "../api/undo";
import { live, type Destination, type ScreenerCard, type ThreadView } from "../ipc";
import { acknowledge } from "../screens/triage";
import { notify } from "./useToast";

/**
 * The senders waiting at the door.
 *
 * A card is identified by its thread key rather than by its address, because All accounts can hold
 * two people at one address and the key is the only thing that is unique in that list.
 *
 * A decision holds its card where it is, buttons down, until the call comes back, and only then
 * does the card leave and the focus move on. It used to leave at once, and the next card took its
 * exact place under the pointer, so a double-click on Yes decided two senders. A failure lets go
 * of the card and says so. Clear all is still optimistic: nothing takes anything's place there.
 */

/**
 * Domains where "everyone at this domain" is not a group, copied from `CONSUMER_DOMAINS` in
 * src-tauri/src/state/write.rs.
 *
 * Copied rather than fetched because there is no command that serves it, and the two have to agree:
 * `state::write::set_rule` returns an error for a domain rule on any of these, so a toggle offered
 * here that Rust refuses is a decision that appears to have been taken and was not. If a domain is
 * added there, add it here.
 */
const CONSUMER_DOMAINS = new Set([
  "aol.com",
  "fastmail.com",
  "gmail.com",
  "gmx.com",
  "gmx.de",
  "googlemail.com",
  "hey.com",
  "hotmail.co.uk",
  "hotmail.com",
  "icloud.com",
  "live.com",
  "mac.com",
  "mail.com",
  "msn.com",
  "me.com",
  "outlook.com",
  "pm.me",
  "proton.me",
  "protonmail.com",
  "yahoo.co.uk",
  "yahoo.com",
  "yandex.ru",
]);

export const domainOf = (address: string): string =>
  address.slice(address.indexOf("@") + 1).trim().toLowerCase();

/** Whether the picker may offer "everyone at this domain" for an address. */
export const domainRuleAllowed = (address: string): boolean => {
  const domain = domainOf(address);
  return domain.length > 0 && !CONSUMER_DOMAINS.has(domain);
};

/** The four destinations, in the order the picker lists them. Screened out is `n`, not a choice. */
export const DESTINATIONS: { destination: Destination; label: string }[] = [
  { destination: "inbox", label: "Inbox" },
  { destination: "feed", label: "Feed" },
  { destination: "paper-trail", label: "Paper Trail" },
];

export const destinationName = (destination: Destination): string =>
  DESTINATIONS.find((d) => d.destination === destination)?.label ?? "Screened out";

type Phase = "idle" | "loading" | "error";

interface ScreenerState {
  cards: ScreenerCard[];
  phase: Phase;
  error: string | null;
  /** The thread key of the card the keyboard is on. */
  focused: string | null;
  /** The one card showing its whole message, if any. */
  expanded: string | null;
  /** The bodies of the cards that have been expanded, kept so a second Enter is instant. */
  views: Record<string, ThreadView>;
  viewPhase: Record<string, "loading" | "error">;
  /** The cards whose decision is out, still on the pile with their buttons down. */
  deciding: string[];

  load: (accountId: string | null) => Promise<void>;
  focus: (key: string | null) => void;
  step: (delta: number) => void;
  toggleExpanded: (key: string) => void;
  /** Asks for an expanded card's message again, from the line that said it did not arrive. */
  retryView: (key: string) => void;
  decide: (key: string, destination: Destination, wholeDomain: boolean) => Promise<void>;
  clearAll: (accountId: string | null) => Promise<void>;
  undo: (accountId: string | null) => Promise<void>;
}

/**
 * A card's whole message: what the mirror has, and when that is a body it has not fetched yet,
 * the fetch and a second read behind it. `thread_view` never waits on the network, so without the
 * second half an expanded card was an empty frame over the Reply button for the whole session. A
 * body that is still missing after the fetch is held as a failure rather than as a view, so the
 * card says so and the next Enter asks again.
 */
async function loadView(key: string): Promise<void> {
  useScreener.setState((s) => ({ viewPhase: { ...s.viewPhase, [key]: "loading" } }));
  try {
    let view = await threadView(key);
    if (view.messages.some((m) => m.bodyPending)) {
      await threadHydrate(key);
      view = await threadView(key);
    }
    const still = view.messages.some((m) => m.bodyPending);
    useScreener.setState((s) => {
      const viewPhase = { ...s.viewPhase };
      if (still) return { viewPhase: { ...viewPhase, [key]: "error" } };
      delete viewPhase[key];
      return { views: { ...s.views, [key]: view }, viewPhase };
    });
  } catch {
    // The card draws the failure and the way to ask again, and `call` has already written the
    // error down, so a toast would be the same sentence a third time.
    useScreener.setState((s) => ({ viewPhase: { ...s.viewPhase, [key]: "error" } }));
  }
}

export const useScreener = create<ScreenerState>((set, get) => ({
  cards: [],
  phase: "idle",
  error: null,
  focused: null,
  expanded: null,
  views: {},
  viewPhase: {},
  deciding: [],

  load: async (accountId) => {
    if (!live()) return;
    set({ phase: "loading" });
    try {
      const cards = await screenerList(accountId);
      set({
        cards,
        phase: "idle",
        error: null,
        // A sender who has been decided cannot keep the keyboard, and one who is still waiting does.
        focused: cards.some((c) => c.threadKey === get().focused) ? get().focused : null,
        expanded: cards.some((c) => c.threadKey === get().expanded) ? get().expanded : null,
      });
    } catch (e) {
      set({ phase: "error", error: String(e) });
      notify(`Could not load the Screener: ${e}`);
    }
  },

  focus: (key) => set({ focused: key }),

  step: (delta) => {
    const { cards, focused } = get();
    if (cards.length === 0) return;
    const at = cards.findIndex((c) => c.threadKey === focused);
    const next = at === -1 ? (delta > 0 ? 0 : cards.length - 1) : at + delta;
    if (next < 0 || next >= cards.length) return;
    set({ focused: cards[next].threadKey });
  },

  toggleExpanded: (key) => {
    if (get().expanded === key) {
      set({ expanded: null });
      return;
    }
    set({ expanded: key, focused: key });
    if (get().views[key] || get().viewPhase[key] === "loading" || !live()) return;
    void loadView(key);
  },

  retryView: (key) => {
    if (get().viewPhase[key] === "loading" || !live()) return;
    void loadView(key);
  },

  decide: async (key, destination, wholeDomain) => {
    const card = get().cards.find((c) => c.threadKey === key);
    if (!card || get().deciding.includes(key)) return;
    set((s) => ({ deciding: [...s.deciding, key] }));

    try {
      const undo = await screenerDecide(card.accountId, card.sender.address, destination, wholeDomain);
      set((s) => {
        const at = s.cards.findIndex((c) => c.threadKey === key);
        const left = s.cards.filter((c) => c.threadKey !== key);
        // Dealing cards: whatever takes this card's place takes the keyboard with it, so the hand
        // stays where it was rather than going back to the top of the pile.
        const next = left[Math.min(Math.max(at, 0), left.length - 1)]?.threadKey ?? null;
        return {
          cards: left,
          deciding: s.deciding.filter((k) => k !== key),
          focused: s.focused === key ? next : s.focused,
          expanded: s.expanded === key ? null : s.expanded,
        };
      });
      acknowledge(undo, () => void get().load(card.accountId));
    } catch (e) {
      set((s) => ({ deciding: s.deciding.filter((k) => k !== key) }));
      notify(`That did not go through: ${e}`);
    }
  },

  clearAll: async (accountId) => {
    const cards = get().cards;
    if (cards.length === 0) return;
    set({ cards: [], focused: null, expanded: null });
    try {
      acknowledge(await screenerClearAll(accountId), () => void get().load(accountId));
    } catch (e) {
      set({ cards });
      notify(`That did not go through: ${e}`);
    }
  },

  undo: async (accountId) => {
    try {
      const label = await undoLast();
      // Nothing to take back is not an error and does not get a toast: the key did nothing.
      if (!label) return;
      await get().load(accountId);
      notify(`Undone: ${label}`);
    } catch (e) {
      notify(`Could not undo that: ${e}`);
    }
  },
}));
