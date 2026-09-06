import { useEffect, useMemo, useRef, useState } from "react";
import { Avatar, Button, EmptyState, Icon, icons, Key } from "../ui";
import { registerCommands } from "../keys/commands";
import { useKeyContext } from "../keys/keymap";
import { threadOpened, threadView } from "../api/threads";
import type { ThreadSummary, ThreadView } from "../ipc";
import { useCompose } from "../store/useCompose";
import { useMail } from "../store/useMail";
import { usePiles } from "../store/usePiles";
import { useSettings } from "../store/useSettings";
import { useStage } from "../store/useStage";
import { cap, displayName, isBrand, messageTime } from "./format";
import { MessageBody } from "./MessageBody";
import "./focus.css";

/**
 * Focus & Reply: the whole stage, one item per thread on the Reply later pile.
 *
 * The page is the pile read one at a time. Each item is the latest message on the left and a reply
 * box on the right, in one bordered card, and the item the keyboard is on takes a ring. `Tab` moves
 * on and the thread stays where it was, which is the point: skipping is the ordinary outcome and
 * costs nothing.
 *
 * Send goes down the same pipeline the compose card and the thread's reply box use: the draft is
 * handed to `useCompose`, queued, and held for the undo delay while the toast counts it down. The
 * item collapses to one line the moment it is queued, so the page shortens as it is worked through,
 * and taking the send back brings the item and what was typed in it straight back.
 *
 * The box here is a textarea rather than the TipTap editor the other two composers use, which is
 * deliberate: this page is a pile answered one line at a time, and a formatting toolbar's worth of
 * document model is not what "Wednesday at four suits us" needs.
 */
