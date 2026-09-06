import { useEffect, useRef } from "react";
import { EmptyState, Icon, NO_AUTOFILL, Row, Toggle, icons } from "../ui";
import { useContacts } from "../store/useContacts";
import { useMail } from "../store/useMail";
import type { ContactCard } from "../ipc";
import { DeliversTo } from "./ContactCards";
import { displayName, isBrand } from "./format";
import "./list.css";
import "./contacts.css";

// The Contacts place: everyone with a decision about them, searchable, with the card's two
// controls on the row.
//
// The row is the mail list's row and not a second one, because a person here is read the same way a
// thread is read in the Inbox and a second anatomy would be a second thing to learn. What is on it
// that a thread has not is the destination and the notify switch, which are the two decisions worth
// changing without opening anything.

/** A decision has a day, not an hour, and the row prints it where a thread prints its time. */
const decidedOn = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });

function ContactItem({ card }: { card: ContactCard }) {
  const name = displayName(card.person);
  const save = useContacts((s) => s.save);
  const at = useRef<HTMLDivElement | null>(null);

  return (
    <div className="contact-item" ref={at}>
      <Row
        sender={name}
        address={card.person.address}
        brand={isBrand(card.person)}
        time={card.screenedAtMs === null ? "" : decidedOn.format(card.screenedAtMs)}
        subject={card.person.address}
        note={card.note ?? undefined}
        onClick={() =>
          void useContacts.getState().show(card.person.address, card.accountId, at.current)
        }
      />
      <div className="contact-controls">
        <DeliversTo
          value={card.destination}
          label={`Delivers to, for ${name}`}
          onChange={(destination) =>
            void save(card.person.address, card.accountId, { destination })
          }
        />
        <Toggle
          checked={card.notify}
          onChange={(on) => void save(card.person.address, card.accountId, { notify: on })}
          label="Notify"
        />
      </div>
    </div>
  );
}

export function Contacts() {
  const people = useContacts((s) => s.people);
  const query = useContacts((s) => s.query);
  const phase = useContacts((s) => s.listPhase);
  const setQuery = useContacts((s) => s.setQuery);
  const load = useContacts((s) => s.load);
  const accountId = useMail((s) => s.accountId);

  useEffect(() => {
    void load();
  }, [accountId, load]);

  return (
    <main className="stage contacts">
      <div className="list-head contacts-head">
        <h1 className="list-title">Contacts</h1>
        <label className="contacts-search">
          <Icon d={icons.SEARCH} size={14} />
          <input
            type="search"
            value={query}
            placeholder="Search contacts"
            aria-label="Search contacts"
            {...NO_AUTOFILL}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>
      </div>

      {/* What was there stays there while the next answer is out, and the phase on the list is
          what says so. Emptying it between keystrokes was the flicker: a query that matched nobody
          and then somebody blinked blank on every letter in between. Before any answer at all
          there is nothing to keep, so the row's shape is drawn faintly instead. */}
      <div
        className={people && people.length > 0 ? "list contacts-list" : "list list-blank contacts-list"}
        data-phase={phase}
        aria-busy={phase === "loading"}
      >
        {phase === "error" ? (
          <p className="contacts-note">Could not load your contacts</p>
        ) : people === null ? (
          <div className="contacts-waiting" aria-hidden>
            <span />
            <span />
            <span />
          </div>
        ) : people.length === 0 ? (
          <EmptyState>{query ? "Nobody by that name" : "Nobody yet"}</EmptyState>
        ) : null}
        {people?.map((card) => (
          <ContactItem key={`${card.accountId}:${card.person.address}`} card={card} />
        ))}
      </div>
    </main>
  );
}

export default Contacts;
