import { useEffect, useState } from "react";
import { Avatar, GroupHead, Icon, Popover, Toggle, icons } from "../ui";
import { registerCommands } from "../keys/commands";
import { useContacts } from "../store/useContacts";
import { useMail } from "../store/useMail";
import { useStage } from "../store/useStage";
import type { ContactCard as Card, ContactPatch, Destination } from "../ipc";
import { displayName, fileSize, isBrand, rowTime } from "./format";
import "./contacts.css";

// The contact card, mounted once at the top of the tree and hanging off whatever asked for it.
//
// It is one host rather than a card per name because there is only ever one open, and because a
// popover positioned from a rect does not need to live inside the thing it points at. Who asked is
// in `useContacts`, which is what lets a name in the list, in the pane or on a Feed card all reach
// the same card without any of them knowing this file exists.

/** The four boxes, in the order the Screener offers them. Screening out is the block. */
const DESTINATIONS: [Destination, string][] = [
  ["inbox", "Inbox"],
  ["feed", "Feed"],
  ["paper-trail", "Paper Trail"],
  ["screened-out", "Screened out"],
];

/**
 * Where a sender's mail delivers, and the one control this card exists for.
 *
 * A native select under the app's own chrome: the four boxes are a closed set and a platform picker
 * is what a closed set is, but the platform's border is not this app's border, so `appearance` is
 * dropped and the chevron is drawn from the icon set like every other one.
 */
export function DeliversTo({
  value,
  label,
  onChange,
}: {
  value: Destination;
  label: string;
  onChange: (destination: Destination) => void;
}) {
  return (
    <span className="contact-picker">
      <select
        className="contact-pick"
        aria-label={label}
        value={value}
        onChange={(e) => onChange(e.target.value as Destination)}
      >
        {DESTINATIONS.map(([id, name]) => (
          <option key={id} value={id}>
            {name}
          </option>
        ))}
      </select>
      <Icon d={icons.CHEVRON_DOWN} size={12} />
    </span>
  );
}

/** A decision has a day, not an hour. `format.ts` has the list's dates and this is not one. */
const decidedOn = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });

const domainOf = (address: string): string => address.slice(address.indexOf("@") + 1);

/** "In, on 12 Aug", or that nobody has decided yet, which is what an empty date means. */
function screenedLine(card: Card): string {
  if (card.screenedAtMs === null) return "Not yet";
  const side = card.destination === "screened-out" ? "Out" : "In";
  return `${side}, on ${decidedOn.format(card.screenedAtMs)}`;
}

/**
 * The element the card hangs off when the keyboard asked for it rather than a pointer. The sender's
 * name in the open thread first, because that is where the mockup hangs it, then the focused row,
 * which is where the sender is when nothing is open.
 */
function anchorFor(address: string): HTMLElement | null {
  const names = [...document.querySelectorAll<HTMLElement>(".msg-name")];
  const inThread = names.reverse().find((el) => (el.textContent ?? "").includes(address));
  if (inThread) return inThread;
  return document.querySelector<HTMLElement>(".row[data-selected] .row-sender");
}

function Note({ card }: { card: Card }) {
  const [draft, setDraft] = useState(card.note ?? "");
  const [editing, setEditing] = useState(false);

  // While somebody is typing the field is theirs; the rest of the time it says what is stored, so
  // a note written on another device shows up without the card having to be closed.
  useEffect(() => {
    if (!editing) setDraft(card.note ?? "");
  }, [card.note, editing]);

  const save = () => {
    setEditing(false);
    if (draft === (card.note ?? "")) return;
    void useContacts.getState().save(card.person.address, card.accountId, { note: draft });
  };

  return (
    <textarea
      className="contact-note"
      rows={2}
      value={draft}
      placeholder="Add a note"
      aria-label="Note"
      onFocus={() => setEditing(true)}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={save}
    />
  );
}

