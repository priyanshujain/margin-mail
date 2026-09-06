import { useEffect, useState } from "react";
import { Button, Key, NO_AUTOFILL, Popover } from "../ui";
import { useKeyContext } from "../keys/keymap";
import type { SnoozeKind, SnoozeTimes } from "../ipc";
import { useSettings } from "../store/useSettings";
import { useSnooze } from "../store/useSnooze";
import "./snooze.css";

/**
 * `b`. Six choices, their keys, and the moment each of them means.
 *
 * The moments are computed here and not in Rust, because "this weekend" is a fact about the person
 * looking at the picker rather than about the mailbox: their clock, their zone, and the four times
 * they set once in settings. Rust is handed an instant.
 *
 * The two choices that ask for a date open in place rather than in a second panel. A picker that
 * had to be dismissed to reach a date field would be two overlays deep for the most ordinary thing
 * on it.
 */

const clock = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});
const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const dayMonth = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });

const DAY_MS = 86_400_000;

const midnight = (ms: number): number => {
  const day = new Date(ms);
  day.setHours(0, 0, 0, 0);
  return day.getTime();
};

/** Whole calendar days from now to then, negative for a moment that has already gone. */
const daysAhead = (ms: number, now: number): number =>
  Math.round((midnight(ms) - midnight(now)) / DAY_MS);

/**
 * When a snooze is due, said the way a person would.
 *
 * A moment that has passed says so plainly rather than pretending: a thread that should have come
 * back yesterday and is still waiting is "Due yesterday", not "Yesterday 08:00", because the point
 * of the line is that it is late.
 */
export function returnTime(ms: number, now = Date.now()): string {
  const days = daysAhead(ms, now);
  if (ms <= now) {
    if (days === 0) return "Due today";
    if (days === -1) return "Due yesterday";
    return `Due ${dayMonth.format(ms)}`;
  }
  if (days === 0) return `Today ${clock.format(ms)}`;
  if (days === 1) return `Tomorrow ${clock.format(ms)}`;
  if (days < 7) return `${weekday.format(ms)} ${clock.format(ms)}`;
  return `${dayMonth.format(ms)} ${clock.format(ms)}`;
}

/**
 * A moment so many days on, at a time of day given in minutes from midnight.
 *
 * Minutes rather than hours because that is what `SnoozeTimes` says it is and what Rust stores:
 * eight in the morning is 480. Reading it as an hour put every Tomorrow at eight minutes past
 * midnight, which is the sort of wrong that looks right in a list of times until somebody misses
 * something.
 */
const atMinutes = (from: number, daysOn: number, minutes: number): number => {
  const day = new Date(from);
  day.setDate(day.getDate() + daysOn);
  day.setHours(0, Math.min(Math.max(minutes, 0), 24 * 60 - 1), 0, 0);
  return day.getTime();
};

/** The next given weekday at the given time of day, and never a moment that has already gone. */
function nextWeekdayAt(weekdayIndex: number, minutes: number, now: number): number {
  const ahead = (weekdayIndex - new Date(now).getDay() + 7) % 7;
  const candidate = atMinutes(now, ahead, minutes);
  return candidate > now ? candidate : atMinutes(now, ahead + 7, minutes);
}

export interface SnoozeChoice {
  kind: SnoozeKind;
  label: string;
  keycap: string;
  /** The moment, or null when the choice is a question rather than an answer. */
  at: number | null;
}

/** The six, in the order the picker lists them and the order their keys are in. */
export function snoozeChoices(times: SnoozeTimes, now = Date.now()): SnoozeChoice[] {
  return [
    {
      kind: "later-today",
      label: "Later today",
      keycap: "1",
      at: now + times.laterTodayHours * 3_600_000,
    },
    { kind: "tomorrow", label: "Tomorrow", keycap: "2", at: atMinutes(now, 1, times.tomorrowAt) },
    { kind: "weekend", label: "This weekend", keycap: "3", at: nextWeekdayAt(6, times.weekendAt, now) },
    { kind: "next-week", label: "Next week", keycap: "4", at: nextWeekdayAt(1, times.nextWeekAt, now) },
    { kind: "date", label: "Pick a date and time", keycap: "5", at: null },
    { kind: "if-no-reply", label: "If no reply by", keycap: "6", at: null },
  ];
}

/** What `<input type="datetime-local">` and `<input type="date">` want, in local time. */
const asInput = (ms: number, withTime: boolean): string => {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  const day = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  return withTime ? `${day}T${pad(d.getHours())}:${pad(d.getMinutes())}` : day;
};

