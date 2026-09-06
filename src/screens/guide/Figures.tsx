import type { ReactElement } from "react";
import { Icon, icons, Key } from "../../ui";
import { ACCOUNT_KEYS, keyLabel, type CommandId } from "../../keys/bindings";
import { cap } from "../format";
import type { FigureId } from "./types";
import "./figures.css";

/**
 * The drawn figures, one per idea the guide has to explain.
 *
 * A picture is the app photographed and a figure is an idea drawn, which is why these are built
 * from the same tokens and the same primitives as the app rather than exported from a drawing
 * program: a diagram of the three boxes that stopped looking like the three boxes would be worse
 * than no diagram, and one that lives in this repository moves when they do.
 *
 * Nothing here is decoration. Every figure is making one sentence, and the sentence is under it.
 */

/**
 * A key as the app prints it, and the same key again as bare text.
 *
 * `<Key>` is not drawn on a phone, and a figure whose meaning rides on its caps would say nothing
 * there, so the letter follows it in plain text and the stylesheet shows whichever of the two the
 * window is for.
 */
function KeyText({ children, size = "sm" }: { children: string; size?: "sm" | "md" }) {
  return (
    <span className="fig-cap">
      <Key size={size}>{children}</Key>
      <span className="fig-cap-flat">{children}</span>
    </span>
  );
}

/** The key a verb answers to today, from the binding table, never a letter typed into a figure. */
function Cap({ of, size }: { of: CommandId; size?: "sm" | "md" }) {
  const key = cap(of);
  return key ? <KeyText size={size}>{key}</KeyText> : null;
}

/** One thread as it reads down a list: the avatar, then the run of words. */
function ThreadRow({ state }: { state?: "selected" | "gone" }) {
  return (
    <span className="fig-row" data-state={state}>
      <span className="fig-avatar" />
      <span className="fig-bar" data-w="name" />
      <span className="fig-bar" data-w="line" />
    </span>
  );
}

/** A stack of cards at the foot of the list, with the top card's thread showing. */
function Pile({ label }: { label: string }) {
  return (
    <span className="fig-pile">
      <span className="fig-pile-edge" />
      <span className="fig-pile-card">
        <span className="fig-mark">{label}</span>
        <span className="fig-bar" data-w="subject" />
      </span>
    </span>
  );
}

/** Where a message can end up, and the one place among the four that is not a box. */
function Routing() {
  return (
    <>
      <span className="fig-chip">Mail from a sender you have no rule for</span>
      <span className="fig-drop" />
      <span className="fig-gate">
        Screener
        <Cap of="place-screener" />
      </span>
      <span className="fig-legs">
        <span className="fig-leg" />
        <span className="fig-leg" />
        <span className="fig-leg" />
        <span className="fig-leg" />
      </span>
      <span className="fig-ends">
        <span className="fig-end">
          Inbox
          <Cap of="place-inbox" />
        </span>
        <span className="fig-end">
          Feed
          <Cap of="place-feed" />
        </span>
        <span className="fig-end">
          Paper Trail
          <Cap of="place-paper-trail" />
        </span>
        <span className="fig-end" data-out="">
          Screened out
        </span>
      </span>
    </>
  );
}

const BOXES: readonly { command: CommandId; name: string; holds: string }[] = [
  { command: "place-inbox", name: "Inbox", holds: "People, and every reply to a thread you are in" },
  { command: "place-feed", name: "Feed", holds: "What you subscribed to, drawn open" },
  {
    command: "place-paper-trail",
    name: "Paper Trail",
    holds: "Receipts, confirmations, what a machine sent",
  },
];

function Boxes() {
  return (
    <>
      {BOXES.map((box) => (
        <span className="fig-box" key={box.name}>
          <span className="fig-box-name">
            {box.name}
            <Cap of={box.command} />
          </span>
          <span className="fig-box-holds">{box.holds}</span>
        </span>
      ))}
    </>
  );
}

/** The window, at the size a diagram of it is read at rather than the size it is. */
function Window() {
  return (
    <span className="fig-window">
      <span className="fig-head">
        <span className="fig-mark">Header</span>
        <span className="fig-seg">
          <span className="fig-seg-on">Inbox</span>
          <span>Feed</span>
          <span>Paper Trail</span>
        </span>
      </span>
      <span className="fig-stage">
        <span className="fig-list">
          <span className="fig-mark">List</span>
          <ThreadRow state="selected" />
          <ThreadRow />
          <span className="fig-piles">
            <Pile label="Reply later" />
            <Pile label="Set aside" />
          </span>
        </span>
        <span className="fig-pane">
          <span className="fig-mark">Reading pane</span>
          <span className="fig-bar" data-w="subject" data-strong="" />
          <span className="fig-bar" data-w="wide" />
          <span className="fig-bar" data-w="wide" />
          <span className="fig-bar" data-w="short" />
        </span>
      </span>
    </span>
  );
}