export function ContactCardBody({ card }: { card: Card }) {
  const name = displayName(card.person);
  const save = useContacts((s) => s.save);
  const hide = useContacts((s) => s.hide);
  const unsubscribe = useContacts((s) => s.unsubscribe);
  const unsubPhase = useContacts((s) => s.unsubscribePhase[card.person.address] ?? "idle");
  const patch = (change: ContactPatch) => void save(card.person.address, card.accountId, change);

  return (
    <>
      <div className="popover-head">
        <Avatar name={name} address={card.person.address} brand={isBrand(card.person)} size="lg" />
        <div className="contact-who">
          <div className="popover-name">{name}</div>
          <div className="popover-sub">{card.person.address}</div>
        </div>
      </div>

      <div className="popover-rows">
        <div className="popover-row">
          <span className="lab">Delivers to</span>
          <span className="val">
            <DeliversTo
              value={card.destination}
              label="Delivers to"
              onChange={(destination) => patch({ destination })}
            />
          </span>
        </div>

        {/* A consumer domain is not a group of any kind, so the toggle is not offered rather than
            offered and then refused. */}
        {card.domainRuleAllowed ? (
          <div className="contact-toggle">
            <Toggle
              checked={card.domainRule}
              onChange={(on) => patch({ domainRule: on })}
              label={`Everyone at ${domainOf(card.person.address)}`}
            />
          </div>
        ) : null}

        <div className="contact-toggle">
          <Toggle checked={card.notify} onChange={(on) => patch({ notify: on })} label="Notify" />
        </div>

        <div className="popover-row">
          <span className="lab">Screened</span>
          <span className="val">{screenedLine(card)}</span>
        </div>

        <div className="popover-row contact-note-row">
          <span className="lab">Note</span>
          <Note card={card} />
        </div>
      </div>

      {card.recentThreads.length > 0 ? (
        <section className="contact-section">
          <GroupHead>Recent threads</GroupHead>
          {card.recentThreads.map((thread) => (
            <button
              key={thread.key}
              type="button"
              className="contact-line"
              onClick={() => {
                hide();
                useStage.getState().close();
                void useMail.getState().open(thread.key);
              }}
            >
              <span className="contact-line-name">{thread.subject}</span>
              <span className="contact-line-side">{rowTime(thread.dateMs)}</span>
            </button>
          ))}
        </section>
      ) : null}

      {card.files.length > 0 ? (
        <section className="contact-section">
          <GroupHead>Files</GroupHead>
          {card.files.map((file) => (
            <div key={file.id} className="contact-line">
              <span className="contact-line-name">{file.filename}</span>
              <span className="contact-line-side">{fileSize(file.size)}</span>
            </div>
          ))}
        </section>
      ) : null}

      {card.unsubscribe ? (
        <div className="contact-foot">
          <button
            type="button"
            className="contact-unsub"
            data-phase={unsubPhase}
            disabled={unsubPhase === "unsubscribing"}
            onClick={() => void unsubscribe(card.accountId, card.person.address)}
          >
            {unsubPhase === "unsubscribing" ? "Unsubscribing" : "Unsubscribe"}
          </button>
        </div>
      ) : null}
    </>
  );
}

export function ContactCards() {
  const anchor = useContacts((s) => s.anchor);
  const address = useContacts((s) => s.address);
  const card = useContacts((s) => s.card);
  const phase = useContacts((s) => s.phase);
  const hide = useContacts((s) => s.hide);

  // `i` has no owner anywhere else: the card is not a screen, so the host that draws it is what
  // registers the key, for as long as it is mounted, which is always.
  useEffect(
    () =>
      registerCommands({
        "contact-card": () => {
          const { threads, focused } = useMail.getState();
          const thread = threads.find((t) => t.key === focused);
          if (!thread) return;
          const at = anchorFor(thread.from.address);
          if (!at) return;
          void useContacts.getState().show(thread.from.address, thread.accountId, at);
        },
      }),
    [],
  );

  return (
    <Popover
      open={address !== null && anchor !== null}
      anchor={anchor}
      onClose={hide}
      label="Contact card"
    >
      {card ? (
        <ContactCardBody card={card} />
      ) : (
        <div className="contact-waiting" data-phase={phase} aria-busy={phase === "loading"}>
          <span />
          <span />
          <span />
        </div>
      )}
    </Popover>
  );
}

export default ContactCards;