export function FocusReply() {
  const threads = usePiles((s) => s.threads);
  const load = usePiles((s) => s.load);
  const accountId = useMail((s) => s.accountId);
  const close = useStage((s) => s.close);

  const items = threads["reply-later"];
  const [at, setAt] = useState(0);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  /** The threads that have been answered on this visit, and who they went to. */
  const [sent, setSent] = useState<Record<string, string>>({});
  const [views, setViews] = useState<Record<string, ThreadView>>({});
  const boxes = useRef(new Map<string, HTMLTextAreaElement>());
  /** Which box the caret is in, which is what decides whose reply `Cmd+Enter` sends. */
  const [typing, setTyping] = useState<string | null>(null);
  const replyKey = useCompose((s) => s.replyKey);
  const holding = useCompose((s) => s.holding);

  useEffect(() => {
    void load();
  }, [accountId, load]);

  // The pile is a handful of threads, so the messages are asked for at once rather than as the
  // page reaches them: this is a page you work down, not a feed you scroll.
  useEffect(() => {
    for (const thread of items) {
      if (views[thread.key]) continue;
      void threadView(thread.key)
        .then((view) => setViews((was) => (was[view.key] ? was : { ...was, [view.key]: view })))
        .catch(() => {
          // A thread whose body will not come back is an item with its subject and its reply box,
          // which is still the thing you came here to do.
        });
    }
  }, [items, views]);

  // `Tab` is this page's and nothing else's while it is up, and the box the caret is in takes the
  // editor's frame on top of it so that `Cmd+Enter` resolves to Send at all. `Tab` still works
  // through the box's own handler, which is what the dispatcher standing out of a text field's way
  // leaves room for.
  useKeyContext("focus");
  useKeyContext("editor", typing !== null);

  const left = useMemo(() => items.filter((t) => !sent[t.key]).length, [items, sent]);

  // Every item's latest message is on the page, but the one with the ring and the caret is the one
  // being read, so it is the one marked seen, the way the pane marks what it shows. Keyed on the
  // thread rather than on the list, because the pile is asked for again on every invalidation and
  // the same item is not opened twice by that.
  const activeKey = items[at]?.key;
  useEffect(() => {
    if (!activeKey) return;
    void threadOpened(activeKey).catch(() => {
      // Nothing on this page shows read state, so there is nothing to put back and nothing to say.
    });
  }, [activeKey]);

  const step = (delta: number) =>
    setAt((was) => Math.min(Math.max(was + delta, 0), Math.max(items.length - 1, 0)));

  /** One item's reply, down the pipeline the other two composers use. */
  const post = (thread: ThreadSummary, now: boolean) => {
    const view = views[thread.key];
    const last = view?.messages.at(-1);
    const body = (drafts[thread.key] ?? "").trim();
    if (!view || !last || !body) return;

    const compose = useCompose.getState();
    compose.answer(view, last, "reply", useSettings.getState().settings?.replyAllDefault ?? false);
    compose.edit("reply", {
      bodyHtml: body
        .split(/\n{2,}/)
        .map((para) => `<p>${para.replace(/\n/g, "<br>")}</p>`)
        .join(""),
    });
    const to = compose.reply?.draft.to[0];
    void compose.post("reply", now);
    setSent((was) => ({ ...was, [thread.key]: to ? displayName(to) : displayName(thread.from) }));
    step(1);
  };

  const latest = useRef(post);
  latest.current = post;

  useEffect(
    () =>
      registerCommands({
        "focus-next": () => step(1),
        "focus-prev": () => step(-1),
      }),
    [items.length],
  );

  useEffect(() => {
    if (typing === null) return;
    const thread = items.find((t) => t.key === typing);
    if (!thread) return;
    return registerCommands({
      send: () => latest.current(thread, false),
      "send-now": () => latest.current(thread, true),
    });
  }, [typing, items]);

  // `z` inside the delay means the send, here as well as in the pane. This page is a stage and the
  // reading pane is not mounted behind it, so the compose card's copy of this registration is not
  // on screen to be reached.
  useEffect(() => {
    if (!holding) return;
    return registerCommands({ undo: () => void useCompose.getState().undoSend() });
  }, [holding]);

  // A send that was taken back puts its item straight back, because `undoSend` reopens the draft on
  // the thread it came from and that is the only signal this page needs.
  useEffect(() => {
    if (!replyKey || !sent[replyKey]) return;
    setSent((was) => {
      const next = { ...was };
      delete next[replyKey];
      return next;
    });
  }, [replyKey, sent]);

  // The box the keyboard is on takes the caret, so the page is typed into rather than clicked
  // into. Tab out of a box is handled by the box itself: the dispatcher never takes a key from a
  // text field, and this screen's field is the one that wants to give this one up.
  useEffect(() => {
    const key = items[at]?.key;
    if (key) boxes.current.get(key)?.focus();
  }, [at, items]);

  if (items.length === 0) {
    return (
      <main className="stage focus">
        <div className="focus-inner">
          <Head left={0} />
          <div className="focus-blank">
            <EmptyState>Nothing in Reply later</EmptyState>
          </div>
        </div>
      </main>
    );
  }

  return (
    <main className="stage focus">
      <div className="focus-inner">
        <Head left={left} />

        {items.map((thread, index) =>
          sent[thread.key] ? (
            <div className="focus-sent" key={thread.key}>
              <Icon d={icons.CHECK} size={14} />
              <span>
                {"Sent to "}
                <b>{sent[thread.key]}</b>
                {` · ${thread.subject}`}
              </span>
            </div>
          ) : (
            <Item
              key={thread.key}
              thread={thread}
              view={views[thread.key]}
              active={index === at}
              draft={drafts[thread.key] ?? ""}
              onDraft={(body) => setDrafts((was) => ({ ...was, [thread.key]: body }))}
              onFocus={() => setAt(index)}
              onTyping={(on) => setTyping(on ? thread.key : null)}
              onSend={() => post(thread, false)}
              onSkip={() => step(1)}
              onStep={step}
              register={(el) => {
                if (el) boxes.current.set(thread.key, el);
                else boxes.current.delete(thread.key);
              }}
            />
          ),
        )}

        <div className="focus-foot">
          <Button variant="ghost" onClick={close} keycap="⎋">
            Back to Reply later
          </Button>
        </div>
      </div>
    </main>
  );
}

