import { useEffect, useRef, useState, type ReactNode } from "react";
import { Button, Key, Sheet } from "../ui";
import type { CommandId } from "../keys/bindings";
import { useOverlays } from "../store/useOverlays";
import { cap } from "./format";
import "./tour.css";

/**
 * The slideshow a newly added account ends on, and the first row of the help menu after that.
 *
 * Nine slides, because the first minute is the only one anybody spends looking for what is
 * different, and almost nothing here works the way their last mail client did. Each is a heading, a
 * small figure of the real screen, and a line or two: the figures are drawn from tokens rather than
 * being pictures, so they follow the theme and cannot go stale the way a screenshot does.
 *
 * Nothing in the copy claims anything docs/features.md does not specify, and every sentence still
 * reads with the keycaps taken out of it, which is what a phone does to them.
 */

/**
 * A keycap, always the binding table's answer for that verb rather than a letter typed into the
 * copy, so a remapped key teaches the key it was remapped to. `cap` is what every other button in
 * the app prints: the bare letter when it is unmodified, real glyphs when it is not.
 */
function Cap({ of, size }: { of: CommandId; size?: "sm" | "md" }) {
  const key = cap(of);
  return key ? <Key size={size}>{key}</Key> : null;
}

/** The eight verbs slide five is about, each printing what the table gives it. */
const VERBS: readonly { command: CommandId; label: string }[] = [
  { command: "archive", label: "Archive" },
  { command: "toggle-seen", label: "Seen" },
  { command: "trash", label: "Trash" },
  { command: "spam", label: "Spam" },
  { command: "note", label: "Note" },
  { command: "ignore", label: "Ignore" },
  { command: "merge", label: "Merge" },
  { command: "contact-card", label: "Contact card" },
];

/** The palette's groups, in the order it lists them. */
const GROUPS: readonly string[] = ["Places", "Labels", "Other", "Actions", "People", "Settings"];

/** The snooze picker's choices, in the order it offers them. */
const WHEN: readonly string[] = [
  "Later today",
  "Tomorrow",
  "This weekend",
  "Next week",
  "Pick a date and time",
];

interface Slide {
  title: string;
  figure: ReactNode;
  body: ReactNode;
}

