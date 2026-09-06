import { useEffect, useMemo, useRef, useState } from "react";
import { Avatar, AvatarStack, Banner, Button, EmptyState, Field, icons, Pill, Sheet } from "../ui";
import { registerCommands, runCommand } from "../keys/commands";
import { attachmentOpen, messageShowImages } from "../api/messages";
import { noteAdd } from "../api/notes";
import { threadMerge, threadRename, threadUnmerge } from "../api/threads";
import { undoToken } from "../api/undo";
import type { CommandId } from "../keys/bindings";
import type { MessageView, Person, Surface, ThreadSummary, Undo } from "../ipc";
import { useAccounts } from "../store/useAccounts";
import { useCompose, type ComposerKind } from "../store/useCompose";
import { useMail } from "../store/useMail";
import { usePiles } from "../store/usePiles";
import { useSelection } from "../store/useSelection";
import { useSettings } from "../store/useSettings";
import { useSnooze } from "../store/useSnooze";
import { useTheme } from "../store/useTheme";
import { notify } from "../store/useToast";
import { LabelPicker, type PickerMode } from "./ActionBar";
import { ReplyBox, SendingLine } from "./Compose";
import { InviteCard } from "./InviteCard";
import { BodyMissing, BodySkeleton, MessageBody } from "./MessageBody";
import { MoreMenu } from "./MoreMenu";
import { SnoozePicker, snoozeAnchor } from "./SnoozePicker";
import * as triage from "./triage";
import {
  cap,
  displayName,
  fileKind,
  fileSize,
  isBrand,
  messageTime,
  participantLine,
  previewOf,
} from "./format";
import "./pane.css";

// The three decisions that are read here and taken from anywhere: a note, a rename and a merge.
//
// They live with the pane because the pane is where all three are read: the note is a block after
// its message, the rename is the line beside the subject, and the merge is the banner over the
// thread. The list imports the two panels below for the same reason it imports the label picker
// from the action bar, which is that a verb belongs with what it draws rather than with what
// happens to have pressed it.

/** The toast a hidden action leaves behind, carrying the token that reverses that one action. */
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

/**
 * The open thread, asked for again.
 *
 * A note, a rename and a merge all change what the pane is drawing, and the invalidation the
 * backend emits refreshes the list rather than the thread in front of you. Nothing happens when
 * the thread is not the one open, which is what makes `y` from the list safe.
 */
function reopen(key: string): void {
  if (useMail.getState().openKey === key) void useMail.getState().open(key);
}

/** `y`. A note is one thread's, it shows itself in the row and in the pane, and it says nothing. */
export async function saveNote(threadKey: string, body: string): Promise<void> {
  const text = body.trim();
  if (!text) return;
  try {
    await noteAdd(threadKey, text);
    reopen(threadKey);
  } catch (e) {
    notify(`That did not go through: ${e}`);
  }
}

/**
 * A name of your own for a thread. No toast: the new name is in the list and the pane the moment
 * it lands, and "renamed · was …" beside the subject is the receipt. `z` still takes it back.
 */
export async function renameThread(key: string, name: string | null): Promise<void> {
  try {
    await threadRename(key, name);
    reopen(key);
  } catch (e) {
    notify(`That did not go through: ${e}`);
  }
}

/** `g`. Two or more threads become one, named after the first, with the banner saying where from. */
export async function mergeThreads(keys: string[]): Promise<void> {
  if (keys.length < 2) return;
  const mail = useMail.getState();
  // The merged thread takes the first key, so the rest are the rows that disappear into it.
  const taken = mail.take(keys.slice(1));
  mail.patch([keys[0]], { merged: true });
  useSelection.getState().clear();
  try {
    const undo = await threadMerge(keys, null);
    // The count and the participants are the merge's answer rather than a guess this side could
    // have made, so the row is asked for again once the write has landed.
    void useMail.getState().load();
    reopen(keys[0]);
    acknowledge(undo);
  } catch (e) {
    useMail.getState().untake(taken);
    notify(`That did not go through: ${e}`);
  }
}

