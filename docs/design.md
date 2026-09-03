# Design

A mail client for Gmail, macOS first and then the iPhone, that borrows the two ideas worth
borrowing from HEY and the two worth borrowing from Superhuman and refuses the rest of both.
It exists because the email experience should belong to the person reading it, not to whoever
stores the bytes, and because nobody has built a native, quiet, keyboard-driven Gmail client with
sender screening. The research behind every claim here is in [research/](research/), and the
screens are in [ui.md](ui.md).

## What we took and what we refused

From HEY: consent and separation. Nobody reaches your Inbox until you have said yes to them once,
and mail from people is kept apart from newsletters and receipts by a decision you made, not by a
classifier guessing. Reply Later and Set Aside are physical piles rather than flags. Read mail
sinks rather than nagging. Notifications are off until you ask for one. Trackers are stripped and
named. We refused HEY's layout (one column, a page per thread, a round trip for everything), its
insistence that routing is per sender only, and its refusal to let you archive.

From Superhuman: speed and the keyboard. One key per verb, the key printed on every button, and a
command palette that is the whole settings and discovery surface, so the app teaches itself. A
list beside a reading pane, so you triage without leaving the list. Remind me if no reply, undo
send, the contact card. We refused the tracking pixels, the AI surface that ships your mail to a
vendor, the inbox-zero streak, the tiny fixed type, and the price.

