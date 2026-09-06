import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Avatar, Banner, Button, EmptyState, icons } from "../ui";
import { registerCommands, runCommand } from "../keys/commands";
import { keyFor, keyLabel, type CommandId } from "../keys/bindings";
import { readLeftOff, writeLeftOff } from "../leftOff";
import type { ThreadSummary, ThreadView } from "../ipc";
import { useContacts } from "../store/useContacts";
import { useFeed } from "../store/useFeed";
import { useMail } from "../store/useMail";
import { displayName, isBrand, messageTime } from "./format";
import { BodyMissing, BodySkeleton, MessageBody } from "./MessageBody";
import * as triage from "./triage";
import "./feed.css";

/**
 * The Feed: the whole stage, one column of cards, each already open.
 *
 * No read state, no counts, no weight. Time is the only order, and the one thing carried from
 * the last visit is a hairline where you stopped.
 */

/**
 * The key a button prints. `cap` in format.ts prints an unmodified key as itself, which is right
 * for `v` and gives `Enter` where the button wants the arrow, so a named key takes its glyph.
 */
function keycapOf(command: CommandId): string | undefined {
  const combo = keyFor(command);
  if (!combo) return undefined;
  return combo.length === 1 ? combo : keyLabel(combo);
}

/** How much of a body shows before the fade, read from the token layer rather than known here. */
function clipHeight(): number {
  const held = getComputedStyle(document.documentElement).getPropertyValue("--feed-clip");
  return parseFloat(held) || 0;
}

const startOfToday = (): number => {
  const day = new Date();
  day.setHours(0, 0, 0, 0);
  return day.getTime();
};

export function Feed() {
  const threads = useMail((s) => s.threads);
  const phase = useMail((s) => s.phase);
  const loadMore = useMail((s) => s.loadMore);
  const views = useFeed((s) => s.views);
  const focused = useFeed((s) => s.focused);
  const marker = useFeed((s) => s.marker);
  const arrive = useFeed((s) => s.arrive);
  const step = useFeed((s) => s.step);
  const toggle = useFeed((s) => s.toggle);
  const focus = useFeed((s) => s.focus);

  const scroller = useRef<HTMLDivElement | null>(null);
  const cards = useRef(new Map<string, HTMLElement>());
  /** The newest card on screen, which is what the hairline will mark on the next visit. */
  const top = useRef<number | null>(null);

  const keys = useMemo(() => threads.map((t) => t.key), [threads]);

  // Where the last visit ended, read once. A marker that moved while you read would be a hairline
  // that walks down the page.
  useEffect(() => {
    arrive(readLeftOff("feed"));
  }, [arrive]);

  // The place is left when this unmounts, which is what going anywhere else does to it. It belongs
  // in the state database's `markers` table; see src/leftOff.ts.
  useEffect(() => {
    const save = () => {
      if (top.current !== null) writeLeftOff("feed", top.current);
    };
    window.addEventListener("pagehide", save);
    return () => {
      window.removeEventListener("pagehide", save);
      save();
    };
  }, []);

  useEffect(
    () =>
      registerCommands({
        "select-next": () => step(keys, 1),
        "select-prev": () => step(keys, -1),
        "open-selection": () => {
          const key = useFeed.getState().focused;
          if (key) toggle(key);
        },
        undo: () => void triage.undo(),
        // The sender of the card the keyboard is on. The button on each card goes to the same
        // action with its own sender, because a click lands before the card takes the focus.
        unsubscribe: () => {
          const key = useFeed.getState().focused;
          const thread = useMail.getState().threads.find((t) => t.key === key);
          if (thread) void useContacts.getState().unsubscribe(thread.accountId, thread.from.address);
        },
        // Move and Save clip have bindings and buttons and no handler here yet. Rust has both
        // commands; what is missing is the picker and the selection on this side. An unregistered
        // command does nothing at all, which is what docs/keyboard.md asks for and better than a
        // verb that half happens.
      }),
    [keys, step, toggle],
  );

  // A card's body is fetched as the column reaches it, so a Feed of two hundred newsletters is not
  // two hundred `thread_view` calls at once, each of which the mirror answers by fetching a body.
  useEffect(() => {
    const root = scroller.current;
    if (!root) return;
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          const key = (entry.target as HTMLElement).dataset.card;
          if (key) useFeed.getState().want(key);
        }
      },
      { root, rootMargin: "600px 0px" },
    );
    for (const el of cards.current.values()) observer.observe(el);
    return () => observer.disconnect();
  }, [keys]);

  // The focused card has to be on screen, or `j` walks the column from behind the fold.
  useEffect(() => {
    if (!focused) return;
    cards.current.get(focused)?.scrollIntoView({ block: "nearest" });
  }, [focused]);

  const onScroll = useCallback(() => {
    const root = scroller.current;
    if (!root) return;
    const edge = root.getBoundingClientRect().top;
    for (const key of keys) {
      const el = cards.current.get(key);
      if (el && el.getBoundingClientRect().bottom > edge + 1) {
        top.current = Number(el.dataset.at);
        break;
      }
    }
    if (root.scrollTop + root.clientHeight > root.scrollHeight - 600) void loadMore();
  }, [keys, loadMore]);

  // The topmost card before anything has scrolled, so leaving a Feed you did not scroll still
  // leaves a mark.
  useEffect(() => {
    if (top.current === null && threads.length > 0) top.current = threads[0].dateMs;
  }, [threads]);

  const strippedToday = useMemo(() => {
    const today = startOfToday();
    return Object.values(views).filter((view) =>
      view.messages.some((m) => m.trackers.length > 0 && m.dateMs >= today),
    ).length;
  }, [views]);

  // The hairline goes above the card that was newest on screen last time, and nowhere at all when
  // that is still the newest card there is: there is nothing above it to have arrived since.
  const markerAt = useMemo(() => {
    if (marker === null) return -1;
    const at = threads.findIndex((t) => t.dateMs <= marker);
    return at > 0 ? at : -1;
  }, [threads, marker]);

  return (
    <main className="stage">
      <div className="feed" ref={scroller} onScroll={onScroll}>
        <div className="feed-inner">
          <Banner icon={icons.SHIELD}>
            <>
              {"Images are loaded through Margin, never from the sender."}
              {strippedToday > 0 ? (
                <>
                  {" Trackers stripped from "}
                  <b>{`${strippedToday} item${strippedToday === 1 ? "" : "s"}`}</b>
                  {" today."}
                </>
              ) : null}
            </>
          </Banner>

          {threads.length === 0 && phase !== "loading" ? (
            <EmptyState>Nothing here</EmptyState>
          ) : null}

          {threads.map((thread, at) => (
            <Fragment key={thread.key}>
              {at === markerAt ? <div className="left-off">You left off here</div> : null}
              <Card
                thread={thread}
                view={views[thread.key]}
                focused={thread.key === focused}
                onFocus={() => focus(thread.key)}
                onToggle={() => toggle(thread.key)}
                hold={(el) => {
                  if (el) cards.current.set(thread.key, el);
                  else cards.current.delete(thread.key);
                }}
              />
            </Fragment>
          ))}
        </div>
      </div>
    </main>
  );
}

