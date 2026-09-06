import { useState } from "react";
import { Button } from "../ui";
import "./windowchoice.css";

/** The same five spans the Storage window row in Settings offers, in the same order. */
export const WINDOWS = [
  { days: 30, label: "30 days" },
  { days: 90, label: "90 days" },
  { days: 180, label: "180 days" },
  { days: 365, label: "A year" },
  { days: 0, label: "Everything" },
];

/** A window as a sentence names it: "the last month", "the last year", "everything". */
export function spanOf(days: number): string {
  if (days === 0) return "everything";
  if (days === 30) return "the last month";
  if (days === 365) return "the last year";
  return `the last ${days} days`;
}

/**
 * The one question between an account being written and its mail arriving: how far back this
 * device holds. Asked here rather than assumed, because a month is a guess and a year of a busy
 * mailbox is a long wait nobody was told about. The welcome screen and the arriving panel both
 * host it, each with its own heading; this is the row of spans and the button that starts the
 * sync, and nothing else.
 */
export function WindowChoice({
  initial,
  busy,
  onStart,
}: {
  initial: number;
  busy: boolean;
  onStart: (days: number) => void;
}) {
  const [days, setDays] = useState(initial);
  return (
    <div className="window-choice">
      <div className="window-choice-options" role="radiogroup" aria-label="How far back">
        {WINDOWS.map((option) => (
          <button
            key={option.days}
            type="button"
            role="radio"
            className="window-choice-option"
            aria-checked={option.days === days}
            data-on={option.days === days ? "" : undefined}
            disabled={busy}
            onClick={() => setDays(option.days)}
          >
            {option.label}
          </button>
        ))}
      </div>
      <p className="window-choice-note">
        Older mail stays on the server and is fetched when you search for it. This can be changed
        later in Settings.
      </p>
      <div className="window-choice-actions">
        <Button variant="primary" disabled={busy} onClick={() => onStart(days)}>
          {busy ? "Starting" : "Start"}
        </Button>
      </div>
    </div>
  );
}
