import { useEffect, useMemo, useState } from "react";
import { Avatar, Button, Confirm, EmptyState, Pill, Sheet, Toggle } from "../ui";
import { registerCommands, runCommand } from "../keys/commands";
import { useKeyContext } from "../keys/keymap";
import type { Destination, ScreenerCard } from "../ipc";
import { useMail } from "../store/useMail";
import {
  DESTINATIONS,
  destinationName,
  domainOf,
  domainRuleAllowed,
  useScreener,
} from "../store/useScreener";
import { cap, displayName, isBrand } from "./format";
import { BodyMissing, BodySkeleton, MessageBody } from "./MessageBody";
import "./screener.css";

/**
 * The Screener: the whole stage, one card per sender waiting at the door.
 *
 * A card is the message rather than a row that opens one, which is why this is not a list column:
 * everything a decision needs is on the card, and the decision is three buttons wide.
 *
 * Nothing here congratulates anybody and nothing here is a count except the sentence at the top,
 * which is the number of people it is about.
 */

const WORDS = [
  "Nobody",
  "One",
  "Two",
  "Three",
  "Four",
  "Five",
  "Six",
  "Seven",
  "Eight",
  "Nine",
  "Ten",
];

/** Small numbers read as words in a sentence and as digits in a pill, which is where the pill is. */
const spell = (n: number): string => WORDS[n] ?? String(n);

export function Screener() {
  const accountId = useMail((s) => s.accountId);
  const cards = useScreener((s) => s.cards);
  const phase = useScreener((s) => s.phase);
  const focused = useScreener((s) => s.focused);
  const expanded = useScreener((s) => s.expanded);
  const deciding = useScreener((s) => s.deciding);
  const load = useScreener((s) => s.load);
  const focus = useScreener((s) => s.focus);
  const step = useScreener((s) => s.step);
  const toggleExpanded = useScreener((s) => s.toggleExpanded);
  const decide = useScreener((s) => s.decide);
  const clearAll = useScreener((s) => s.clearAll);

  const [picking, setPicking] = useState<string | null>(null);
  const [clearing, setClearing] = useState(false);

  useEffect(() => {
    void load(accountId);
  }, [accountId, load]);

  // The top card takes the keyboard as soon as there is one. Every verb on this screen acts on the
  // focused card, and a Screener whose first `y` does nothing is a Screener you press `y` at twice.
  useEffect(() => {
    if (focused === null && cards.length > 0) focus(cards[0].threadKey);
  }, [cards, focused, focus]);

  // The card owns `y`, `v` and `n` while the Screener is up, which is the one place in the app
  // where a letter means something other than what the list would have made of it.
  useKeyContext("screener");
  // A panel is in front, so the card's keys and the view's both stand back until it closes.
  useKeyContext("overlay", picking !== null || clearing);

  // Every verb here acts on the focused card, and on an empty pile there is none, which is how a
  // key that would act on nothing comes to do nothing. A card whose decision is already out is
  // the same: its buttons are down, and the letter is the same button.
  useEffect(() => {
    const on = (run: (card: ScreenerCard) => void) => () => {
      const state = useScreener.getState();
      const card = state.cards.find((c) => c.threadKey === state.focused);
      if (card && !state.deciding.includes(card.threadKey)) run(card);
    };
    return registerCommands({
      "screen-yes": on((card) => void decide(card.threadKey, card.suggestion, false)),
      "screen-elsewhere": on((card) => setPicking(card.threadKey)),
      "screen-no": on((card) => void decide(card.threadKey, "screened-out", false)),
      // `r` screens in to the Inbox and opens a reply, and there is no composer to open one in
      // yet. Half of it is worse than none of it, so nobody owns the verb and the button does
      // nothing, the same way Reply in the reading pane does nothing.
      "select-next": () => step(1),
      "select-prev": () => step(-1),
      "open-selection": () => {
        const key = useScreener.getState().focused;
        if (key) toggleExpanded(key);
      },
      undo: () => void useScreener.getState().undo(accountId),
    });
  }, [accountId, decide, step, toggleExpanded]);

  const picked = useMemo(
    () => cards.find((c) => c.threadKey === picking) ?? null,
    [cards, picking],
  );

  return (
    <main className="stage">
      <section className="screener">
        <div className="screen-intro">
          <h1>Screener</h1>
          {cards.length > 0 ? (
            <p>
              {`${spell(cards.length)} ${cards.length === 1 ? "person" : "people"} wrote to you for the first time. Say where their mail goes, or that it goes nowhere. Nobody is told.`}
            </p>
          ) : null}
        </div>

        {cards.length === 0 ? (
          <div className="screen-blank">
            {phase === "loading" ? null : <EmptyState>No one is waiting</EmptyState>}
          </div>
        ) : (
          <div className="screen-list">
            <div className="screen-tools">
              <Button variant="ghost" onClick={() => setClearing(true)}>
                Clear all
              </Button>
            </div>

            {cards.map((card) => (
              <Card
                key={card.threadKey}
                card={card}
                focused={card.threadKey === focused}
                open={card.threadKey === expanded}
                deciding={deciding.includes(card.threadKey)}
                onFocus={() => focus(card.threadKey)}
                onDecide={(destination) => void decide(card.threadKey, destination, false)}
                onElsewhere={() => setPicking(card.threadKey)}
              />
            ))}

            <p className="screen-note">
              Screened-out mail sits under Screened out for as long as this account keeps mail on the
              device. Change your mind from a sender's contact card at any time.
            </p>
          </div>
        )}
      </section>

      <Picker
        card={picked}
        onClose={() => setPicking(null)}
        onChoose={(destination, wholeDomain) => {
          setPicking(null);
          if (picked) void decide(picked.threadKey, destination, wholeDomain);
        }}
      />

      <Sheet
        open={clearing}
        size="mini"
        title="Clear the Screener"
        onClose={() => setClearing(false)}
      >
        <Confirm
          title={`Screen out ${spell(cards.length).toLowerCase()} ${cards.length === 1 ? "sender" : "senders"}?`}
          body={
            <p>
              Nothing is sent. Their mail goes to Screened out from now on, and any of them can be
              let back in from their contact card.
            </p>
          }
          confirmLabel="Screen them out"
          onConfirm={() => {
            setClearing(false);
            void clearAll(accountId);
          }}
          onCancel={() => setClearing(false)}
        />
      </Sheet>
    </main>
  );
}