/**
 * Eight verbs as eight single caps, which is the argument: what a figure of the keyboard has to
 * show is that there is never a second key to hold down with the first.
 */
const VERBS: readonly { command: CommandId; label: string }[] = [
  { command: "archive", label: "Archive" },
  { command: "reply-later", label: "Reply later" },
  { command: "set-aside", label: "Set aside" },
  { command: "snooze", label: "Snooze" },
  { command: "toggle-seen", label: "Seen" },
  { command: "note", label: "Note" },
  { command: "trash", label: "Trash" },
  { command: "spam", label: "Spam" },
];

function Keyboard() {
  return (
    <>
      {VERBS.map((verb) => (
        <span className="fig-verb" key={verb.command}>
          <Cap of={verb.command} size="md" />
          <span className="fig-verb-name">{verb.label}</span>
        </span>
      ))}
    </>
  );
}

/** One card with its parts named, because every part of it is a thing somebody asks about. */
function ScreenerCard() {
  return (
    <>
      <span className="fig-mark">Who wrote</span>
      <span className="fig-card-who">
        <span className="fig-avatar" data-lg="" />
        <span className="fig-lines">
          <span className="fig-bar" data-w="name" />
          <span className="fig-bar" data-w="subject" />
        </span>
      </span>

      <span className="fig-mark">What they sent</span>
      <span className="fig-lines">
        <span className="fig-bar" data-w="wide" />
        <span className="fig-bar" data-w="short" />
      </span>

      <span className="fig-mark">Why, and where</span>
      <span className="fig-lines">
        <span className="fig-tag">Written by a person · suggested Inbox</span>
      </span>

      <span className="fig-mark">Your three answers</span>
      <span className="fig-answers">
        <span className="fig-chip">
          Yes
          <Cap of="screen-yes" />
        </span>
        <span className="fig-chip">
          Elsewhere
          <Cap of="screen-elsewhere" />
        </span>
        <span className="fig-chip">
          No
          <Cap of="screen-no" />
        </span>
      </span>
    </>
  );
}

/** One collapsed message: everything it has to say fits on the line it is given. */
function Collapsed() {
  return (
    <span className="fig-msg">
      <span className="fig-avatar" />
      <span className="fig-bar" data-w="name" />
      <span className="fig-bar" data-w="line" />
      <span className="fig-bar" data-w="time" />
    </span>
  );
}

function Thread() {
  return (
    <span className="fig-thread">
      <span className="fig-bar" data-w="subject" data-strong="" />
      <Collapsed />
      <Collapsed />
      <span className="fig-msg" data-open="">
        <span className="fig-msg-head">
          <span className="fig-avatar" />
          <span className="fig-bar" data-w="name" />
          <span className="fig-bar" data-w="time" />
        </span>
        <span className="fig-bar" data-w="wide" />
        <span className="fig-bar" data-w="wide" />
        <span className="fig-bar" data-w="short" />
      </span>
    </span>
  );
}

function Trackers() {
  return (
    <span className="fig-message">
      <span className="fig-banner">
        Blocked 3 trackers. Remote images are off.
        <span className="fig-banner-action">Show images</span>
      </span>
      <span className="fig-bar" data-w="wide" />
      <span className="fig-blocked" />
      <span className="fig-bar" data-w="wide" />
      <span className="fig-bar" data-w="short" />
    </span>
  );
}

/** Out of the list and into a pile, and back by the key that put it there. */
function Piles() {
  return (
    <>
      <span className="fig-list">
        <ThreadRow />
        <ThreadRow state="gone" />
        <ThreadRow />
      </span>
      <span className="fig-both-ways">
        <span className="fig-way">
          <Cap of="reply-later" />
          <span className="fig-arrow" />
        </span>
        <span className="fig-way">
          <span className="fig-arrow" data-back="" />
          <Cap of="reply-later" />
        </span>
      </span>
      <span className="fig-piles">
        <Pile label="Reply later" />
      </span>
    </>
  );
}

const WHEN: readonly string[] = [
  "Later today",
  "Tomorrow",
  "This weekend",
  "Next week",
  "A date you pick",
];