/**
 * The banner's verb. The re-read is awaited, unlike the merge's: the banner this was pressed on is
 * up until the view without `mergedFrom` lands and its button is saying it is working until then,
 * so settling any earlier would put "Unmerge" back on a strip that is about to go.
 */
export async function unmergeThread(key: string): Promise<void> {
  try {
    const undo = await threadUnmerge(key);
    void useMail.getState().load();
    acknowledge(undo);
    if (useMail.getState().openKey === key) await useMail.getState().open(key);
  } catch (e) {
    notify(`That did not go through: ${e}`);
  }
}

interface VerbSpec {
  command: CommandId;
  label: string;
  icon: string;
}

/**
 * The bar of verbs, in the order the hand reaches for them: the four that move a thread on the
 * left, and Archive on the right where it cannot be hit by accident.
 *
 * Every one of them goes through the command registry rather than calling anything directly, which
 * is what keeps the button and the key one code path. Reply all and Forward are not in the bar and
 * are not missing: `a` and `f` open the same box this button does with different recipients in it,
 * and three buttons for one verb is a bar you have to read.
 */
const LEAD: VerbSpec[] = [
  { command: "reply", label: "Reply", icon: icons.REPLY },
  { command: "reply-later", label: "Reply later", icon: icons.CLOCK },
  { command: "set-aside", label: "Set aside", icon: icons.SET_ASIDE },
  { command: "snooze", label: "Snooze", icon: icons.SNOOZE },
];

/** Paper Trail is not a place you answer from, and it has somewhere of its own to send a thread. */
const TRAIL_LEAD: VerbSpec[] = LEAD.filter((v) => v.command !== "reply-later");

function PaneBar({ notify }: { notify: boolean }) {
  const place = useMail((s) => s.place);
  const lead = place === "paper-trail" ? TRAIL_LEAD : LEAD;
  const [more, setMore] = useState(false);
  const moreButton = useRef<HTMLButtonElement | null>(null);

  // `.` and the button are one verb, so the key is registered by the bar that draws the button and
  // for exactly as long as there is a thread for the menu to be about.
  useEffect(() => registerCommands({ more: () => setMore((was) => !was) }), []);

  return (
    <div className="pane-bar">
      {lead.map((verb) => (
        <Verb key={verb.command} {...verb} />
      ))}
      <span className="pane-gap" />
      {place === "paper-trail" ? (
        <Verb command="move" label="Move to Inbox" icon={icons.INBOX} />
      ) : null}
      {notify ? <Verb command="notify" label="Notify" icon={icons.BELL} /> : null}
      <Verb command="archive" label="Archive" icon={icons.ARCHIVE} />
      <Button
        ref={moreButton}
        variant="ghost"
        iconOnly
        icon={icons.MORE}
        title={`More (${cap("more")})`}
        label="More"
        active={more}
        onClick={() => setMore((was) => !was)}
      />
      <MoreMenu open={more} anchor={moreButton.current} onClose={() => setMore(false)} />
    </div>
  );
}

/** Whether an address is one of this app's own, which is what "to you" and "and you" both mean. */
function useIsMe(): (person: Person) => boolean {
  const accounts = useAccounts((s) => s.accounts);
  const mine = useMemo(() => new Set(accounts.map((a) => a.email.toLowerCase())), [accounts]);
  return (person: Person) => mine.has(person.address.toLowerCase());
}

/**
 * The thread you clicked, on the frame you clicked it, before its view has arrived.
 *
 * Everything here comes off the row the list is already holding: the subject, the sender, the
 * people and the time. Only the bodies need a read, so only the bodies wait, and the alternative
 * this replaces was leaving the last thread you read on screen under the new row's selection.
 *
 * `data-opening` is the state, on the pane rather than in it, so a test can ask which of the two
 * heads it is looking at without either of them having to look different.
 */