const SLIDES: readonly Slide[] = [
  {
    title: "Nobody new reaches you until you say so",
    figure: (
      <div className="tour-fig">
        <span className="tour-card">
          <span className="tour-avatar" />
          <span className="tour-stack">
            <span className="tour-bar" data-w="name" />
            <span className="tour-bar" data-w="subject" />
            <span className="tour-tag">Written by a person · suggested Inbox</span>
          </span>
          <span className="tour-choices">
            <span className="tour-chip">
              Yes
              <Cap of="screen-yes" size="sm" />
            </span>
            <span className="tour-chip">
              Elsewhere
              <Cap of="screen-elsewhere" size="sm" />
            </span>
            <span className="tour-chip">
              No
              <Cap of="screen-no" size="sm" />
            </span>
          </span>
        </span>
      </div>
    ),
    body: (
      <>
        <p>
          The first message from a sender you have no rule for waits in the Screener rather than in
          a box. Yes <Cap of="screen-yes" /> sends them where the app suggests, Elsewhere{" "}
          <Cap of="screen-elsewhere" /> picks somewhere else, No <Cap of="screen-no" /> screens them
          out.
        </p>
        <p>
          Nothing is ever sent back to the sender either way. Everyone you already write to was
          screened in when the account was added, so what waits here is somebody genuinely new.
        </p>
      </>
    ),
  },
  {
    title: "Three boxes, not one inbox",
    figure: (
      <div className="tour-fig">
        <span className="tour-boxes">
          <span className="tour-box" data-on="">
            Inbox
            <Cap of="place-inbox" size="sm" />
          </span>
          <span className="tour-box">
            Feed
            <Cap of="place-feed" size="sm" />
          </span>
          <span className="tour-box">
            Paper Trail
            <Cap of="place-paper-trail" size="sm" />
          </span>
        </span>
      </div>
    ),
    body: (
      <>
        <p>
          Inbox <Cap of="place-inbox" /> is people. Feed <Cap of="place-feed" /> is what you
          subscribed to, drawn open, with no counts and no read state. Paper Trail{" "}
          <Cap of="place-paper-trail" /> is receipts, confirmations and the mail a machine sent you.
        </p>
        <p>
          Where a sender goes is one decision rather than a filter to keep up, and the contact card
          changes it.
        </p>
      </>
    ),
  },
  {
    title: "Reply later and Set aside, instead of flags",
    figure: (
      <div className="tour-fig">
        <span className="tour-piles">
          <span className="tour-pile">
            <span className="tour-pile-edge" />
            <span className="tour-pile-card">
              <span className="tour-pile-label">Reply later</span>
              <span className="tour-bar" data-w="subject" />
              <span className="tour-bar" data-w="name" />
            </span>
          </span>
          <span className="tour-pile">
            <span className="tour-pile-edge" />
            <span className="tour-pile-card">
              <span className="tour-pile-label">Set aside</span>
              <span className="tour-bar" data-w="subject" />
              <span className="tour-bar" data-w="name" />
            </span>
          </span>
        </span>
      </div>
    ),
    body: (
      <>
        <p>
          Reply later <Cap of="reply-later" /> and Set aside <Cap of="set-aside" /> move a thread out
          of the list and into a stack at its foot. The same key puts it back.
        </p>
        <p>
          Focus & Reply is every thread you owe an answer to, one under the next, with a reply box
          beside each one.
        </p>
      </>
    ),
  },
  {
    title: "Snooze, and reminders when nobody answers",
    figure: (
      <div className="tour-fig">
        <span className="tour-menu">
          {WHEN.map((when) => (
            <span className="tour-menu-row" key={when}>
              {when}
            </span>
          ))}
          <span className="tour-menu-row" data-apart="">
            If no reply by
          </span>
        </span>
      </div>
    ),
    body: (
      <>
        <p>
          Snooze <Cap of="snooze" /> takes a thread away until later today, tomorrow, this weekend,
          next week, or a date you pick. It comes back to the top of the place it left.
        </p>
        <p>
          If no reply by is the other half of the picker: the thread comes back only if nobody has
          written since, and a reply cancels it.
        </p>
      </>
    ),
  },
  {
    title: "One key per verb",
    figure: (
      <div className="tour-fig">
        <span className="tour-verbs">
          {VERBS.map((verb) => (
            <span className="tour-verb" key={verb.command}>
              {verb.label}
              <Cap of={verb.command} size="sm" />
            </span>
          ))}
        </span>
      </div>
    ),
    body: (
      <>
        <p>
          Archive, seen, trash, spam, note, ignore, merge, contact card. One unmodified key each:
          nothing is chorded and nothing is modal.
        </p>
        <p>
          Every button prints the key it answers to, which is how the mouse teaches the keyboard,
          and the shortcut sheet <Cap of="shortcuts" /> is the whole table.
        </p>
      </>
    ),
  },
  {
    title: "The palette is the only menu",
    figure: (
      <div className="tour-fig">
        <span className="tour-palette">
          <span className="tour-field">
            <span className="tour-bar" data-w="query" />
          </span>
          {GROUPS.map((group) => (
            <span className="tour-group" key={group}>
              <span className="tour-group-label">{group}</span>
              <span className="tour-bar" data-w="row" />
            </span>
          ))}
        </span>
      </div>
    ),
    body: (
      <>
        <p>
          One panel <Cap of="command-palette" /> with everything in it: Places, Labels, Other,
          Actions, People and Settings. Typing filters across all six at once.
        </p>
        <p>
          There is no sidebar and no toolbar to hunt through. This is the only menu, and every
          setting in the app is reached from it.
        </p>
      </>
    ),
  },
  {
    title: "The things kept beside the mail",
    figure: (
      <div className="tour-fig">
        <span className="tour-kept">
          <span className="tour-kept-item" data-note="">
            Note
          </span>
          <span className="tour-kept-item">Renamed</span>
          <span className="tour-kept-item">Merged</span>
          <span className="tour-kept-item">Clip</span>
          <span className="tour-kept-item">All files</span>
        </span>
      </div>
    ),
    body: (
      <>
        <p>
          A private note on a thread, a subject you renamed, threads merged into one, a passage saved
          as a clip, and every attachment in one place.
        </p>
        <p>
          None of it is a change to the mailbox. A rename is yours alone, and a reply still carries
          the subject the sender wrote.
        </p>
      </>
    ),
  },
  {
    title: "Quiet by default",
    figure: (
      <div className="tour-fig">
        <span className="tour-message">
          <span className="tour-banner">Images blocked</span>
          <span className="tour-blocked" />
          <span className="tour-bar" data-w="row" />
          <span className="tour-bar" data-w="subject" />
        </span>
      </div>
    ),
    body: (
      <>
        <p>
          Notifications are off everywhere until you turn one on, for a thread, for a person, or for
          a place.
        </p>
        <p>
          Remote images are blocked and trackers are stripped before a message is drawn. The dock
          badge counts unseen Inbox threads and nothing else.
        </p>
      </>
    ),
  },
  {
    title: "Undo covers everything",
    figure: (
      <div className="tour-fig">
        <span className="tour-toast">
          <span className="tour-toast-text">Archived</span>
          <span className="tour-toast-action">
            Undo
            <Cap of="undo" size="sm" />
          </span>
        </span>
      </div>
    ),
    body: (
      <>
        <p>
          Undo <Cap of="undo" /> takes back the last thing you did: an archive, a pile, a screening
          decision, a whole selection at once, and a send inside its ten seconds.
        </p>
        <p>
          That is everything. The question mark in the corner opens this again, and the guide beside
          it goes into the rest.
        </p>
      </>
    ),
  },
];

