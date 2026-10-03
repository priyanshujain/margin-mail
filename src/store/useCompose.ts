import { create } from "zustand";
import { undoToken } from "../api/undo";
import { draftDelete, draftGet, draftImport, draftList, draftSave, outboxList, send, sendNow } from "../api/write";
import {
  live,
  type Draft,
  type DraftAttachment,
  type MessageView,
  type Person,
  type ThreadView,
  type Undo,
} from "../ipc";
import { useAccounts } from "./useAccounts";
import { useMail } from "./useMail";
import { useSettings } from "./useSettings";
import { notify, useToast } from "./useToast";
import { signatureHtml } from "../signature";

/**
 * Everything being written, and the one send that has not gone yet.
 *
 * There are two places a message is written and they are the same thing twice: the card that floats
 * over the list, and the box under the last message of a thread. Both hold a `Draft`, both save on
 * the same debounce and both send down the same pipeline, so they are one shape with a name saying
 * where it is drawn rather than two half-composers that drift apart.
 *
 * A closed card is parked rather than thrown away. Escape means "not now", `Cmd+Shift+,` means
 * discard, and a mail client that loses four paragraphs to the wrong key is a mail client you write
 * in somewhere else and paste into.
 */

/** Gmail's ceiling, quoted so an attachment can be refused before a round trip rather than after. */
const LIMIT_BYTES = 35 * 1024 * 1024;

/** Base64 costs a third on top, which is what "encoded size" means and why a 30 MB file will not go. */
const ENCODED = 4 / 3;

/** Long enough that a sentence is one save, short enough that a draft is never more than a line behind. */
const SAVE_MS = 800;

export type ComposerAt = "card" | "reply";
export type ComposerKind = "new" | "reply" | "forward";

/** What Instant intro moved, so pressing it again puts it back exactly. */
export interface Intro {
  person: Person;
  line: string;
}

export interface Composer {
  at: ComposerAt;
  kind: ComposerKind;
  draft: Draft;
  /** The heading over it: "New message", or who it answers. */
  title: string;
  phase: "idle" | "saving" | "error";
  /** What the last save said the message weighs, and whether that is already too much. */
  encodedSize: number;
  overLimit: boolean;
  /** Cc and Bcc are behind a word until somebody wants them. */
  showCc: boolean;
  intro: Intro | null;
  /**
   * Nothing has been changed since it opened. A reply that is opened and abandoned must not leave a
   * draft behind: the signature it was seeded with is not something anybody wrote.
   */
  pristine: boolean;
  /** Whether the recipients are everyone's, which is what `a` switches. */
  all: boolean;
  /** The two answers `r` and `a` mean, worked out when the box opened so the switch is instant. */
  sender: Person[];
  everyone: Person[];
  everyoneCc: Person[];
}

/** A send inside its delay: what it was, and what taking it back would put back on the screen. */
interface Holding {
  token: string;
  label: string;
  endsAtMs: number;
  draft: Draft;
  at: ComposerAt;
  title: string;
  threadKey: string | null;
}

interface ComposeState {
  drafts: Draft[];
  loadDrafts: () => Promise<void>;
  resumeDraft: (draft: Draft) => void;
  openDraft: (accountId: string, messageId: string) => Promise<void>;
  deleteDraft: (id: string) => Promise<boolean>;
  card: Composer | null;
  /** The card, closed but not thrown away. */
  parked: Composer | null;
  /** The card takes the whole window, which is `Cmd+Shift+P`. */
  expanded: boolean;
  reply: Composer | null;
  /** The thread the reply box is under. A different thread is a different box. */
  replyKey: string | null;
  /** The box, closed but not thrown away, and the thread it belongs to. */
  parkedReply: Composer | null;
  parkedReplyKey: string | null;
  holding: Holding | null;
  /** Send now is a round trip to the provider for every held send, and the line says so. */
  flushPhase: "idle" | "sending" | "error";

  /** `c`. The parked draft when there is one, a blank message when there is not. */
  compose: () => void;
  closeCard: () => Promise<void>;
  toggleExpanded: () => void;

  /** `r`, `a` and `f`, from the pane. */
  answer: (thread: ThreadView, message: MessageView, kind: ComposerKind, all: boolean) => void;
  /** `a` again, on a box that is already open. */
  setAll: (all: boolean) => void;
  closeReply: () => Promise<void>;