function Snooze() {
  return (
    <>
      <span className="fig-chip">
        Snooze
        <Cap of="snooze" />
      </span>
      <span className="fig-time">
        {WHEN.map((when, at) => (
          <span className="fig-stop" key={when} data-open={at === WHEN.length - 1 ? "" : undefined}>
            <span className="fig-stop-name">{when}</span>
            <span className="fig-tick" />
          </span>
        ))}
      </span>
      <span className="fig-axis">
        <span>Now</span>
        <span>Later</span>
      </span>
    </>
  );
}

const BAR: readonly { command: CommandId; label: string }[] = [
  { command: "reply-later", label: "Reply later" },
  { command: "set-aside", label: "Set aside" },
  { command: "snooze", label: "Snooze" },
  { command: "toggle-seen", label: "Seen" },
  { command: "archive", label: "Archive" },
  { command: "trash", label: "Trash" },
];

/** A row with the gutter's box ticked, which is the app's own checkbox and its own tick. */
function Checked() {
  return (
    <span className="fig-row" data-state="selected">
      <span className="fig-tick-box">
        <Icon d={icons.CHECK} size={10} />
      </span>
      <span className="fig-bar" data-w="name" />
      <span className="fig-bar" data-w="line" />
    </span>
  );
}

/** The same foot of the same list, before a selection and during one. */
function Selection() {
  return (
    <>
      <span className="fig-list">
        <ThreadRow />
        <ThreadRow />
        <ThreadRow />
        <span className="fig-piles">
          <Pile label="Reply later" />
          <Pile label="Set aside" />
        </span>
      </span>
      <span className="fig-way">
        <Cap of="select" />
        <span className="fig-arrow" />
      </span>
      <span className="fig-list">
        <Checked />
        <Checked />
        <ThreadRow />
        <span className="fig-action-bar">
          <span className="fig-mark">2 selected</span>
          <span className="fig-verbs">
            {BAR.map((verb) => (
              <span className="fig-chip" key={verb.command}>
                {verb.label}
                <Cap of={verb.command} />
              </span>
            ))}
          </span>
        </span>
      </span>
    </>
  );
}

/** One thread you owe an answer to, and the box the answer goes in. */
function Owed() {
  return (
    <span className="fig-item">
      <span className="fig-lines">
        <span className="fig-msg-head">
          <span className="fig-avatar" />
          <span className="fig-bar" data-w="name" />
          <span className="fig-bar" data-w="time" />
        </span>
        <span className="fig-bar" data-w="wide" />
        <span className="fig-bar" data-w="short" />
      </span>
      <span className="fig-reply">
        <span className="fig-bar" data-w="line" />
        <span className="fig-send">
          Send
          <Cap of="send" />
        </span>
      </span>
    </span>
  );
}

function Focus() {
  return (
    <>
      <Owed />
      <Owed />
    </>
  );
}

/** The ten seconds a send waits in, drawn as the length of time it is. */
function Undo() {
  return (
    <>
      <span className="fig-toast">
        Sent to Maya
        <span className="fig-toast-action">
          Undo
          <Cap of="undo" />
        </span>
      </span>
      <span className="fig-drop" />
      <span className="fig-line">
        <span className="fig-chip">
          Send
          <Cap of="send" />
        </span>
        <span className="fig-clock">
          <span className="fig-track">
            <span className="fig-held">Ten seconds</span>
          </span>
          <span className="fig-axis">
            <span>Undo, and the draft comes back</span>
            <span>It goes</span>
          </span>
        </span>
      </span>
    </>
  );
}

function Merge() {
  return (
    <>
      <span className="fig-list">
        <ThreadRow />
        <ThreadRow />
      </span>
      <span className="fig-brace" />
      <span className="fig-way">
        <Cap of="merge" />
        <span className="fig-arrow" />
      </span>
      <span className="fig-merged">
        <ThreadRow />
        <span className="fig-banner" data-note="">
          Merged from 2 threads
          <span className="fig-banner-action">Unmerge</span>
        </span>
      </span>
    </>
  );
}

const REACH: readonly string[] = ["Inbox", "Feed", "Paper Trail", "Screened out", "Spam", "Trash"];

function SearchReach() {
  return (
    <>
      <span className="fig-field">
        <span className="fig-bar" data-w="query" />
        <Cap of="search" />
      </span>
      <span className="fig-drop" />
      <span className="fig-zone">
        <span className="fig-mark">This device, first</span>
        <span className="fig-zone-body">
          {REACH.map((place) => (
            <span className="fig-chip" key={place}>
              {place}
            </span>
          ))}
        </span>
      </span>
      <span className="fig-drop" data-ask="" />
      <span className="fig-zone" data-remote="">
        <span className="fig-mark">The provider, when you ask</span>
        <span className="fig-zone-body">
          <span className="fig-chip" data-action="">
            Search older mail on Gmail
          </span>
        </span>
      </span>
    </>
  );
}

