// Every primitive in every state, on one page, in both palettes.
//
// This is how a restyle is reviewed and how the UI suite proves the design system still looks like
// itself: two screenshots, light and dark, compared against the mockups by eye. It imports nothing
// but React and src/ui, which is the point. If something on a screen cannot be built out of what
// is on this page, the thing to change is a primitive and not the screen.

import { useState, type ReactNode } from "react";
import {
  Avatar,
  AvatarStack,
  Banner,
  Button,
  Confirm,
  EmptyState,
  Field,
  GroupHead,
  Icon,
  icons,
  Key,
  Palette,
  Pill,
  Popover,
  Row,
  Segment,
  Sheet,
  Toast,
  Toggle,
} from "../ui";
import "./kit.css";

function Section({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return (
    <section className="kit-section">
      <h2 className="kit-title">{title}</h2>
      {note ? <p className="kit-note">{note}</p> : null}
      <div className="kit-body">{children}</div>
    </section>
  );
}

function Bench({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="kit-bench">
      <span className="kit-label">{label}</span>
      <div className="kit-items">{children}</div>
    </div>
  );
}

/** A bounded box that catches the fixed positioning of an overlay, so a sheet, the palette and a
 *  toast can be seen in place on a page rather than over it. */
function Stage({
  label,
  size,
  children,
}: {
  label: string;
  size?: "short" | "tall";
  children: ReactNode;
}) {
  return (
    <div className="kit-bench">
      <span className="kit-label">{label}</span>
      <div
        className="kit-stage"
        data-short={size === "short" ? "" : undefined}
        data-tall={size === "tall" ? "" : undefined}
      >
        {children}
      </div>
    </div>
  );
}

const PALETTE_GROUPS = [
  {
    id: "places",
    label: "Places",
    items: [
      { id: "inbox", label: "Inbox", keys: ["1"] },
      { id: "feed", label: "Feed", keys: ["2"] },
      { id: "trail", label: "Paper Trail", keys: ["3"] },
      { id: "later", label: "Reply later", keys: ["4"] },
      { id: "aside", label: "Set aside", keys: ["5"] },
    ],
  },
  {
    id: "actions",
    label: "Actions",
    items: [
      { id: "archive", label: "Archive", keys: ["e"] },
      { id: "snooze", label: "Snooze", hint: "later today, tomorrow, the weekend", keys: ["b"] },
      { id: "note", label: "Note", keys: ["y"] },
    ],
  },
  {
    id: "settings",
    label: "Settings",
    items: [
      { id: "undo", label: "Undo delay", hint: "10 seconds" },
      { id: "fonts", label: "Fonts", hint: "Hanken Grotesk and Literata" },
    ],
  },
];

export default function Kit() {
  const [theme, setTheme] = useState(
    () => document.documentElement.getAttribute("data-theme") ?? "light",
  );
  const [phone, setPhone] = useState(() => document.documentElement.hasAttribute("data-phone"));
  const [touch, setTouch] = useState(() => document.documentElement.hasAttribute("data-touch"));

  const [text, setText] = useState("Priyanshu Jain");
  const [note, setNote] = useState("Always cc the studio address on anything about the lease.");
  const [query, setQuery] = useState("");
  const [notify, setNotify] = useState(true);
  const [images, setImages] = useState(false);
  const [box, setBox] = useState("inbox");
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);

  const setThemeAttr = (next: string) => {
    document.documentElement.setAttribute("data-theme", next);
    setTheme(next);
  };
  const setPhoneAttr = (next: boolean) => {
    document.documentElement.toggleAttribute("data-phone", next);
    setPhone(next);
  };
  const setTouchAttr = (next: boolean) => {
    document.documentElement.toggleAttribute("data-touch", next);
    setTouch(next);
  };

  return (
    <div className="kit">
      <header className="kit-head">
        <h1 className="kit-heading">Margin Mail, the kit</h1>
        <div className="kit-controls">
          <Segment
            label="Theme"
            value={theme}
            onChange={setThemeAttr}
            options={[
              { id: "light", label: "Light" },
              { id: "dark", label: "Dark" },
            ]}
          />
          <Toggle checked={phone} onChange={setPhoneAttr} label="data-phone" />
          <Toggle checked={touch} onChange={setTouchAttr} label="data-touch" />
        </div>
      </header>

      <Section
        title="Row"
        note="Two lines at 58px. The list is the app, so this is the one to get right."
      >
        <div className="kit-list">
          <Row
            unread
            sender="Maya Raghunathan"
            address="maya@example.org"
            time="11:42"
            count={3}
            subject="Dinner on Thursday?"
            snippet="Priya said the place on Church Street takes bookings"
          />
          <Row
            unread
            selected
            sender="Arun Kulkarni"
            address="arun@meridianproperties.in"
            time="10:15"
            subject="Re: Lease renewal for the studio"
            snippet="Attached the revised draft. The only change is clause 7"
          />
          <Row
            unread
            brand
            sender="Airbnb"
            address="automated@airbnb.com"
            time="09:03"
            subject="Your reservation in Lisbon is confirmed"
            snippet="Check-in Friday 12 September after 15:00"
          />
          <Row
            unread
            mark={icons.PAPERCLIP}
            sender="Sam Okafor"
            address="sam@okafor.co"
            time="Yesterday"
            subject="Piano lessons, the form you sent"
            snippet="Got it, thank you. Wednesdays at five work for us"
          />
          <div>
            <Row
              sender="Lena Brandt"
              address="lena@brandt.de"
              time="Yesterday"
              count={7}
              subject="Kitchen bench quote"
              snippet="Sounds good, Julie. Any afternoon next week works"
              note="Ask about the oak finish before confirming"
            />
            <Row
              sender="Dev Patel"
              address="dev@patel.dev"
              time="Mon"
              accountHue={4}
              subject="Slides from the talk"
              snippet="Here they are, plus the reading list I mentioned"
            />
            <Row
              brand
              sender="DocuSign"
              address="dse@docusign.net"
              time="Mon"
              mark={icons.STAR}
              subject="Completed: Studio lease 2026"
              snippet="All parties have completed the envelope"
            />
            <Row
              selecting
              checked
              sender="Hannah Weiss"
              address="hannah@weiss.ch"
              time="Sun"
              count={2}
              subject="Photos from the Hawaii trip"
              snippet="Finally went through them all"
            />
            <Row
              selecting
              sender="Russell Young"
              address="russell@young.name"
              time="30 Aug"
              subject="Pumpkin bread recipe"
              snippet="From my mother's card, transcribed"
            />
          </div>
        </div>
      </Section>

      <Section title="Popover" note="Anchored to what it explains: the contact card, the snooze picker.">
        <div className="kit-bench kit-hang">
          <span className="kit-label">anchored, open</span>
          <div className="kit-items">
            <span className="kit-anchor" ref={setAnchor}>
              <Button icon={icons.INBOX} keycap="i">
                Arun Kulkarni
              </Button>
            </span>
          </div>
        </div>
        <Popover open anchor={anchor} onClose={() => {}} label="Arun Kulkarni">
          <div className="popover-head">
            <Avatar name="Arun Kulkarni" address="arun@meridianproperties.in" size="lg" />
            <span>
              <div className="popover-name">Arun Kulkarni</div>
              <div className="popover-sub">arun@meridianproperties.in</div>
            </span>
          </div>
          <div className="popover-rows">
            <div className="popover-row">
              <span className="lab">Delivers to</span>
              <span className="val">
                <Pill tone="wash">Inbox</Pill>
              </span>
            </div>
            <div className="popover-row">
              <span className="lab">Notify</span>
              <span className="val">On for this thread</span>
            </div>
            <div className="popover-row">
              <span className="lab">Screened</span>
              <span className="val">12 March, to Inbox</span>
            </div>
          </div>
        </Popover>
      </Section>

      <Section title="Button" note="Every one carries the key its verb answers to.">
        <Bench label="variants">
          <Button>Default</Button>
          <Button variant="primary">Send</Button>
          <Button variant="ghost">Ghost</Button>
          <Button variant="danger">Delete</Button>
        </Bench>
        <Bench label="with a key">
          <Button icon={icons.REPLY} keycap="r">
            Reply
          </Button>
          <Button variant="primary" icon={icons.PEN} keycap="c">
            Write
          </Button>
          <Button variant="ghost" icon={icons.CLOCK} keycap="l">
            Reply later
          </Button>
          <Button variant="ghost" icon={icons.SET_ASIDE} keycap="s">
            Set aside
          </Button>
          <Button variant="ghost" icon={icons.SNOOZE} keycap="b">
            Snooze
          </Button>
          <Button variant="ghost" icon={icons.ARCHIVE} keycap="e">
            Archive
          </Button>
          <Button variant="danger" icon={icons.TRASH} keycap="#">
            Trash
          </Button>
        </Bench>
        <Bench label="sizes">
          <Button size="sm" keycap="a">
            Small
          </Button>
          <Button size="md" keycap="a">
            Medium
          </Button>
          <Button size="lg" keycap="a">
            Large
          </Button>
        </Bench>
        <Bench label="icon only">
          <Button iconOnly icon={icons.SEARCH} title="Search (/)" />
          <Button iconOnly icon={icons.PLACES} title="Places (⌘K)" />
          <Button iconOnly icon={icons.MORE} title="More actions (.)" />
          <Button iconOnly variant="ghost" icon={icons.CLOSE} title="Close (⎋)" />
          <Button iconOnly variant="ghost" active icon={icons.BELL} title="Notify me (⇧N)" />
        </Bench>
        <Bench label="disabled">
          <Button disabled>Default</Button>
          <Button variant="primary" disabled>
            Send
          </Button>
          <Button variant="ghost" disabled>
            Ghost
          </Button>
          <Button variant="danger" disabled>
            Delete
          </Button>
        </Bench>
      </Section>

      <Section title="Key" note="The cap on its own, for the palette and the shortcut sheet.">
        <Bench label="standing alone">
          <Key>j</Key>
          <Key>k</Key>
          <Key>⏎</Key>
          <Key>⎋</Key>
          <Key>⌘K</Key>
          <Key>⇧S</Key>
        </Bench>
        <Bench label="inside a control">
          <Key size="sm">1</Key>
          <Key size="sm">e</Key>
          <Key size="sm">z</Key>
        </Bench>
      </Section>

      <Section title="Avatar" note="Initials on one of the eight hues, chosen from the address.">
        <Bench label="sizes">
          <Avatar name="Priyanshu Jain" address="pj@73ai.org" size="xs" />
          <Avatar name="Priyanshu Jain" address="pj@73ai.org" size="sm" />
          <Avatar name="Priyanshu Jain" address="pj@73ai.org" />
          <Avatar name="Priyanshu Jain" address="pj@73ai.org" size="lg" />
        </Bench>
        <Bench label="the eight hues">
          {["a", "b", "c", "d", "e", "f", "g", "h"].map((seed, i) => (
            <Avatar key={seed} name={`Hue ${i + 1}`} address={`${seed}@example.com`} />
          ))}
        </Bench>
        <Bench label="brand marks">
          <Avatar brand name="Airbnb" address="automated@airbnb.com" />
          <Avatar brand name="DocuSign" address="dse@docusign.net" />
          <Avatar brand name="Stripe" address="receipts@stripe.com" />
        </Bench>
        <Bench label="stacked participants">
          <AvatarStack
            people={[
              { name: "Arun Kulkarni", address: "arun@meridianproperties.in" },
              { name: "Priyanshu Jain", address: "pj@73ai.org" },
              { name: "Maya Raghunathan", address: "maya@example.org" },
            ]}
          />
          <span className="kit-quiet">Arun Kulkarni and you · 2 messages</span>
        </Bench>
      </Section>

      <Section title="Pill">
        <Bench label="tones">
          <Pill icon={icons.SHIELD} keycap="6" onClick={() => {}}>
            Screen 3 new senders
          </Pill>
          <Pill tone="wash">Written by a person · suggested Inbox</Pill>
          <Pill tone="quiet" onClick={() => {}}>
            ··· Show quoted text
          </Pill>
          <Pill>2 messages</Pill>
        </Bench>
      </Section>

      <Section title="Segment" note="The three boxes, each printing its number.">
        <Bench label="places">
          <Segment
            label="Places"
            value={box}
            onChange={setBox}
            options={[
              { id: "inbox", label: "Inbox", keycap: "1" },
              { id: "feed", label: "Feed", keycap: "2" },
              { id: "trail", label: "Paper Trail", keycap: "3" },
            ]}
          />
        </Bench>
        <Bench label="disabled, while the last choice is written">
          <Segment
            label="Backup store"
            value="drive"
            onChange={() => {}}
            disabled
            options={[
              { id: "none", label: "Off" },
              { id: "drive", label: "Google Drive" },
              { id: "r2", label: "Cloudflare R2" },
            ]}
          />
        </Bench>
      </Section>

      <Section title="Toggle">
        <div className="kit-settings">
          <Toggle
            checked={notify}
            onChange={setNotify}
            label="Notify me about this thread"
            note="Off everywhere else until you ask, which is the whole point."
          />
          <Toggle
            checked={images}
            onChange={setImages}
            label="Load remote images"
            note="Trackers are stripped before anything loads, and the banner names them."
          />
          <Toggle checked={false} onChange={() => {}} label="Send read receipts" disabled />
        </div>
      </Section>

      <Section title="Field">
        <div className="kit-form">
          <Field label="Display name" value={text} onChange={setText} />
          <Field
            label="Note about this sender"
            value={note}
            onChange={setNote}
            multiline
            rows={3}
            hint="Private to you. It never touches Gmail."
          />
          <Field
            label="Forward to"
            value=""
            onChange={() => {}}
            placeholder="someone@example.com"
            hint="That address is not on this account."
            tone="error"
          />
          <Field label="Account" value="pj@73ai.org" onChange={() => {}} disabled />
        </div>
      </Section>

      <Section title="GroupHead">
        <div className="kit-list">
          <GroupHead>Back</GroupHead>
          <GroupHead action={{ label: "Show all", onClick: () => {} }}>This week</GroupHead>
          <GroupHead rule>You left off here</GroupHead>
        </div>
      </Section>

      <Section title="Banner">
        <div className="kit-column">
          <Banner icon={icons.SHIELD} action={{ label: "Show images", onClick: () => {} }}>
            Blocked <b>1 tracker</b> from HubSpot. Remote images are off for this sender.
          </Banner>
          <Banner
            icon={icons.SHIELD}
            action={{ label: "Show images", busy: true, busyLabel: "Loading images…", onClick: () => {} }}
          >
            Blocked <b>1 tracker</b> from HubSpot. Remote images are off for this sender.
          </Banner>
          <Banner tone="muted" icon={icons.MERGE} action={{ label: "Unmerge", onClick: () => {} }}>
            Merged from two threads.
          </Banner>
          <Banner tone="muted" icon={icons.ENVELOPE}>
            You are ignoring this thread. It will not come back to the top.
          </Banner>
        </div>
      </Section>

      <Section title="EmptyState" note="One quiet line in the text face and nothing else.">
        <div className="kit-column">
          <EmptyState>Nothing here</EmptyState>
          <EmptyState>No one is waiting</EmptyState>
        </div>
      </Section>

      <Section title="Icon" note="A path, not a set. These are all of them.">
        <div className="kit-icons">
          {Object.entries(icons).map(([name, d]) => (
            <span className="kit-icon" key={name}>
              <Icon d={d} size={20} />
              <span className="kit-quiet">{name.toLowerCase().replace(/_/g, " ")}</span>
            </span>
          ))}
        </div>
      </Section>

      <Section title="Toast" note="The only acknowledgement a triage key gives.">
        <Stage label="sent" size="short">
          <Toast action={{ label: "Undo", keycap: "z", onClick: () => {} }}>
            Sent to Arun Kulkarni
          </Toast>
        </Stage>
        <Stage label="archived" size="short">
          <Toast action={{ label: "Undo", keycap: "z", onClick: () => {} }}>
            Archived 4 threads
          </Toast>
        </Stage>
      </Section>

      <Section title="Sheet" note="Every panel, and on a phone every bottom sheet.">
        <Stage label="with a back trail and a foot">
          <Sheet
            open
            title="Note about Arun"
            onClose={() => {}}
            onBack={() => {}}
            backLabel="the contact card"
            foot={
              <>
                <Button>Cancel</Button>
                <Button variant="primary" keycap="⌘⏎">
                  Save
                </Button>
              </>
            }
          >
            <Field label="Note" value={note} onChange={setNote} multiline rows={3} />
            <Toggle checked={notify} onChange={setNotify} label="Notify me about this sender" />
          </Sheet>
        </Stage>
        <Stage label="a confirmation in front of it">
          <Sheet open title="Screening" onClose={() => {}}>
            <Confirm
              title="Screen out Meridian Properties?"
              body={
                <>
                  <p>
                    Nothing from this address reaches a box again. Nothing is deleted and you can
                    reverse it from their contact card.
                  </p>
                  <label className="confirm-option">
                    <input type="checkbox" defaultChecked />
                    <span>
                      Also move what they have already sent to the bin
                      <span className="confirm-option-note">
                        Left unticked, it stays where it is.
                      </span>
                    </span>
                  </label>
                </>
              }
              confirmLabel="Screen out"
              onConfirm={() => {}}
              onCancel={() => {}}
            />
          </Sheet>
        </Stage>
        <Stage label="a confirmation that is running">
          <Sheet open busy title="Clear the mirror" size="mini" onClose={() => {}}>
            <Confirm
              title="Clear the local copy of pj@73ai.org?"
              body={
                <p>
                  This deletes the mail on this device and syncs the last month again from Gmail.
                  Nothing is removed from Gmail.
                </p>
              }
              confirmLabel="Clear it"
              busy
              busyLabel="Clearing"
              onConfirm={() => {}}
              onCancel={() => {}}
            />
          </Sheet>
        </Stage>
      </Section>

      <Section title="Palette" note="The shell. The data comes from the keymap and the places.">
        <Stage label="open" size="tall">
          <Palette
            open
            query={query}
            onQuery={setQuery}
            groups={PALETTE_GROUPS}
            activeId="later"
            onChoose={() => {}}
            onClose={() => {}}
          />
        </Stage>
      </Section>
    </div>
  );
}