function OpeningPane({ summary }: { summary: ThreadSummary }) {
  const isMe = useIsMe();
  const others = summary.participants.filter((p) => !isMe(p));
  return (
    <section className="pane" data-opening="">
      <PaneBar notify={summary.notify} />
      <div className="thread">
        <div className="thread-inner">
          <div className="thread-head">
            <h1 className="thread-subject" title={summary.subject}>
              {/* The same class as the rename control the loaded head puts here, because it is
                  the same slot: the clamp and the face belong to the subject, not to the button
                  that has not been drawn yet. */}
              <span className="thread-name">{summary.subject}</span>
            </h1>
            <div className="thread-meta">
              <AvatarStack
                people={others.map((p) => ({
                  name: displayName(p),
                  address: p.address,
                  brand: isBrand(p),
                }))}
              />
              <span className="thread-people">{participantLine(others.map(displayName), false)}</span>
              <span aria-hidden="true">·</span>
              <span>{`${summary.messageCount} message${summary.messageCount === 1 ? "" : "s"}`}</span>
            </div>
          </div>
          <article className="msg">
            <div className="msg-head">
              <Avatar
                name={displayName(summary.from)}
                address={summary.from.address}
                brand={isBrand(summary.from)}
              />
              <div className="msg-who">
                <div className="msg-name">{displayName(summary.from)}</div>
              </div>
              <div className="msg-time">{messageTime(summary.dateMs)}</div>
            </div>
            <div className="msg-body">
              <BodySkeleton />
            </div>
          </article>
        </div>
      </div>
    </section>
  );
}