/** The title, the sentence, and the three keys this page answers to, right aligned under them. */
function Head({ left }: { left: number }) {
  return (
    <div className="focus-head">
      <h1 className="focus-title">Focus &amp; Reply</h1>
      <p className="focus-lede">
        Every thread in Reply later, each with its own reply box. Send, or move on to the next.
      </p>
      <p className="focus-hint">
        <span>{`${left} left`}</span>
        <span aria-hidden="true">·</span>
        <Key size="sm">{cap("focus-next") ?? "Tab"}</Key>
        <span>next</span>
        <span aria-hidden="true">·</span>
        <Key size="sm">{cap("send") ?? "⌘↩"}</Key>
        <span>send</span>
        <span aria-hidden="true">·</span>
        <Key size="sm">⎋</Key>
        <span>back</span>
      </p>
    </div>
  );
}

interface ItemProps {
  thread: ThreadSummary;
  view: ThreadView | undefined;
  active: boolean;
  draft: string;
  onDraft: (body: string) => void;
  onFocus: () => void;
  /** Whether the caret is in this box, which is what decides whose reply `Cmd+Enter` sends. */
  onTyping: (on: boolean) => void;
  onSend: () => void;
  onSkip: () => void;
  onStep: (delta: number) => void;
  register: (el: HTMLTextAreaElement | null) => void;
}

function Item({
  thread,
  view,
  active,
  draft,
  onDraft,
  onFocus,
  onTyping,
  onSend,
  onSkip,
  onStep,
  register,
}: ItemProps) {
  const message = view?.messages.at(-1);
  const from = message?.from ?? thread.from;
  const name = displayName(from);

  return (
    <article className="focus-item" data-active={active ? "" : undefined} onClick={onFocus}>
      <div className="focus-message">
        <h2 className="focus-subject">{thread.subject}</h2>
        <div className="focus-from">
          <Avatar name={name} address={from.address} brand={isBrand(from)} />
          <div className="focus-who">
            <div className="focus-name">
              {name}
              <span className="addr">{from.address}</span>
            </div>
            <div className="focus-to">to you</div>
          </div>
          <div className="focus-time">{messageTime(message?.dateMs ?? thread.dateMs)}</div>
        </div>
        <div className="focus-body">
          {message ? (
            <MessageBody html={message.html} surface={message.surface} />
          ) : (
            <p className="focus-snippet">{thread.snippet}</p>
          )}
        </div>
      </div>

      <div className="focus-reply">
        <div className="focus-reply-head">
          {"Reply to "}
          <b>{name}</b>
        </div>
        <textarea
          className="focus-box"
          ref={register}
          value={draft}
          placeholder="Write a reply"
          aria-label={`Reply to ${name}`}
          onChange={(e) => onDraft(e.target.value)}
          onFocus={() => {
            onFocus();
            onTyping(true);
          }}
          onBlur={() => onTyping(false)}
          onKeyDown={(e) => {
            // The keymap never takes a key from a text field, and this field is the one that wants
            // to hand this one over: Tab is documented as the next item and it means that here.
            if (e.key !== "Tab") return;
            e.preventDefault();
            onStep(e.shiftKey ? -1 : 1);
          }}
        />
        <div className="focus-reply-foot">
          <Button
            variant="primary"
            keycap={cap("send")}
            disabled={!view || draft.trim().length === 0}
            onClick={onSend}
          >
            Send
          </Button>
          <Button variant="ghost" keycap={cap("focus-next")} onClick={onSkip}>
            Skip
          </Button>
        </div>
      </div>
    </article>
  );
}

export default FocusReply;