function Storage() {
  return (
    <>
      <span className="fig-mailbox">
        <span className="fig-mailbox-name">Your mailbox, all of it, at the provider</span>
        <span className="fig-held-mail">
          <span className="fig-mailbox-name">On this device, the last month</span>
        </span>
      </span>
      <span className="fig-axis">
        <span>Older</span>
        <span>Now</span>
      </span>
    </>
  );
}

const MAILBOXES: readonly { name: string; hue: string; key: string }[] = [
  { name: "Work", hue: "hue-2", key: ACCOUNT_KEYS[0] },
  { name: "Personal", hue: "hue-4", key: ACCOUNT_KEYS[1] },
];

const OWN: readonly string[] = ["Its own places", "Its own rules", "Its own window"];

function Accounts() {
  return (
    <>
      <span className="fig-switcher">
        {MAILBOXES.map((mailbox, at) => (
          <span className="fig-menu-row" key={mailbox.name} data-on={at === 0 ? "" : undefined}>
            <span className="fig-hue" data-hue={mailbox.hue} />
            {mailbox.name}
            <KeyText>{keyLabel(mailbox.key)}</KeyText>
          </span>
        ))}
        <span className="fig-menu-row" data-apart="">
          All accounts
          <Cap of="accounts" />
        </span>
      </span>
      <span className="fig-mailboxes">
        {MAILBOXES.map((mailbox) => (
          <span className="fig-mailbox-card" key={mailbox.name} data-hue={mailbox.hue}>
            <span className="fig-box-name">{mailbox.name}</span>
            <span className="fig-zone-body">
              {OWN.map((each) => (
                <span className="fig-chip" key={each}>
                  {each}
                </span>
              ))}
            </span>
          </span>
        ))}
      </span>
    </>
  );
}

/** Three switches in a row, all off, and the quiet that is what off means. */
function Notify() {
  return (
    <>
      <span className="fig-chip">New mail</span>
      <span className="fig-drop" />
      <span className="fig-gates">
        <span className="fig-switch">
          <span className="fig-toggle">
            <span className="fig-knob" />
          </span>
          <span className="fig-switch-name">
            This thread
            <Cap of="notify" />
          </span>
        </span>
        <span className="fig-switch">
          <span className="fig-toggle">
            <span className="fig-knob" />
          </span>
          <span className="fig-switch-name">This person</span>
        </span>
        <span className="fig-switch">
          <span className="fig-toggle">
            <span className="fig-knob" />
          </span>
          <span className="fig-switch-name">This place</span>
        </span>
      </span>
      <span className="fig-drop" data-faint="" />
      <span className="fig-chip" data-out="">
        Nothing
      </span>
    </>
  );
}

const STAYS: readonly string[] = [
  "The mail in your window",
  "Your piles, notes, renames and clips",
  "Your sender rules",
  "Every search you run",
];

const LEAVES: readonly string[] = [
  "Mail, to and from your provider",
  "Your backup, encrypted here first",
];

function Privacy() {
  return (
    <>
      <span className="fig-side">
        <span className="fig-mark">Stays on this device</span>
        {STAYS.map((each) => (
          <span className="fig-chip" key={each}>
            {each}
          </span>
        ))}
      </span>
      <span className="fig-side" data-leaves="">
        <span className="fig-mark">Leaves, because it has to</span>
        {LEAVES.map((each) => (
          <span className="fig-chip" key={each}>
            {each}
          </span>
        ))}
      </span>
    </>
  );
}

const FIGURES: Record<FigureId, () => ReactElement> = {
  routing: Routing,
  boxes: Boxes,
  window: Window,
  keyboard: Keyboard,
  "screener-card": ScreenerCard,
  thread: Thread,
  trackers: Trackers,
  piles: Piles,
  snooze: Snooze,
  selection: Selection,
  focus: Focus,
  undo: Undo,
  merge: Merge,
  "search-reach": SearchReach,
  storage: Storage,
  accounts: Accounts,
  notify: Notify,
  privacy: Privacy,
};

export function Figure({ of }: { of: FigureId }) {
  const Drawn = FIGURES[of];
  return (
    <div className="fig" data-fig={of}>
      <Drawn />
    </div>
  );
}

export default Figure;