export function ReadingPane() {
  const place = useMail((s) => s.place);
  const thread = useMail((s) => s.thread);
  const opening = useMail((s) => s.opening);
  const phase = useMail((s) => s.threadPhase);

  const [expanded, setExpanded] = useState<string[]>([]);
  const [quoted, setQuoted] = useState<string[]>([]);
  const [focus, setFocus] = useState<number | null>(null);
  /** Messages re-rendered with their images fetched, which replaces the one the thread came with. */
  const [shown, setShown] = useState<Record<string, MessageView>>({});
  /**
   * Where the request for the pictures is. Rust fetches them and that is a network round trip per
   * host, so the button has to say it is working: pressed and unchanged for three seconds is the
   * one thing this pane must never look like.
   */
  const [images, setImages] = useState<"idle" | "fetching">("idle");
  const imagesRequest = useRef(0);
  /** Same for the merge banner's verb, which is three round trips with nothing else on screen. */
  const [unmerging, setUnmerging] = useState<"idle" | "working">("idle");
  const [picker, setPicker] = useState<PickerMode | null>(null);
  const [noting, setNoting] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const scroller = useRef<HTMLDivElement | null>(null);
  const pane = useMail((s) => s.pane);
  const openKey = useMail((s) => s.openKey);
  const reply = useCompose((s) => s.reply);
  const replyKey = useCompose((s) => s.replyKey);

  const isMe = useIsMe();

  const messages = useMemo(
    () => (thread?.messages ?? []).map((m) => shown[m.id] ?? m),
    [thread, shown],
  );

  // A thread opens on its latest message with everything before it collapsed to a line, which is
  // the shape of every conversation you already know the beginning of.
  useEffect(() => {
    const last = thread?.messages.at(-1);
    setExpanded(last ? [last.id] : []);
    setQuoted([]);
    setFocus(null);
    setShown({});
    setImages("idle");
    imagesRequest.current += 1;
    setUnmerging("idle");
    // The failed slots were the last thread's, and nothing in `open` knows about them.
    useMail.getState().clearBodyPhase();
    scroller.current?.scrollTo({ top: 0 });
  }, [thread?.key]);

  const toggle = (id: string) =>
    setExpanded((was) => (was.includes(id) ? was.filter((x) => x !== id) : [...was, id]));

  // A reply belongs to the thread it answers, so leaving the thread parks it with the provider and
  // takes the box off the screen rather than carrying it to the next conversation.
  useEffect(() => {
    const compose = useCompose.getState();
    if (compose.replyKey && compose.replyKey !== thread?.key) compose.closeReply();
  }, [thread?.key]);

  // A box that opened below the fold is a box you have to go and find.
  useEffect(() => {
    if (!reply || replyKey !== thread?.key) return;
    scroller.current?.querySelector(".reply")?.scrollIntoView({ block: "nearest" });
  }, [reply !== null, replyKey, thread?.key]);

  // `r`, `a` and `f`. All three open the same box under the last message with different people in
  // it, and `a` on a box that is already open is the switch rather than a second box.
  useEffect(() => {
    if (!thread) return;
    const answer = (kind: ComposerKind, all: boolean) => {
      const view = useMail.getState().thread;
      const last = view?.messages.at(-1);
      if (!view || !last) return;
      useCompose.getState().answer(view, last, kind, all);
    };
    return registerCommands({
      reply: () => answer("reply", useSettings.getState().settings?.replyAllDefault ?? false),
      "reply-all": () => {
        const compose = useCompose.getState();
        const open = compose.replyKey === thread.key ? compose.reply : null;
        if (open && open.kind !== "forward") compose.setAll(!open.all);
        else answer("reply", true);
      },
      forward: () => answer("forward", false),
    });
  }, [thread?.key, thread]);

  useEffect(() => {
    if (!thread) return;
    const at = (delta: number) => {
      setFocus((was) => {
        const next = was === null ? messages.length - 1 : was + delta;
        return Math.min(Math.max(next, 0), messages.length - 1);
      });
    };
    return registerCommands({
      "message-next": () => at(1),
      "message-prev": () => at(-1),
      "message-toggle": () => {
        const id = messages[focus ?? messages.length - 1]?.id;
        if (id) toggle(id);
      },
      "message-expand-all": () => setExpanded(messages.map((m) => m.id)),
    });
  }, [thread, messages, focus]);

  // With the reading pane hidden the thread is the page and the list is not on screen, so the
  // triage verbs act on what is open here. While the list is up they are the list's, which is what
  // keeps `e` on the row the keyboard is on rather than on whatever was opened last.
  const alone = !pane && openKey !== null;
  useEffect(() => {
    if (!alone || !openKey) return;
    const keys = [openKey];
    return registerCommands({
      archive: () => triage.archive(keys),
      "toggle-seen": () => triage.toggleSeen(keys),
      "toggle-star": () => triage.toggleStar(keys),
      trash: () => triage.trash(keys),
      spam: () => triage.spam(keys),
      label: () => setPicker("apply"),
      undo: () => void triage.undo(),
      "reply-later": () => void usePiles.getState().toggle(keys, "reply-later"),
      "set-aside": () => void usePiles.getState().toggle(keys, "set-aside"),
      snooze: () => useSnooze.getState().show(keys, snoozeAnchor()),
      note: () => setNoting(keys[0]),
      // The subject in the pane is the control. This is how somebody who has not discovered that
      // finds it, which is what the palette is for.
      rename: () => setRenaming(keys[0]),
      // `g` is not here. A merge is two threads or more and there is one thread on this screen,
      // so the key would have nothing to act on.
    });
  }, [alone, openKey]);

  // The focused message has to be on screen, or `n` walks the thread from behind the fold.
  useEffect(() => {
    if (focus === null) return;
    const id = messages[focus]?.id;
    if (!id) return;
    const el = scroller.current?.querySelector(`[data-message="${CSS.escape(id)}"]`);
    el?.scrollIntoView({ block: "nearest" });
    // An invitation is answered with `y`, `m` and `n`, and those three are the card's only while
    // the card holds the focus. Walking onto the message that carries one is how a keyboard
    // reaches it; a card you had to click first would be a card the keyboard could not answer.
    el?.querySelector<HTMLElement>(".invite")?.focus();
  }, [focus, messages]);

  if (!thread) {
    // The row is enough to be the new thread already. Blank is only for a thread opened from
    // somewhere with no row behind it, which gets the skeleton while its view is read, and for
    // nothing being open at all.
    if (opening) return <OpeningPane summary={opening} />;
    return (
      <section className="pane">
        <div className="pane-blank">
          {phase === "loading" ? <BodySkeleton /> : <EmptyState>Nothing selected</EmptyState>}
        </div>
      </section>
    );
  }

  const trackers = messages.flatMap((m) => m.trackers);
  const blocked = messages.filter((m) => m.blockedImages > 0);
  // Asked for and still not there: a host that would not answer. The banner says so rather than
  // offering the same button again as if nothing had happened.
  const missing = blocked.reduce((n, m) => (m.imagesLoaded ? n + m.blockedImages : n), 0);
  const vendors = [...new Set(trackers.map((t) => t.vendor))];

  const showImages = async () => {
    if (images === "fetching") return;
    const mine = ++imagesRequest.current;
    setImages("fetching");
    const results = await Promise.allSettled(blocked.map((m) => messageShowImages(m.id)));
    const next: Record<string, MessageView> = {};
    let failure: unknown = null;
    results.forEach((result, index) => {
      if (result.status === "fulfilled") next[blocked[index].id] = result.value;
      else failure ??= result.reason;
    });
    setShown((was) => ({ ...was, ...next }));
    // Another thread has been opened since, and its banner is not this request's to settle.
    if (imagesRequest.current !== mine) return;
    setImages("idle");
    // A press that came to nothing has to say so, or the button is the one that looked stuck.
    if (failure !== null) notify(`Could not load the images: ${String(failure)}`);
  };

  const unmerge = async () => {
    if (unmerging === "working") return;
    setUnmerging("working");
    await unmergeThread(thread.key);
    // Landed, the banner has already gone with the re-read; refused, the verb comes back.
    setUnmerging("idle");
  };

  const wroteBack = messages.some((m) => m.sentByMe);
  const others = thread.participants.filter((p) => !isMe(p));
  const faces = wroteBack ? thread.participants : others;
  // The line names three of forty and counts the rest, so the forty are in the title: the answer to
  // "who else is on this" is one hover away rather than gone.
  const everyone = others.map(displayName).join(", ");

  return (
    <section className="pane">
      <PaneBar notify={thread.notify} />

      <div className="thread" ref={scroller}>
        <div className="thread-inner">
          <div className="thread-head">
            {/* The subject is the control that renames it. There is no verb for this in the
                keymap and no button beside it: the thing you want to change is the thing you
                press, which is how a file is renamed everywhere else. */}
            {/* Clamped to two lines with the whole of it in the title, because a subject is a
                sentence somebody typed and some of them carry the date, the room and your own
                address in them. Three lines at display size is a page you scroll past. */}
            <h1 className="thread-subject" title={thread.subject}>
              <button
                type="button"
                className="thread-name"
                title="Rename this thread"
                onClick={() => setRenaming(thread.key)}
              >
                {thread.subject}
              </button>
              {thread.originalSubject ? (
                <span className="renamed">{`renamed · was "${thread.originalSubject}"`}</span>
              ) : null}
            </h1>
            <div className="thread-meta">
              <AvatarStack
                people={faces.map((p) => ({
                  name: displayName(p),
                  address: p.address,
                  brand: isBrand(p),
                }))}
              />
              <span className="thread-people" title={everyone}>
                {participantLine(others.map(displayName), wroteBack)}
              </span>
              <span aria-hidden="true">·</span>
              <span>{`${messages.length} message${messages.length === 1 ? "" : "s"}`}</span>
            </div>
          </div>

          {/* Where something else put this thread, and the way back out. The banner is the mouse
              path to a rescue and the place the thirty day rule belongs: the list's foot says it
              once for the whole place, and this says it about the thread you are looking at. */}
          {thread.trashed || thread.spam ? (
            <div className="thread-banner">
              <Banner
                icon={thread.trashed ? icons.TRASH : icons.SPAM}
                action={{
                  label: thread.trashed ? "Put back" : "Not spam",
                  onClick: () =>
                    thread.trashed ? triage.trash([thread.key]) : triage.spam([thread.key]),
                }}
              >
                {thread.trashed
                  ? "In the trash. Gmail empties it after 30 days, and putting it back lands it where it was."
                  : "Gmail marked this as spam. It empties spam after 30 days, and taking the mark off routes it like any other mail."}
              </Banner>
            </div>
          ) : null}

          {thread.mergedFrom.length > 0 ? (
            <div className="thread-banner">
              <Banner
                tone="muted"
                icon={icons.MERGE}
                action={{
                  label: "Unmerge",
                  busy: unmerging === "working",
                  busyLabel: "Unmerging…",
                  onClick: () => void unmerge(),
                }}
              >
                <>
                  {"Merged from "}
                  <b>{`${thread.mergedFrom.length} threads`}</b>
                  {` · ${thread.mergedFrom.map((m) => m.subject).join(", ")}`}
                </>
              </Banner>
            </div>
          ) : null}

          {trackers.length > 0 || blocked.length > 0 ? (
            <div className="thread-banner">
              <Banner
                icon={icons.SHIELD}
                action={
                  blocked.length > 0
                    ? {
                        label: missing > 0 ? "Try again" : "Show images",
                        busy: images === "fetching",
                        busyLabel: "Loading images…",
                        onClick: () => void showImages(),
                      }
                    : undefined
                }
              >
                {/* The count is only spoken when there is one. A message with no trackers and a
                    remote image says the thing that is true about it, because a banner reading
                    "Blocked 0 trackers" is the app taking credit for doing nothing. */}
                <>
                  {trackers.length > 0 ? (
                    <>
                      {"Blocked "}
                      <b>{`${trackers.length} tracker${trackers.length === 1 ? "" : "s"}`}</b>
                      {vendors.length > 0 ? ` from ${vendors.join(", ")}. ` : ". "}
                    </>
                  ) : null}
                  {blocked.length > 0
                    ? missing > 0
                      ? `${missing} image${missing === 1 ? "" : "s"} did not load.`
                      : "Remote images are off for this sender."
                    : null}
                </>
              </Banner>
            </div>
          ) : null}

          {messages.map((message, index) => (
            <Message
              key={message.id}
              message={message}
              accountId={thread.accountId}
              plain={place === "paper-trail"}
              open={expanded.includes(message.id)}
              focused={focus === index}
              quoted={quoted.includes(message.id)}
              me={isMe(message.from)}
              toYou={message.to.some(isMe)}
              onToggle={() => {
                setFocus(index);
                toggle(message.id);
              }}
              onQuoted={() =>
                setQuoted((was) =>
                  was.includes(message.id)
                    ? was.filter((x) => x !== message.id)
                    : [...was, message.id],
                )
              }
              notes={thread.notes.filter((n) => n.afterMessageId === message.id)}
            />
          ))}

          {thread.notes
            .filter((note) => note.afterMessageId === null)
            .map((note) => (
              <Note key={note.id} body={note.body} />
            ))}

          <SendingLine threadKey={thread.key} />

          {/* The box a reply is written in, under what it answers. `r`, `a` and `f` open it and
              nothing else does: there is no permanent box at the foot of every thread, because
              most threads are read and not answered. */}
          {reply && replyKey === thread.key ? (
            <ReplyBox to={reply.sender[0] ?? thread.participants[0] ?? messages[0].from} composer={reply} />
          ) : null}
        </div>
      </div>

      <LabelPicker mode={picker} keys={openKey ? [openKey] : []} onClose={() => setPicker(null)} />
      <NoteSheet threadKey={noting} onClose={() => setNoting(null)} />
      <RenameSheet
        threadKey={renaming}
        subject={thread.subject}
        original={thread.originalSubject}
        onClose={() => setRenaming(null)}
      />
      {/* With the list on screen the picker is the list's, because `b` is the list's. Here it is
          mounted only when the thread is the page and there is no list to own it. */}
      {alone ? <SnoozePicker /> : null}
    </section>
  );
}