export function Tour() {
  const open = useOverlays((s) => s.open) === "tour";
  const close = useOverlays((s) => s.close);
  if (!open) return null;
  return <Slides onClose={close} />;
}

/**
 * The slides, as their own component so that which one is up is state that only exists while the
 * tour does. Reopening it from the help menu or the palette mounts this again and starts at one,
 * which is what somebody who asked for the tour a second time is asking for.
 */
function Slides({ onClose }: { onClose: () => void }) {
  const [at, setAt] = useState(0);
  const primary = useRef<HTMLButtonElement | null>(null);
  const last = at === SLIDES.length - 1;
  const slide = SLIDES[at];

  // The primary control takes the focus, so Return and Space advance without this panel binding
  // either of them. The Sheet focuses `[data-autofocus]`, which Button has no way to carry through
  // to its element, so the focus is taken here instead.
  useEffect(() => {
    primary.current?.focus();
  }, []);

  // The arrows are the panel's while it is open. The app's keymap is already shadowed by the
  // overlay frame, and neither arrow is bound in it anyway.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing || (e.key !== "ArrowRight" && e.key !== "ArrowLeft")) return;
      e.preventDefault();
      const delta = e.key === "ArrowRight" ? 1 : -1;
      setAt((was) => Math.min(SLIDES.length - 1, Math.max(0, was + delta)));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <Sheet
      open
      title="Getting started"
      size="wide"
      onClose={onClose}
      foot={
        // The foot is flex-end, so this is one child that takes the row and lays itself out.
        <div className="tour-foot">
          <Button variant="ghost" onClick={onClose}>
            Skip
          </Button>
          <span className="tour-dots">
            {SLIDES.map((each, index) => (
              <button
                key={each.title}
                type="button"
                className="tour-dot"
                data-on={index === at ? "" : undefined}
                aria-current={index === at || undefined}
                aria-label={`Slide ${index + 1}: ${each.title}`}
                onClick={() => setAt(index)}
              />
            ))}
          </span>
          <Button disabled={at === 0} onClick={() => setAt(at - 1)}>
            Back
          </Button>
          <Button
            ref={primary}
            variant="primary"
            onClick={() => (last ? onClose() : setAt(at + 1))}
          >
            {last ? "Done" : "Next"}
          </Button>
        </div>
      }
    >
      <div className="tour" data-slide={at}>
        {/* Keyed on the slide, so each one arrives with the fade rather than the words changing
            under the eye. */}
        <div className="tour-slide" key={slide.title}>
          <h3 className="tour-title">{slide.title}</h3>
          {slide.figure}
          <div className="tour-copy">{slide.body}</div>
        </div>
      </div>
    </Sheet>
  );
}

export default Tour;