  edit: (at: ComposerAt, changes: Partial<Draft>) => void;
  setShowCc: (at: ComposerAt, show: boolean) => void;
  /** Refuses anything that would not fit and says which file it was. */
  attach: (at: ComposerAt, files: DraftAttachment[]) => void;
  detach: (at: ComposerAt, index: number) => void;
  /** `Cmd+Shift+I`, and `Cmd+Shift+I` again. */
  instantIntro: (at: ComposerAt) => void;
  /** `Cmd+Shift+H`. Null turns it off. */
  remind: (at: ComposerAt, atMs: number | null) => void;

  save: (at: ComposerAt) => Promise<void>;
  discard: (at: ComposerAt) => Promise<void>;
  /** `Cmd+Enter`, and `Cmd+Shift+Enter` when `now`. */
  post: (at: ComposerAt, now: boolean) => Promise<void>;
  /** `z` and the toast, inside the delay: the send is cancelled and the draft comes back. */
  undoSend: () => Promise<void>;
  /** The Send now beside "Waiting to send": whatever is holding goes out at once. */
  flush: () => Promise<void>;
  /** The delay has run out. The send is Rust's now and there is nothing left to take back. */
  settle: () => void;
}

const composerAt = (state: ComposeState, at: ComposerAt): Composer | null =>
  at === "card" ? state.card : state.reply;

const put = (at: ComposerAt, composer: Composer | null) =>
  at === "card" ? { card: composer } : { reply: composer };

/**
 * Whichever copy of this composer is the live one, and how to write it back.
 *
 * A card that has been closed is still saving, so the debounce that fires after Escape has to find
 * the parked draft rather than deciding there is nothing to save.
 */
function held(
  state: ComposeState,
  at: ComposerAt,
): { composer: Composer; write: (next: Composer) => Partial<ComposeState> } | null {
  const onScreen = composerAt(state, at);
  if (onScreen) return { composer: onScreen, write: (next) => put(at, next) };
  const parked = at === "card" ? state.parked : state.parkedReply;
  if (!parked) return null;
  return {
    composer: parked,
    write: (next) => (at === "card" ? { parked: next } : { parkedReply: next }),
  };
}

const blank = (at: ComposerAt, kind: ComposerKind, title: string, draft: Draft): Composer => ({
  at,
  kind,
  draft,
  title,
  phase: "idle",
  encodedSize: 0,
  overLimit: false,
  showCc: (draft.cc?.length ?? 0) > 0 || (draft.bcc?.length ?? 0) > 0,
  intro: null,
  pristine: true,
  all: false,
  sender: [],
  everyone: [],
  everyoneCc: [],
});

const sizeOf = (draft: Draft): number =>
  (draft.attachments ?? []).reduce((total, file) => total + file.size * ENCODED, 0);

const named = (person: Person): string => person.name?.trim() || person.address;

/** The account a new message goes out from: the one being looked at, or the first one there is. */
function sendingAccount(): string | null {
  const looking = useMail.getState().accountId;
  if (looking) return looking;
  return useAccounts.getState().accounts[0]?.id ?? null;
}

function signatureFor(accountId: string): string {
  const settings = useSettings.getState().settings;
  const signature = settings?.accounts.find((a) => a.accountId === accountId)?.signature ?? "";
  return signature ? `<p></p>${signatureHtml(signature)}` : "<p></p>";
}

/** The people a reply goes to, with you and the sender taken out of the extra ones. */
function others(people: Person[], mine: Set<string>, sender: string): Person[] {
  const seen = new Set<string>([sender.toLowerCase()]);
  const out: Person[] = [];
  for (const person of people) {
    const address = person.address.toLowerCase();
    if (mine.has(address) || seen.has(address)) continue;
    seen.add(address);
    out.push(person);
  }
  return out;
}

const escapeHtml = (text: string): string =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

// -------------------------------------------------------------------------------------------
// The debounce and the countdown
//
// Both are timers rather than state, so they are held beside the store rather than in it: a store
// that re-rendered every screen once a second because a toast is counting down would be a store
// nothing else could subscribe to.
// -------------------------------------------------------------------------------------------