interface NoteSheetProps {
  /** The thread the note is about, or null when nothing is being written. */
  threadKey: string | null;
  onClose: () => void;
}

/**
 * `y`. A private note about a thread, text only.
 *
 * A panel rather than a box in the pane, because `y` is pressed on a row in the list as often as
 * on an open thread, and a note you can only write with the thread in front of you is a note you
 * write after reading rather than while deciding.
 */
export function NoteSheet({ threadKey, onClose }: NoteSheetProps) {
  const [body, setBody] = useState("");

  useEffect(() => {
    if (threadKey) setBody("");
  }, [threadKey]);

  if (!threadKey) return null;

  const save = () => {
    onClose();
    void saveNote(threadKey, body);
  };

  return (
    <Sheet
      open
      size="mini"
      title="Note to self"
      onClose={onClose}
      foot={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={body.trim().length === 0} onClick={save}>
            Save
          </Button>
        </>
      }
    >
      <Field
        label="Only you see this"
        value={body}
        onChange={setBody}
        placeholder="Ask about the oak finish before confirming"
        multiline
        rows={4}
        autoFocus
      />
    </Sheet>
  );
}

interface RenameSheetProps {
  threadKey: string | null;
  subject: string;
  /** The real subject, when this thread already carries a name of its own. */
  original: string | null;
  onClose: () => void;
}