interface CardProps {
  card: ScreenerCard;
  focused: boolean;
  open: boolean;
  /** The decision is out. The card stays, with its buttons down, until it comes back. */
  deciding: boolean;
  onFocus: () => void;
  onDecide: (destination: Destination) => void;
  onElsewhere: () => void;
}

function Card({ card, focused, open, deciding, onFocus, onDecide, onElsewhere }: CardProps) {
  const view = useScreener((s) => s.views[card.threadKey]);
  const viewPhase = useScreener((s) => s.viewPhase[card.threadKey]);
  const retryView = useScreener((s) => s.retryView);
  const name = displayName(card.sender);
  const suggested = destinationName(card.suggestion);
  const message = view?.messages.at(-1);

  return (
    <article
      className="screen-card"
      data-sender={card.sender.address}
      data-selected={focused ? "" : undefined}
      data-state={deciding ? "deciding" : undefined}
      onClick={onFocus}
    >
      <Avatar name={name} address={card.sender.address} brand={isBrand(card.sender)} />

      <div className="screen-who">
        <div className="screen-name">
          {name}
          <span className="addr">{card.sender.address}</span>
        </div>
        <div className="screen-subject">{card.subject}</div>
        <div className="screen-snippet">{card.snippet}</div>
        <Pill tone="wash">{`${card.reason} · suggested ${suggested}`}</Pill>
      </div>

      <div className="screen-actions">
        <Button
          variant="primary"
          keycap={cap("screen-yes")}
          disabled={deciding}
          onClick={() => onDecide(card.suggestion)}
        >
          {`Yes, to ${suggested}`}
        </Button>
        <Button keycap={cap("screen-elsewhere")} disabled={deciding} onClick={onElsewhere}>
          Elsewhere
        </Button>
        <Button
          variant="ghost"
          keycap={cap("screen-no")}
          disabled={deciding}
          onClick={() => onDecide("screened-out")}
        >
          No
        </Button>
      </div>

      {open ? (
        <div className="screen-message">
          {message ? (
            <MessageBody html={message.html} surface={message.surface} />
          ) : viewPhase === "error" ? (
            <BodyMissing onRetry={() => retryView(card.threadKey)} />
          ) : (
            <BodySkeleton />
          )}
          <div className="screen-message-foot">
            <Button keycap={cap("screen-reply")} onClick={() => runCommand("screen-reply")}>
              Reply
            </Button>
            <span className="screen-message-note">
              Replying screens this sender in to your Inbox.
            </span>
          </div>
        </div>
      ) : null}
    </article>
  );
}

interface PickerProps {
  card: ScreenerCard | null;
  onClose: () => void;
  onChoose: (destination: Destination, wholeDomain: boolean) => void;
}

/** `v`. The three boxes, and the domain rule where a domain can carry one. */
function Picker({ card, onClose, onChoose }: PickerProps) {
  const [wholeDomain, setWholeDomain] = useState(false);

  useEffect(() => {
    if (card) setWholeDomain(false);
  }, [card]);

  if (!card) return null;

  const domain = domainOf(card.sender.address);
  // A consumer domain is not one sender, and `state::write::set_rule` refuses the rule, so the
  // toggle is not offered rather than offered and taken back.
  const allowed = domainRuleAllowed(card.sender.address);

  return (
    <Sheet open size="mini" title="Where does their mail go?" onClose={onClose}>
      <ul className="screen-picker">
        {DESTINATIONS.map((option) => (
          <li key={option.destination}>
            <button
              type="button"
              className="screen-option"
              onClick={() => onChoose(option.destination, allowed && wholeDomain)}
            >
              {option.label}
            </button>
          </li>
        ))}
      </ul>
      {allowed ? (
        <Toggle
          checked={wholeDomain}
          onChange={setWholeDomain}
          label={`Everyone at ${domain}`}
          note="One rule for the whole company rather than for this person."
        />
      ) : null}
    </Sheet>
  );
}

export default Screener;