const saveTimers: Record<ComposerAt, number> = { card: 0, reply: 0 };
const saveRequests: Partial<Record<ComposerAt, Promise<void>>> = {};
const attachmentRequests: Record<ComposerAt, Set<Promise<void>>> = { card: new Set(), reply: new Set() };

export function storeAttachments(at: ComposerAt, work: Promise<void>): Promise<void> {
  attachmentRequests[at].add(work);
  const clear = () => attachmentRequests[at].delete(work);
  void work.then(clear, clear);
  return work;
}

function scheduleSave(at: ComposerAt): void {
  if (typeof window === "undefined") return;
  window.clearTimeout(saveTimers[at]);
  saveTimers[at] = window.setTimeout(() => void useCompose.getState().save(at), SAVE_MS);
}

function cancelSave(at: ComposerAt): void {
  if (typeof window === "undefined") return;
  window.clearTimeout(saveTimers[at]);
  saveTimers[at] = 0;
}

let beat = 0;

function stopBeat(): void {
  if (typeof window === "undefined") return;
  window.clearInterval(beat);
  beat = 0;
}

/**
 * The toast, said again every second with one fewer on it.
 *
 * `useToast` holds a sentence rather than a node, so a countdown is a new sentence rather than a
 * component that ticks. Saying it again also restarts the dismissal timer, which is what keeps the
 * toast on screen for the whole of a thirty second delay.
 */
function tick(): void {
  const holding = useCompose.getState().holding;
  if (!holding) {
    stopBeat();
    return;
  }
  const left = Math.ceil((holding.endsAtMs - Date.now()) / 1000);
  if (left <= 0) {
    useCompose.getState().settle();
    return;
  }
  notify(`${holding.label} · ${left} s`, {
    label: "Undo",
    keycap: "z",
    run: () => void useCompose.getState().undoSend(),
  });
}

function startBeat(): void {
  if (typeof window === "undefined") return;
  stopBeat();
  tick();
  beat = window.setInterval(tick, 1000);
}

