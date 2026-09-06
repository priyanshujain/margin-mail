import { create } from "zustand";
import { contactCard, contactUpdate, contactsList } from "../api/contacts";
import { undoToken } from "../api/undo";
import { unsubscribe as unsubscribeFrom } from "../api/write";
import { live, type ContactCard, type ContactPatch, type Undo } from "../ipc";
import { useMail } from "./useMail";
import { notify } from "./useToast";

/**
 * The card that is open, and the Contacts place's list.
 *
 * The anchor is in the store, which is unusual and deliberate. A name is drawn in the list, in the
 * pane, on a Feed card, on a Screener card and on a compose chip, and every one of them has to be
 * able to ask for the card that hangs off it. Threading a callback from `ContactCards` down through
 * five components would make all five know about contacts to reach a popover that is mounted once
 * at the top of the tree, so instead a screen calls `show` with the node it drew and the host does
 * the rest. The element is a DOM reference rather than state, but it is the request, and the
 * request is what a store holds.
 */
type Phase = "idle" | "loading" | "error";

export type UnsubscribePhase = "idle" | "unsubscribing" | "error";

/** Long enough that a fast typist sends one query rather than eight, short enough to feel live. */
const SETTLE_MS = 140;

interface ContactsState {
  anchor: HTMLElement | null;
  address: string | null;
  accountId: string | null;
  card: ContactCard | null;
  phase: Phase;

  /** The Contacts place: what was typed, and who answered. */
  query: string;
  /** Null until the first list has come back, so the first open can wait rather than say nobody. */
  people: ContactCard[] | null;
  listPhase: Phase;

  /**
   * Per address, because the same sender is a button on the contact card and on every one of
   * their Feed cards at once, and all of them have to say the same thing while the POST is out.
   */
  unsubscribePhase: Record<string, UnsubscribePhase>;

  /** Asks for the card for one address, hanging off the element that was clicked. */
  show: (address: string, accountId: string, anchor: HTMLElement | null) => Promise<void>;
  hide: () => void;
  /** Writes one or more fields and shows the change before the call comes back. */
  save: (address: string, accountId: string, patch: ContactPatch) => Promise<void>;
  /** The RFC 8058 one-click where the sender offers it, the mailto or the page where not. */
  unsubscribe: (accountId: string, address: string) => Promise<void>;
  setQuery: (query: string) => void;
  load: () => Promise<void>;
}

/** The toast an unsubscribe leaves behind, carrying the token that reverses that one action. */
function acknowledge(undo: Undo): void {
  notify(undo.label, {
    label: "Undo",
    keycap: "z",
    run: () => {
      void undoToken(undo.token)
        .then(() => void useMail.getState().load())
        .catch((e) => notify(`Could not undo that: ${e}`));
    },
  });
}

/** One query per pause rather than one per keystroke, the way the header's search does it. */
let settle: number | undefined;

/** The optimistic half of a patch: what the card looks like the instant a control is used. */
const applied = (card: ContactCard, patch: ContactPatch): ContactCard => ({
  ...card,
  ...(patch.destination === undefined ? {} : { destination: patch.destination }),
  ...(patch.domainRule === undefined ? {} : { domainRule: patch.domainRule }),
  ...(patch.notify === undefined ? {} : { notify: patch.notify }),
  ...(patch.note === undefined ? {} : { note: patch.note }),
  ...(patch.allowRemoteImages === undefined
    ? {}
    : { allowRemoteImages: patch.allowRemoteImages }),
  ...(patch.autoTrashDays === undefined ? {} : { autoTrashDays: patch.autoTrashDays }),
  ...(patch.bundle === undefined ? {} : { bundle: patch.bundle }),
});

export const useContacts = create<ContactsState>((set, get) => ({
  anchor: null,
  address: null,
  accountId: null,
  card: null,
  phase: "idle",

  query: "",
  people: null,
  listPhase: "idle",

  unsubscribePhase: {},

  show: async (address, accountId, anchor) => {
    // Asking for the card that is already open closes it, so the same name is a toggle and `i`
    // pressed twice puts you back where you were.
    if (get().address === address && get().anchor === anchor) {
      get().hide();
      return;
    }
    set({ anchor, address, accountId, card: null, phase: "loading" });
    if (!live()) return;
    try {
      const card = await contactCard(accountId, address);
      if (get().address !== address) return;
      set({ card, phase: "idle" });
    } catch (e) {
      if (get().address !== address) return;
      notify(`Could not open that contact: ${e}`);
      // An empty card hanging off the name would be a card that is still coming, and it is not.
      get().hide();
    }
  },

  hide: () => set({ anchor: null, address: null, accountId: null, card: null, phase: "idle" }),

  save: async (address, accountId, patch) => {
    set((s) => ({
      card: s.card && s.card.person.address === address ? applied(s.card, patch) : s.card,
      people:
        s.people &&
        s.people.map((person) =>
          person.person.address === address && person.accountId === accountId
            ? applied(person, patch)
            : person,
        ),
    }));
    if (!live()) return;
    try {
      await contactUpdate(accountId, address, patch);
      // A destination carries a screening date the frontend cannot invent, so the card is asked
      // again once the write has landed rather than guessed at.
      if (patch.destination !== undefined || patch.domainRule !== undefined) {
        const card = await contactCard(accountId, address);
        set((s) => ({
          card: s.card && s.card.person.address === address ? card : s.card,
          people:
            s.people &&
            s.people.map((person) =>
              person.person.address === address && person.accountId === accountId
                ? { ...card, recentThreads: person.recentThreads, files: person.files }
                : person,
            ),
        }));
      }
    } catch (e) {
      notify(`Could not save that: ${e}`);
      // The optimistic change was wrong, so put back whatever is actually there.
      void get().load();
      try {
        const card = await contactCard(accountId, address);
        set((s) => (s.address === address ? { card } : {}));
      } catch {
        // The backend that refused the write is the one being asked, so a second failure is the
        // same failure and the toast has already said it once.
      }
    }
  },

  // The two follow-ups docs/features.md has the confirmation offer, trashing what is here and
  // screening the sender out, are not offered yet, so neither is taken: this unsubscribes and
  // nothing else, and the toast is the way back.
  unsubscribe: async (accountId, address) => {
    if (!live() || get().unsubscribePhase[address] === "unsubscribing") return;
    const mark = (phase: UnsubscribePhase) =>
      set((s) => ({ unsubscribePhase: { ...s.unsubscribePhase, [address]: phase } }));
    mark("unsubscribing");
    try {
      const undo = await unsubscribeFrom(accountId, address, false, false);
      mark("idle");
      acknowledge(undo);
    } catch (e) {
      mark("error");
      notify(`Could not unsubscribe: ${e}`);
    }
  },

  setQuery: (query) => {
    set({ query });
    window.clearTimeout(settle);
    settle = window.setTimeout(() => void get().load(), SETTLE_MS);
  },

  load: async () => {
    if (!live()) return;
    const query = get().query;
    // The place follows the account the header is on, the way every other list does.
    const accountId = useMail.getState().accountId;
    set({ listPhase: "loading" });
    try {
      const people = await contactsList(accountId, query);
      if (get().query !== query) return;
      set({ people, listPhase: "idle" });
    } catch (e) {
      set({ listPhase: "error" });
      notify(`Could not load your contacts: ${e}`);
    }
  },
}));