From neither: the app is the only place your state lives, and that state is yours. Your piles,
screening decisions, notes, renames and clips are keyed on the mail itself (the RFC Message-ID
and the sender's address), not on Gmail's identifiers, and they are kept in a local database that
backs up to a store you choose. Move to Fastmail or Proton next year and every decision comes
with you. Gmail is storage and transport. It never sees a HEY-style label, a note, or a rule.

## The four places

There are three boxes and one gate, and every sender has exactly one destination among them.

**Inbox** is for people and for the few services you want to hear from as they arrive. It is a
stream, not a queue: what you have not looked at sits under New for you at the top, and
everything you have opened or sent sinks to Previously seen beneath it. Nothing counts anything.
A reply pulls a thread back up. There is an archive key, because some people need an empty list
to feel finished, but nothing in the design pushes you towards it.

**Feed** is for newsletters and long reads. Every item is already open, in one scrolling column,
newest first, with a marker where you left off. There is no read state and no obligation. You
scroll it when you want to read, and it never tells you how far behind you are.

**Paper Trail** is for receipts, confirmations and notifications: things you file and later
search for. A flat list, no read state, senders who flood you bundled into one row.

**The Screener** holds the first message from anyone new until you decide. The card shows who they
are, what they sent, and what Margin thinks the right box is and why: written by a person, so
Inbox; carries an unsubscribe header, so Feed; sent by a service on someone's behalf, so Paper
Trail. One key accepts the suggestion, one picks elsewhere, one says no. No is silent. The
decision can be per address or per domain, which is the thing HEY's users ask for most and HEY
will not give them. Replies to a thread you are already in bypass the whole system and land in
the Inbox, because the sender rule is about first contact, not about conversations.

On first run there is no Screener avalanche. Everyone who has ever written to you is screened in,
routed by the same suggestion rules, and movable later from their contact card. Only genuinely
new senders from that point on are held.

## The two piles

Reply Later and Set Aside sit at the foot of the list as two stacks of cards, always visible,
each showing what is on top. A thread you pile leaves the list. Reply Later is for what you owe;
Set Aside is for what you need to hand (a ticket, an itinerary, a code). Focus & Reply lines up
everything in Reply Later on one page with a reply box beside each, and sending a reply clears
the thread from the pile.

Snooze covers the rest: later today, tomorrow, the weekend, a date, or if no reply by a date. A
snoozed thread leaves the list and comes back to the top of the Inbox when it is due. There is no
scheduler and no server. Whichever of your devices next opens the app checks what is due and
brings it back. That is honest about what a client without a server can do, and it is enough.

## Keyboard first, mouse whole

Every verb is one unmodified key, the same key Gmail and Superhuman use where they agree
(`j`, `k`, `e`, `r`, `a`, `f`, `c`, `/`, `x`, `u`, `z`) and HEY's letters for HEY's verbs (`l`
Reply Later, `s` Set Aside, `b` snooze, `y` note, `m` ignore). Number keys go to places. There
are no two-key chords, nothing is modal, and the key is printed on every button so the mouse
teaches the keyboard. Cmd-K opens the palette, which also lists every place, every command and
every setting. The full map and the reasoning for each conflict are in [keyboard.md](keyboard.md).

## Quiet by design

No badge, no unread count, no streak, no photograph when the list is empty. No notification
unless you turned it on for that thread or that person. Remote images do not load until you ask,
tracking pixels are removed before the message renders, and the banner tells you whose pixel it
was. Nothing you send carries a tracker and nothing reports when it was opened. Links open with
their tracking parameters removed.

There is no AI in this version. Classification is by headers and by your decisions, the way HEY
does it, and it is explainable in a sentence on every Screener card. The design leaves a seat for
a local model later; it does not leave one for a cloud one.

## Reading, writing, and the things HEY owns

Reading happens in a pane beside the list. Long threads collapse quoted text and older messages;
`n` and `p` step through messages. A calendar invite renders as a card with accept, maybe and
decline, and a link that opens the event in Margin Calendar. A sender's name opens their contact
card: where their mail delivers, whether they notify you, your note about them, recent threads,
files.

Replies are written under the last message. New mail is a card floating over the list that can
expand to the whole window. Every send waits ten seconds with an undo, then leaves. Remind me if
no reply is a toggle on the send button.

Notes, renames, merges and clips are the features HEY can offer only because it owns the data
model. We can offer them because we keep our own, beside the mail rather than inside it. A note
is a private block in the thread and a line under its row. A rename changes the subject for you
alone and says so. A merge shows several threads as one and can be undone. A clip is a saved
passage with a link back. All of it roams through the backup store, none of it touches Gmail.

## What it is not

Not a team tool: no shared threads, comments, or read statuses. Not a calendar: invites hand off
to Margin Calendar. Not a scheduler: no send later in this version. Not an assistant: nothing is
summarised or drafted for you. Not a Gmail skin: the Gmail web UI is not a consideration, since
the whole point is never opening it.

## Visual language

Lifted from margin and the calendar unchanged: warm paper, ink and two softer inks, hairline
borders, a four-step type scale, three radii, one easing curve, light and dark driven by
`data-theme`. Hanken Grotesk for the interface, Literata for the subject line and for message
bodies, because mail is reading and reading deserves a text face. The additions are mail-specific:
a row hover and a row selection wash, a warmer band for Previously seen, a note surface, a dot
colour for new mail, and the eight muted hues the calendar already uses, here for avatars and
account edges. No CSS framework, no component library, hand-written CSS on tokens.

Rows are two lines: sender and time, then subject and snippet. New mail gets a dot and a heavier
sender; nothing else is bold. No chips, no labels in the row, no icons appearing on hover. The
reading pane has one bar of verbs with their keys, a serif subject, and messages separated by
hairlines rather than cards.

## On a phone

The list is the screen, the piles sit above a tab bar with the three boxes and a write button, and
a thread is a page with a floating action bar where the thumb is. Swipes are Reply Later to the
right and Set Aside to the left, both changeable. Nothing that only appears on hover exists on a
phone; there is no hover. Notifications remain opt-in per thread and per person, and there is no
badge. The same premise, the same four places, the same two piles.

## Licence and ownership

FSL-1.1-MIT, as margin is: use it for anything except a competing product, and each version turns
MIT two years after release. Ownership is a feature, not a slogan: mail exports as mbox, contacts
as vCard, and every piece of app state as JSON, from settings, at any time.