export const useCompose = create<ComposeState>((set, get) => ({
  drafts: [],
  loadDrafts: async () => {
    if (!live()) return;
    try { set({ drafts: await draftList() }); } catch (e) { notify(`Could not load drafts: ${e}`); }
  },
  resumeDraft: (draft) => {
    const current = get().card;
    if (current?.draft.id === draft.id) return;
    if (current) {
      notify("Close the current composer before opening another draft");
      return;
    }
    if (get().reply?.draft.id === draft.id) return;
    set({ card: blank("card", "new", "Draft", draft), parked: null, expanded: false });
    if (get().parkedReply?.draft.id === draft.id) set({ parkedReply: null, parkedReplyKey: null });
  },
  openDraft: async (accountId, messageId) => {
    try {
      const draft = await draftImport(accountId, messageId);
      await get().loadDrafts();
      get().resumeDraft(await draftGet(draft.id!));
    } catch (e) { notify(`Could not open the draft: ${e}`); }
  },
  deleteDraft: async (id) => {
    await Promise.all(Object.values(saveRequests));
    try {
      await draftDelete(id);
      const state = get();
      if (state.card?.draft.id === id) set({ card: null, expanded: false });
      if (state.reply?.draft.id === id) set({ reply: null, replyKey: null });
      if (state.parked?.draft.id === id) set({ parked: null });
      if (state.parkedReply?.draft.id === id) set({ parkedReply: null, parkedReplyKey: null });
      await get().loadDrafts();
      void useMail.getState().load();
      void useMail.getState().refreshThread();
      return true;
    } catch (e) { notify(`Could not discard that draft: ${e}`); return false; }
  },
  card: null,
  parked: null,
  expanded: false,
  reply: null,
  replyKey: null,
  parkedReply: null,
  parkedReplyKey: null,
  holding: null,
  flushPhase: "idle",

  compose: () => {
    if (get().card) return;
    // The signature and the undo delay are both settings, and settings are only read when the
    // settings screen asks for them. Writing is the other thing that needs them.
    if (!useSettings.getState().settings) void useSettings.getState().load();

    const parked = get().parked;
    if (parked) {
      set({ card: parked, parked: null });
      return;
    }
    const accountId = sendingAccount();
    if (!accountId) return;
    set({
      card: blank("card", "new", "New message", {
        accountId,
        to: [],
        cc: [],
        bcc: [],
        subject: "",
        bodyHtml: signatureFor(accountId),
        attachments: [],
      }),
    });
  },

  closeCard: async () => {
    try { await Promise.all(attachmentRequests.card); } catch { return; }
    await get().save("card");
    const card = get().card;
    if (!card || card.phase === "error") return;
    cancelSave("card");
    set({ card: null, parked: card, expanded: false });

  },

  toggleExpanded: () => set((s) => (s.card ? { expanded: !s.expanded } : {})),

  answer: (thread, message, kind, all) => {
    const active = get().reply;
    if (active) {
      if (get().replyKey === thread.key && active.kind === kind) { get().setAll(all); return; }
      notify("Close the current reply before starting another reply or forward");
      return;
    }
    if (!useSettings.getState().settings) void useSettings.getState().load();

    // A box that was closed rather than sent comes back with what was in it, the same way the card
    // does. Without this, pressing `r` twice on one thread leaves two drafts with the provider.
    const parked = get().parkedReply;
    if (parked && get().parkedReplyKey === thread.key && parked.kind === kind) {
      set({ reply: parked, replyKey: thread.key, parkedReply: null, parkedReplyKey: null });
      if (kind !== "forward" && parked.all !== all) get().setAll(all);
      return;
    }

    const accountId = thread.accountId;
    const mine = new Set(
      useAccounts
        .getState()
        .accounts.map((a) => a.email.toLowerCase())
        .concat(
          (useSettings.getState().settings?.accounts ?? []).flatMap((a) =>
            a.aliases.map((alias) => alias.toLowerCase()),
          ),
        ),
    );

    // Reply-To is the header that says where an answer belongs, and a list that sets it means it.
    const back = message.replyTo.length > 0 ? message.replyTo : [message.from];
    const sender = message.sentByMe && message.to.length > 0 ? message.to : back;
    const everyone = others(
      [...sender, ...message.to],
      mine,
      sender[0]?.address ?? "",
    );
    const everyoneCc = others(message.cc, mine, sender[0]?.address ?? "");
    const subject = message.subject;

    const forwarding = kind === "forward";
    const prefix = forwarding ? "Fwd: " : "Re: ";
    const already = new RegExp(`^${forwarding ? "fwd:" : "re:"}\\s`, "i").test(subject);

    const quoted = forwarding
      ? `${signatureFor(accountId)}<p>Forwarded message from ${escapeHtml(named(message.from))}, ${escapeHtml(subject)}</p><blockquote>${message.html}</blockquote>`
      : signatureFor(accountId);

    const to = forwarding ? [] : all ? [...sender, ...everyone] : sender;

    const composer = blank("reply", kind, forwarding ? "Forward" : `Reply to ${named(sender[0] ?? message.from)}`, {
      accountId,
      threadKey: thread.key,
      inReplyTo: message.messageId,
      to,
      cc: forwarding || !all ? [] : everyoneCc,
      bcc: [],
      subject: already ? subject : `${prefix}${subject}`,
      bodyHtml: quoted,
      attachments: [],
    });

    set({
      reply: { ...composer, all: forwarding ? false : all, sender, everyone, everyoneCc },
      replyKey: thread.key,
    });
  },

  setAll: (all) =>
    set((s) => {
      const reply = s.reply;
      if (!reply || reply.kind === "forward" || reply.all === all) return {};
      return {
        reply: {
          ...reply,
          all,
          // Everyone else on the thread arriving in a field that is behind a word is everyone else
          // on the thread not arriving at all, as far as anybody looking at the box can tell.
          showCc: reply.showCc || (all && reply.everyoneCc.length > 0),
          draft: {
            ...reply.draft,
            to: all ? [...reply.sender, ...reply.everyone] : reply.sender,
            cc: all ? reply.everyoneCc : [],
          },
        },
      };
    }),

  closeReply: async () => {
    try { await Promise.all(attachmentRequests.reply); } catch { return; }
    await get().save("reply");
    const reply = get().reply;
    if (!reply || reply.phase === "error") return;
    cancelSave("reply");
    set({ reply: null, replyKey: null, parkedReply: reply, parkedReplyKey: get().replyKey });

  },

  edit: (at, changes) => {
    const composer = composerAt(get(), at);
    if (!composer) return;
    set(put(at, { ...composer, pristine: false, draft: { ...composer.draft, ...changes } }));
    scheduleSave(at);
  },

  setShowCc: (at, showCc) => {
    const composer = composerAt(get(), at);
    if (!composer) return;
    set(put(at, { ...composer, showCc }));
  },

  attach: (at, files) => {
    const composer = composerAt(get(), at);
    if (!composer || files.length === 0) return;
    const already = Math.max(composer.encodedSize, sizeOf(composer.draft));
    const adding = files.reduce((total, file) => total + file.size * ENCODED, 0);
    if (already + adding > LIMIT_BYTES) {
      const which = files.length === 1 ? files[0].filename : `${files.length} files`;
      notify(`${which} would put this message over the 35 MB the provider takes`);
      return;
    }
    get().edit(at, { attachments: [...(composer.draft.attachments ?? []), ...files] });
  },

  detach: (at, index) => {
    const composer = composerAt(get(), at);
    if (!composer) return;
    const attachments = (composer.draft.attachments ?? []).filter((_, i) => i !== index);
    set(put(at, { ...composer, overLimit: false }));
    get().edit(at, { attachments });
  },

  instantIntro: (at) => {
    const composer = composerAt(get(), at);
    if (!composer) return;

    if (composer.intro) {
      const { person, line } = composer.intro;
      const bcc = (composer.draft.bcc ?? []).filter((p) => p.address !== person.address);
      set(
        put(at, {
          ...composer,
          pristine: false,
          intro: null,
          draft: {
            ...composer.draft,
            to: [person, ...composer.draft.to.filter((p) => p.address !== person.address)],
            bcc,
            bodyHtml: composer.draft.bodyHtml.replace(line, ""),
          },
        }),
      );
      scheduleSave(at);
      return;
    }

    const person = composer.draft.to[0];
    if (!person) return;
    const thanks = useSettings.getState().settings?.instantIntro?.trim();
    if (!thanks) return;
    const line = `<p>${escapeHtml(thanks)}</p>`;
    set(
      put(at, {
        ...composer,
        pristine: false,
        intro: { person, line },
        showCc: true,
        draft: {
          ...composer.draft,
          to: composer.draft.to.filter((p) => p.address !== person.address),
          bcc: [...(composer.draft.bcc ?? []), person],
          bodyHtml: `${line}${composer.draft.bodyHtml}`,
        },
      }),
    );
    scheduleSave(at);
  },

  remind: (at, atMs) => get().edit(at, { remindAtMs: atMs }),

  save: async (at) => {
    const work = async () => {
      const start = held(get(), at);
      if (!start || !live()) return;
      cancelSave(at);
      const draft = start.composer.draft;
      if (start.composer.pristine) return;
      set(start.write({ ...start.composer, phase: "saving" }));
      try {
        const saved = await draftSave(draft);
        void get().loadDrafts();
        const after = held(get(), at);
        if (!after || after.composer.draft.id !== draft.id || after.composer.draft.accountId !== draft.accountId || after.composer.draft.threadKey !== draft.threadKey) return;
        const same = after.composer.draft === draft;
        set(
          after.write({
            ...after.composer,
            phase: "idle",
            draft: { ...after.composer.draft, id: saved.id },
            ...(same ? { encodedSize: saved.encodedSize, overLimit: saved.overLimit } : {}),
          }),
        );
      } catch (e) {
        const after = held(get(), at);
        if (after) set(after.write({ ...after.composer, phase: "error" }));
        notify(`Could not save the draft: ${e}`);
      }
    };
    const pending = (saveRequests[at] ?? Promise.resolve()).then(work);
    saveRequests[at] = pending;
    await pending;
    if (saveRequests[at] === pending) delete saveRequests[at];
  },

  discard: async (at) => {
    cancelSave(at);
    await saveRequests[at];
    const composer = composerAt(get(), at);
    if (!composer) return;
    if (composer.draft.id) { await get().deleteDraft(composer.draft.id); return; }
    if (at === "card") set({ card: null, parked: null, expanded: false });
    else set({ reply: null, replyKey: null, parkedReply: null, parkedReplyKey: null });
  },

  post: async (at, now) => {
    try { await Promise.all(attachmentRequests[at]); } catch { return; }
    cancelSave(at);
    await saveRequests[at];
    const composer = composerAt(get(), at);
    if (!composer) return;
    if (composer.draft.to.length === 0) {
      notify("There is nobody to send it to yet");
      return;
    }
    if (composer.overLimit) {
      notify("This message is over the 35 MB the provider takes. Take an attachment off it.");
      return;
    }
    cancelSave(at);
    const draft = composer.draft;
    const title = composer.title;
    const threadKey = draft.threadKey ?? null;

    if (at === "card") set({ card: null, parked: null, expanded: false });
    else set({ reply: null, replyKey: null, parkedReply: null, parkedReplyKey: null });

    // The row says it is going before the call comes back, because a mailbox that waits for a round
    // trip before it admits you pressed Send is a mailbox you press Send on twice.
    if (threadKey) useMail.getState().patch([threadKey], { sending: true, hasDraft: false });

    // Rust stamps the hold before it builds the message, and building reads the attachments off
    // disk, so the delay is counted from here rather than from when the answer comes back.
    const started = Date.now();
    let undo: Undo;
    try {
      undo = await send(draft);
      void get().loadDrafts();
    } catch (e) {
      if (threadKey) useMail.getState().patch([threadKey], { sending: false, hasDraft: true });
      // Back on screen, not parked: after "That did not go through" the message should be right
      // there, not behind `c`. A card opened in the meantime keeps its place.
      if (at === "card") set(get().card ? { parked: composer } : { card: composer, parked: null });
      else set({ reply: composer, replyKey: threadKey });
      notify(`That did not go through: ${e}`);
      return;
    }

    if (now) {
      // A reply has its thread's "Waiting to send" line for this. A card has nothing on screen
      // from here until the provider answers, and that can be twenty seconds.
      notify(`Sending to ${named(draft.to[0])}`);
      try {
        await sendNow(undo.token);
      } catch (e) {
        notify(`Could not skip the wait: ${e}`);
        void useMail.getState().load();
        return;
      }
      notify(undo.label);
      void useMail.getState().load();
      return;
    }

    set({
      holding: {
        token: undo.token,
        label: undo.label,
        endsAtMs: started + undo.undoMs,
        draft,
        at,
        title,
        threadKey,
      },
    });
    startBeat();
  },

  undoSend: async () => {
    const holding = get().holding;
    if (!holding) return;
    stopBeat();
    set({ holding: null });
    useToast.getState().dismiss();
    try {
      await undoToken(holding.token);
    } catch (e) {
      notify(`Could not take that back: ${e}`);
      return;
    }
    const composer = blank(holding.at, holding.at === "card" ? "new" : "reply", holding.title, holding.draft);
    if (holding.at === "card") set({ card: composer, parked: null });
    else set({ reply: composer, replyKey: holding.threadKey, parkedReply: null, parkedReplyKey: null });
    if (holding.threadKey) {
      useMail.getState().patch([holding.threadKey], { sending: false, hasDraft: true });
    }
    void useMail.getState().load();
  },

  flush: async () => {
    if (get().flushPhase === "sending") return;
    stopBeat();
    const holding = get().holding;
    set({ holding: null, flushPhase: "sending" });
    if (holding && useToast.getState().message?.startsWith(holding.label)) {
      useToast.getState().dismiss();
    }
    try {
      for (const item of await outboxList()) await sendNow(item.id);
      set({ flushPhase: "idle" });
      void useMail.getState().load();
    } catch (e) {
      set({ flushPhase: "error" });
      notify(`Could not send that now: ${e}`);
    }
  },

  settle: () => {
    const holding = get().holding;
    stopBeat();
    if (!holding) return;
    set({ holding: null });
    // Only this toast, and only while it is still the one on screen: something else may have said
    // something in the meantime and dismissing that would be taking away somebody else's notice.
    if (useToast.getState().message?.startsWith(holding.label)) useToast.getState().dismiss();
    void useMail.getState().load();
  },
}));

/** Whether an editor's HTML has anything in it, which `<p></p>` does not. */
export function hasText(html: string): boolean {
  return html.replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").trim().length > 0;
}