interface CardProps {
  thread: ThreadSummary;
  view: ThreadView | undefined;
  focused: boolean;
  onFocus: () => void;
  onToggle: () => void;
  hold: (el: HTMLElement | null) => void;
}

function Card({ thread, view, focused, onFocus, onToggle, hold }: CardProps) {
  const expanded = useFeed((s) => s.expanded.includes(thread.key));
  const phase = useFeed((s) => s.phase[thread.key]);
  const retry = useFeed((s) => s.retry);
  const body = useRef<HTMLDivElement | null>(null);
  const [clipped, setClipped] = useState(false);

  // How tall the body really is, and so whether there is more of it than the clip shows. Measured
  // inside the frame rather than off it, because the frame is at least its own default height
  // whatever the message in it comes to, and it arrives a beat after the card does.
  useEffect(() => {
    const el = body.current;
    const frame = el?.querySelector("iframe");
    if (!el || !frame) return;
    const measure = () => {
      const content = frame.contentDocument?.body?.scrollHeight ?? 0;
      if (content <= 0) return;
      el.style.setProperty("--feed-body-h", `${content}px`);
      // While it is open there is no clip to overflow, so the answer from when it was closed
      // stands: this is what keeps See less from turning back into Read more.
      if (!expanded) setClipped(content > clipHeight());
    };
    const observer = new ResizeObserver(measure);
    observer.observe(frame);
    // A body shorter than the frame's default never changes the frame's box, so the observer would
    // never fire for it: the load is the only moment those cards can be measured at.
    frame.addEventListener("load", measure);
    measure();
    return () => {
      observer.disconnect();
      frame.removeEventListener("load", measure);
    };
  }, [view, expanded]);

  const message = view?.messages.at(-1);

  return (
    <article
      className="feed-card"
      ref={hold}
      data-card={thread.key}
      data-at={thread.dateMs}
      data-selected={focused ? "" : undefined}
      onClick={onFocus}
    >
      <div className="feed-head">
        <Avatar
          name={displayName(thread.from)}
          address={thread.from.address}
          brand={isBrand(thread.from)}
        />
        <div className="msg-who">
          <div className="msg-name">
            {displayName(thread.from)}
            <span className="addr">{thread.from.address}</span>
          </div>
        </div>
        <div className="msg-time">{messageTime(thread.dateMs)}</div>
      </div>

      <h2 className="feed-title">{thread.subject}</h2>

      {message ? (
        <div
          className="feed-body"
          ref={body}
          data-open={expanded ? "" : undefined}
          data-paper={message.surface === "paper" ? "" : undefined}
        >
          <MessageBody html={message.html} surface={message.surface} />
          {clipped && !expanded ? <div className="feed-fade" /> : null}
        </div>
      ) : (
        <div className="feed-wait">
          {phase === "error" ? (
            <BodyMissing onRetry={() => retry(thread.key)} />
          ) : (
            <BodySkeleton />
          )}
        </div>
      )}

      <div className="feed-foot">
        <Button keycap={keycapOf("open-selection")} onClick={onToggle}>
          {expanded ? "See less" : "Read more"}
        </Button>
        <Button keycap={keycapOf("save-clip")} onClick={() => runCommand("save-clip")}>
          Save clip
        </Button>
        <span className="feed-gap" />
        <Button
          variant="ghost"
          keycap={keycapOf("unsubscribe")}
          onClick={() =>
            void useContacts.getState().unsubscribe(thread.accountId, thread.from.address)
          }
        >
          Unsubscribe
        </Button>
        <Button variant="ghost" keycap={keycapOf("move")} onClick={() => runCommand("move")}>
          Move
        </Button>
      </div>
    </article>
  );
}

export default Feed;