/** A name of your own for a thread. Replies still carry the real subject, so threading holds. */
export function RenameSheet({ threadKey, subject, original, onClose }: RenameSheetProps) {
  const [name, setName] = useState(subject);

  useEffect(() => {
    if (threadKey) setName(subject);
  }, [threadKey, subject]);

  if (!threadKey) return null;

  const rename = (to: string | null) => {
    onClose();
    void renameThread(threadKey, to);
  };

  return (
    <Sheet
      open
      size="mini"
      title="Rename this thread"
      onClose={onClose}
      foot={
        <>
          {original ? <Button onClick={() => rename(null)}>Use the real subject</Button> : null}
          <Button
            variant="primary"
            disabled={name.trim().length === 0 || name === subject}
            onClick={() => rename(name.trim())}
          >
            Rename
          </Button>
        </>
      }
    >
      <Field
        label="What you want to call it"
        value={name}
        onChange={setName}
        hint={original ? `The real subject is "${original}", and replies still carry it.` : undefined}
        autoFocus
      />
    </Sheet>
  );
}

function Verb({ command, label, icon }: VerbSpec) {
  return (
    <Button variant="ghost" icon={icon} keycap={cap(command)} onClick={() => runCommand(command)}>
      {label}
    </Button>
  );
}

/**
 * Messages whose surface the reader has overruled, for as long as the app is open.
 *
 * Every heuristic misses, and the one behind `message.surface` misses in two directions: a page
 * painted only in a `<style>` rule reads as no page at all, and a signature block with a wash
 * behind it can read as one. So there is a way out, and it is one press.
 *
 * Deliberately not stored. A remembered override is a good idea and it is a different piece of
 * work: it is a decision about a sender rather than about a message, it belongs beside the other
 * per-sender decisions in the state database rather than in the mirror, and it has to roam through
 * the backup store with them. A `Map` for the session is the whole of what was asked for here.
 */
