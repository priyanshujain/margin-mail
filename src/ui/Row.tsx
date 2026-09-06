import { memo } from "react";

import { Avatar } from "./Avatar";
import { Icon } from "./Icon";
import { CHECK } from "./icons";
import "./Row.css";

export interface RowProps {
  sender: string;
  address?: string;
  time: string;
  subject: string;
  snippet?: string;
  /** Shown between the sender and the time when the thread has more than one message. */
  count?: number;
  /** Your note about the thread, one line on the note surface under the row. */
  note?: string;
  /** A company rather than a person, which changes the avatar and nothing else. */
  brand?: boolean;
  /** In All accounts, the hue of the account this landed in, drawn as a 2px left edge. */
  accountHue?: number;
  /**
   * Unseen: the sender and the subject at 600 weight in the full ink, and nothing else. No dot and
   * no band, because a second signal for the same bit is a second thing that can lag behind the
   * first, and when it did the row read as broken. Weight alone is Superhuman's rule.
   */
  unread?: boolean;
  selected?: boolean;
  /** Selection mode is on, so the gutter holds a checkbox. */
  selecting?: boolean;
  checked?: boolean;
  /** One small glyph before the time: starred, has an attachment, or is filed away. */
  mark?: string;
  /** What the glyph means, for the two that are a place rather than a state. */
  markTitle?: string;
  onClick?: () => void;
  onToggleCheck?: () => void;
}

/**
 * Two lines at 58px, and the single most looked at thing in the app.
 *
 * Everything that could be here and is not was left out on purpose: no chips, no labels, no icons
 * that appear on hover, no unread count, no unread dot. A list you scan a hundred times a day earns
 * nothing from decoration, and the two states worth seeing at a glance (new, and yours to answer)
 * are a weight and a note.
 */
function RowBase({
  sender,
  address,
  time,
  subject,
  snippet,
  count,
  note,
  brand,
  accountHue,
  unread,
  selected,
  selecting,
  checked,
  mark,
  markTitle,
  onClick,
  onToggleCheck,
}: RowProps) {
  return (
    <div
      className="row"
      data-new={unread ? "" : undefined}
      data-selected={selected ? "" : undefined}
      data-selecting={selecting ? "" : undefined}
      data-checked={checked ? "" : undefined}
      data-noted={note ? "" : undefined}
      onClick={onClick}
    >
      {accountHue ? <span className="row-account" data-hue={accountHue} /> : null}

      <div className="row-gutter">
        {selecting ? (
          <button
            type="button"
            className="row-check"
            role="checkbox"
            aria-checked={Boolean(checked)}
            aria-label={`Select ${sender}`}
            onClick={(e) => {
              e.stopPropagation();
              onToggleCheck?.();
            }}
          >
            {checked ? <Icon d={CHECK} size={10} /> : null}
          </button>
        ) : null}
      </div>

      <Avatar name={sender} address={address} brand={brand} />

      <div className="row-main">
        <div className="row-top">
          <span className="row-sender">{sender}</span>
          {count && count > 1 ? <span className="row-count">{count}</span> : null}
          {mark ? (
            <span className="row-mark" title={markTitle}>
              <Icon d={mark} size={13} />
            </span>
          ) : null}
          <span className="row-time">{time}</span>
        </div>
        <div className="row-bottom">
          <span className="row-subject">{subject}</span>
          {snippet ? <span className="row-snippet">{snippet}</span> : null}
        </div>
      </div>

      {note ? <span className="row-note">{note}</span> : null}
    </div>
  );
}

/**
 * Two rows that print the same thing are the same row.
 *
 * Compared field by field rather than by identity, because a sync pass hands the list a whole
 * fresh page of summaries whether or not a word of them changed, and a shallow compare would
 * therefore never hold: every visible row would redraw and react-virtuoso would measure the list
 * again, several times a second while a mailbox is still coming in.
 *
 * The two handlers are deliberately not compared. Each is a closure over its own thread's key, so
 * a new one arrives on every render of the list and it always does the same thing; comparing them
 * would mean the rest of this function never ran.
 */
function draws(a: RowProps, b: RowProps): boolean {
  return (
    a.sender === b.sender &&
    a.address === b.address &&
    a.time === b.time &&
    a.subject === b.subject &&
    a.snippet === b.snippet &&
    a.count === b.count &&
    a.note === b.note &&
    a.brand === b.brand &&
    a.accountHue === b.accountHue &&
    a.unread === b.unread &&
    a.selected === b.selected &&
    a.selecting === b.selecting &&
    a.checked === b.checked &&
    a.mark === b.mark &&
    a.markTitle === b.markTitle
  );
}

export const Row = memo(RowBase, draws);
