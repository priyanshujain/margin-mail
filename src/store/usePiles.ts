import { create } from "zustand";
import { pileToggle, threadsList } from "../api/threads";
import { live, type Pile, type Place, type ThreadSummary } from "../ipc";
import { acknowledge } from "../screens/triage";
import { useMail } from "./useMail";
import { useSelection } from "./useSelection";
import { notify } from "./useToast";

/**
 * What is on the two piles.
 *
 * The piles at the foot of the list are not a slice of the list above them: a thread on a pile is
 * exactly the thread the Inbox is no longer showing, so the stack has to be its own query. It is
 * the same query the place behind `4` and `5` runs, which is why the whole pile is held here
 * rather than only its top card: Focus & Reply is a page over this list and nothing else.
 */
const PAGE = 50;

type Phase = "idle" | "loading" | "error";

export const PILE_NAMES: Record<Pile, string> = {
  "reply-later": "Reply later",
  "set-aside": "Set aside",
};

const OTHER: Record<Pile, Pile> = {
  "reply-later": "set-aside",
  "set-aside": "reply-later",
};

/**
 * Whether a row still belongs in the place it is being looked at, once its pile is what it is
 * about to be.
 *
 * The three boxes hold loose threads only, so piling one takes it out of them; a pile's own place
 * holds exactly its pile, so unpiling takes it out of that. Everything else (Everything, Starred, a
 * label, a search) is indifferent, and a row that vanished from Everything for being piled would
 * read as a bug rather than as a rule.
 */
export function belongsInPlace(place: Place, pile: Pile | null): boolean {
  if (place === "inbox" || place === "feed" || place === "paper-trail") return pile === null;
  if (place === "reply-later" || place === "set-aside") return pile === place;
  return true;
}

interface PilesState {
  /** Each pile's threads, in the order its place lists them. */
  threads: Record<Pile, ThreadSummary[]>;
  phase: Phase;
  load: () => Promise<void>;
  /** `l` and `s`: on if any of them is off, off otherwise, which is what Rust decides too. */
  toggle: (keys: string[], pile: Pile) => Promise<void>;
}

/** An undone pile move is a row back in the list and a card back on the stack, so both are asked. */
const reloadBoth = (): void => {
  void useMail.getState().load();
  void usePiles.getState().load();
};

export const usePiles = create<PilesState>((set, get) => ({
  threads: { "reply-later": [], "set-aside": [] },
  phase: "idle",

  load: async () => {
    if (!live()) return;
    const accountId = useMail.getState().accountId;
    set({ phase: "loading" });
    try {
      const [later, aside] = await Promise.all([
        threadsList({ accountId, place: "reply-later", limit: PAGE, cursor: null }),
        threadsList({ accountId, place: "set-aside", limit: PAGE, cursor: null }),
      ]);
      // A pile that came back while you were already on another account is not this pile's answer.
      if (useMail.getState().accountId !== accountId) return;
      set({
        threads: { "reply-later": later.threads, "set-aside": aside.threads },
        phase: "idle",
      });
    } catch (e) {
      set({ phase: "error" });
      notify(`Could not read the piles: ${e}`);
    }
  },

  toggle: async (keys, pile) => {
    if (keys.length === 0) return;
    const mail = useMail.getState();
    const rows = mail.threads.filter((t) => keys.includes(t.key));
    const onPile = new Set(get().threads[pile].map((t) => t.key));
    // The same answer Rust reaches, from the same question: anything not on the pile puts the
    // whole selection on it. Reading it any other way sends the row the wrong way for a frame.
    const adding = rows.length > 0 ? rows.some((t) => t.pile !== pile) : keys.some((k) => !onPile.has(k));
    const next = adding ? pile : null;

    const going = !belongsInPlace(mail.place, next);
    const before = rows.map((row) => ({ key: row.key, pile: row.pile }));
    const taken = going ? mail.take(keys) : [];
    if (!going) mail.patch(keys, { pile: next });
    // A bulk action consumes the selection it acted on: the rows are gone and a checkbox over
    // nothing is a state you have to press Escape to get out of.
    if (going) useSelection.getState().clear();

    // The stack at the foot of the list moves with the row, because they are the same thread and
    // watching one of them wait for a round trip is watching the app think.
    const moved = rows.length > 0 ? rows : get().threads[OTHER[pile]].filter((t) => keys.includes(t.key));
    set((s) => {
      const drop = (list: ThreadSummary[]) => list.filter((t) => !keys.includes(t.key));
      const added = adding ? moved.map((t) => ({ ...t, pile })) : [];
      return {
        threads: {
          ...s.threads,
          [pile]: [...added, ...drop(s.threads[pile])].sort((a, b) => b.dateMs - a.dateMs),
          [OTHER[pile]]: drop(s.threads[OTHER[pile]]),
        },
      };
    });

    try {
      const undo = await pileToggle(keys, pile);
      // The optimistic guess got the membership right; what it cannot know is the order the
      // place would have put it in, so the pile is asked once the write has landed.
      void get().load();
      acknowledge(undo, reloadBoth);
    } catch (e) {
      if (going) useMail.getState().untake(taken);
      else for (const row of before) useMail.getState().patch([row.key], { pile: row.pile });
      void get().load();
      notify(`That did not go through: ${e}`);
    }
  },
}));