const overruled = new Map<string, Surface>();

/**
 * The one control that flips it.
 *
 * Only in dark, and only on a message that arrived as HTML. In the light palette the two surfaces
 * are the same warm white to within a shade nobody can name, so the button would be a control that
 * visibly does nothing; and a body this app set itself out of plain text was never on a sender's
 * page to be taken off one.
 */
function SurfaceToggle({ surface, onFlip }: { surface: Surface; onFlip: () => void }) {
  const paper = surface === "paper";
  return (
    <Button
      variant="ghost"
      size="sm"
      iconOnly
      icon={icons.MOON}
      active={!paper}
      onClick={onFlip}
      title={paper ? "Read this message in dark" : "Read this message on a light page"}
    />
  );
}

function Note({ body }: { body: string }) {
  return (
    <div className="note">
      <span className="note-label">Note to self</span>
      {body}
    </div>
  );
}

interface MessageProps {
  message: MessageView;
  /** The mailbox this thread is in, which is the account whose scopes an invite is answered with. */
  accountId: string;
  plain: boolean;
  open: boolean;
  focused: boolean;
  quoted: boolean;
  me: boolean;
  toYou: boolean;
  onToggle: () => void;
  onQuoted: () => void;
  notes: { id: string; body: string }[];
}

function Message({
  message,
  accountId,
  plain,
  open,
  focused,
  quoted,
  me,
  toYou,
  onToggle,
  onQuoted,
  notes,
}: MessageProps) {
  const who = me ? "You" : displayName(message.from);
  const theme = useTheme((s) => s.theme);
  const pending = useMail((s) => s.bodyPhase[message.id]);
  /**
   * Where each chip's request is, by attachment id. Rust hands the file to the OS once it has the
   * bytes, and the bytes are a network round trip when nobody has opened this file before, so the
   * chip has to say it is working the way the images button does.
   */
  const [files, setFiles] = useState<Record<string, "idle" | "opening" | "error">>({});
  const openFile = async (id: string) => {
    if (files[id] === "opening") return;
    setFiles((was) => ({ ...was, [id]: "opening" }));
    try {
      await attachmentOpen(id);
      setFiles((was) => ({ ...was, [id]: "idle" }));
    } catch (e) {
      setFiles((was) => ({ ...was, [id]: "error" }));
      // As it is: the refusals are sentences written for the person holding the laptop.
      notify(String(e));
    }
  };
  const [override, setOverride] = useState<Surface | null>(() => overruled.get(message.id) ?? null);
  const surface = override ?? message.surface;
  const flip = () => {
    const next: Surface = surface === "paper" ? "theme" : "paper";
    overruled.set(message.id, next);
    setOverride(next);
  };
  return (
    <article
      className="msg"
      data-message={message.id}
      data-collapsed={open ? undefined : ""}
      data-focus={focused ? "" : undefined}
    >
      <div className="msg-head" onClick={open ? undefined : onToggle}>
        {/* The face is the sender's, whatever the line beside it calls them: a thread of your own
            replies is a column of your initials, not a column of the word You. */}
        <Avatar
          name={displayName(message.from)}
          address={message.from.address}
          brand={isBrand(message.from)}
        />
        <div className="msg-who">
          <div className="msg-name">
            {who}
            {open && !me ? <span className="addr">{message.from.address}</span> : null}
          </div>
          {open ? (
            <div className="msg-to">{toYou ? "to you" : `to ${message.to.map(displayName).join(", ")}`}</div>
          ) : (
            <div className="msg-preview">{previewOf(message.html)}</div>
          )}
        </div>
        <div className="msg-aside">
          <span className="msg-time">{messageTime(message.dateMs)}</span>
          {open && theme === "dark" && message.isHtml ? (
            <SurfaceToggle surface={surface} onFlip={flip} />
          ) : null}
        </div>
      </div>

      {open ? (
        <div className="msg-body">
          {message.bodyPending ? (
            pending === "error" ? (
              <BodyMissing onRetry={() => void useMail.getState().hydrateThread()} />
            ) : (
              <BodySkeleton />
            )
          ) : (
            <MessageBody html={message.html} plain={plain} surface={surface} />
          )}

          {message.quotedHtml ? (
            <div className="msg-quoted">
              <Pill tone="quiet" onClick={onQuoted}>
                {quoted ? "Hide quoted text" : "··· Show quoted text"}
              </Pill>
              {quoted ? <MessageBody html={message.quotedHtml} plain={plain} surface={surface} /> : null}
            </div>
          ) : null}

          {message.invite ? (
            <InviteCard invite={message.invite} messageId={message.id} accountId={accountId} />
          ) : null}

          {message.attachments.length > 0 ? (
            <div className="attachments">
              {message.attachments.map((file) => (
                <button
                  type="button"
                  className="attachment"
                  key={file.id}
                  data-phase={files[file.id] ?? "idle"}
                  disabled={files[file.id] === "opening"}
                  onClick={() => void openFile(file.id)}
                >
                  <span className="ext">{fileKind(file.filename, file.mimeType)}</span>
                  {file.filename}
                  <span className="size">{fileSize(file.size)}</span>
                </button>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}

      {notes.map((note) => (
        <Note key={note.id} body={note.body} />
      ))}
    </article>
  );
}

export default ReadingPane;