/**
 * Where the popover hangs from, which is the row the verb is about when there is one.
 *
 * `b` is a key rather than a control, so there is nothing it was pressed on. The focused row is
 * the honest anchor: it is the thing that is about to leave the list. Failing that the pane's bar,
 * which is where the Snooze button is, and failing that the head of the list.
 */
export function snoozeAnchor(): HTMLElement | null {
  // One at a time and in this order. A selector list would answer with whichever of them comes
  // first in the document, which is the head of the list every time.
  for (const selector of [".row[data-selected]", ".thread-head", ".pane-bar", ".list-head"]) {
    const found = document.querySelector<HTMLElement>(selector);
    if (found) return found;
  }
  return null;
}

export function SnoozePicker() {
  const open = useSnooze((s) => s.open);
  const anchor = useSnooze((s) => s.anchor);
  const hide = useSnooze((s) => s.hide);
  const choose = useSnooze((s) => s.choose);
  const times = useSettings((s) => s.settings?.snoozeTimes);
  const settingsPhase = useSettings((s) => s.phase);
  const settingsError = useSettings((s) => s.error);

  /** The choice that asked for a date, and what has been typed into it. */
  const [asking, setAsking] = useState<SnoozeChoice | null>(null);
  const [typed, setTyped] = useState("");

  useEffect(() => {
    if (!open) {
      setAsking(null);
      setTyped("");
    }
  }, [open]);

  // In front of the list, so the place keys stand back and the digits below mean these six.
  useKeyContext("overlay", open);

  const choices = open && times ? snoozeChoices(times) : [];

  const take = (choice: SnoozeChoice) => {
    if (!times) return;
    if (choice.at !== null) {
      void choose(choice.kind, choice.at);
      return;
    }
    // A date choice opens its field on tomorrow morning, so the commonest answer to both of them
    // is one more key rather than a form to fill in.
    setAsking(choice);
    setTyped(asInput(atMinutes(Date.now(), 1, times.tomorrowAt), choice.kind === "date"));
  };

  useEffect(() => {
    if (!open || choices.length === 0) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
      const choice = choices.find((c) => c.keycap === e.key);
      if (!choice) return;
      e.preventDefault();
      e.stopPropagation();
      take(choice);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (!open) return null;

  const confirm = () => {
    if (!asking || !typed || !times) return;
    const at = new Date(typed).getTime();
    if (!Number.isFinite(at)) return;
    // A date with no time on it means the start of that day, which for a reminder is the morning
    // rather than midnight.
    void choose(asking.kind, asking.kind === "date" ? at : atMinutes(at, 0, times.tomorrowAt));
  };

  // The popover comes up on the keystroke whether or not the four times are here yet. Before they
  // are it is a quiet line, or the reason they are not coming; a `b` that showed nothing at all
  // until settings had been read was a `b` that got pressed twice.
  const waiting = !times;
  const failed = waiting && settingsPhase === "error";

  return (
    <Popover open anchor={anchor} onClose={hide} width={264} label="Snooze until">
      <div
        className="snooze-picker"
        data-state={failed ? "error" : waiting ? "loading" : undefined}
      >
        {waiting ? (
          <p className="snooze-wait">
            {failed ? `Could not read your snooze times: ${settingsError}` : "Reading your snooze times"}
          </p>
        ) : asking ? (
          <div className="snooze-form">
            <label className="snooze-field">
              {asking.label}
              <input
                type={asking.kind === "date" ? "datetime-local" : "date"}
                value={typed}
                autoFocus
                {...NO_AUTOFILL}
                onChange={(e) => setTyped(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") confirm();
                }}
              />
            </label>
            <div className="snooze-form-foot">
              <Button variant="ghost" size="sm" onClick={() => setAsking(null)}>
                Back
              </Button>
              <Button variant="primary" size="sm" onClick={confirm}>
                Snooze
              </Button>
            </div>
          </div>
        ) : (
          <ul className="snooze-list">
            {choices.map((choice) => (
              <li key={choice.kind}>
                <button type="button" className="snooze-option" onClick={() => take(choice)}>
                  <Key size="sm">{choice.keycap}</Key>
                  <span className="snooze-label">{choice.label}</span>
                  <span className="snooze-when">
                    {choice.at === null ? "" : returnTime(choice.at)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Popover>
  );
}

export default SnoozePicker;
