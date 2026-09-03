# Features

The specification for version one. Each feature says what it does, how it is invoked, the rules it
follows, where its state lives, and what the provider (Gmail for now) sees. "Local" means the
portable state database described in [architecture.md](architecture.md), which roams through the
backup store and never depends on the provider. "Provider" means a change made to the mailbox
itself. The reasoning behind the choices is in [design.md](design.md); the screens are in
[ui.md](ui.md); every key is in [keyboard.md](keyboard.md).

## 1. Places

A place is a view. Numbers reach the seven daily ones, `0` reaches Everything, and the palette
reaches all of them.

| Key | Place | What it holds |
|---|---|---|
| `1` | Inbox | Threads from senders routed to Inbox, plus any reply to a thread you are in |
| `2` | Feed | Threads from senders routed to Feed, rendered open |
| `3` | Paper Trail | Threads from senders routed to Paper Trail |
| `4` | Reply later | The Reply later pile |
| `5` | Set aside | The Set aside pile |
| `6` | Screener | First messages from senders with no decision yet |
| `7` | Snoozed | Threads waiting to return, with their return time |
| `0` | Everything | Every thread in the account, including archived, in date order |
| palette | Sent, Drafts, Starred, Screened out, Spam, Trash | The usual folders |
| palette | All files, Clips, Contacts | The libraries |
| palette | Labels | The provider's labels or folders, one place each |

Every place except Feed, Screener and Focus & Reply is a list column beside the reading pane.
Feed and Screener take the whole stage because their content is inline. A place remembers its
scroll position and selection while the app is open.

## 2. Routing and the Screener

### Destinations

Every sender has exactly one destination: Inbox, Feed, Paper Trail, or Screened out. The rule is
keyed on the sender's address, or on the domain when the user chose "everyone at this domain".
Address rules beat domain rules. Consumer domains (gmail.com, outlook.com, yahoo.com, icloud.com,
proton.me, hey.com and a maintained list) cannot carry a domain rule.

Two overrides apply before the sender rule:

- A message whose `In-Reply-To` or `References` points at a thread the account is already in goes
  where that thread is, or to the Inbox if the thread was screened. A reply is never held.
- A message from an address in the account's contacts, or one the account has ever sent to, is
  screened in on first run and routed by the suggestion rules, never held.

### The Screener

A message from a sender with no rule is held: it is not shown in any box, and the Inbox shows a
pill "Screen N new senders". The Screener place lists one card per sender: avatar, name, address,
subject, snippet, and a one-line reason with the suggested destination. Keys:

- `y` accepts the suggestion and sets the rule for the address.
- `v` opens the destination picker: Inbox, Feed, Paper Trail, and a toggle for "everyone at
  this domain".
- `n` screens the sender out. Nothing is sent. Their mail is routed to Screened out from then on.
- `Enter` expands the card to show the whole message, with a Reply button that screens the
  sender into the Inbox and opens a reply.
- Clear all screens out every sender currently waiting, after a confirmation.

Screened out mail is kept for 90 days in the Screened out place, then trashed. Reversing a
decision is done from the sender's contact card, and re-screening someone in brings back whatever
they sent in the last 90 days.

### Suggestions

The suggestion is a deterministic function of the first message, and the reason shown is the rule
that fired. In order:

1. Written by a person: no `List-Id`, no `List-Unsubscribe`, no `Precedence: bulk` or `list`, no
   `Auto-Submitted`, a `From` local part that is not `noreply`, `no-reply`, `donotreply`,
   `notifications`, `mailer`, `bounce` or similar. Suggest Inbox.
2. Carries `List-Unsubscribe`, `List-Id` or `Precedence: bulk`, or Gmail put it in
   `CATEGORY_PROMOTIONS`. Suggest Feed.
3. Sent by a service for a person, or transactional: a no-reply local part, Gmail's
   `CATEGORY_UPDATES`, or a subject matching receipt, order, confirmation, invoice, shipped,
   payment, verify, code, ticket, itinerary, reservation. Suggest Paper Trail.
4. Anything else: Inbox.

Rule 3 wins over rule 2 when both match, because a receipt with an unsubscribe footer is still a
receipt. The rules are a table in the source and the reason strings are the table's rows, so a
wrong suggestion is a one-line fix.

### First run

When an account is added, every sender in the mirror is screened in with a rule set by the same
suggestion function, silently. The user can move any sender from the contact card, and the move
applies to that sender's existing threads immediately. Only senders whose first message arrives
after the account was added are held in the Screener.

Alongside this, a first-run panel offers "Start fresh": mark everything older than a chosen age
(default one week) as seen, so New for you holds only what is recent. This is the only bulk
write to the provider the app ever proposes, it is optional, and it is reversible for seven days
from the palette.

State: sender rules are local, keyed on address or domain. Provider: nothing. A held or
screened-out message keeps whatever labels Gmail gave it.

## 3. The Inbox

Two groups, fixed order, newest first within each: New for you (threads with at least one message
the account has not seen) and Previously seen (everything else routed to Inbox, including sent
threads). A third group, Back, appears above New for you when a snoozed thread has returned, and
holds it until it is opened.

- Opening a thread marks it seen. `u` toggles seen on the selected thread. Mark all as seen is a
  link on the New for you heading and a palette command.
- A new message in a Previously seen thread moves the thread to New for you.
- `e` archives: the thread leaves the Inbox and lives in Everything. A new message in an archived
  thread brings it back to New for you. Archive is a provider change (Gmail: remove `INBOX`).
- There are no counts on the groups, on the place, or on the app icon. A dock badge for New for
  you exists as a setting and is off.
- A note on a thread shows as a single line under its row.

Seen state is the provider's read state (Gmail: `UNREAD`), so it is not app state. Everything
else about the Inbox is a view over the mirror plus the sender rules.

## 4. The Feed

The Feed renders every thread routed to it as an open card: brand avatar, sender and address,
time, the subject as a title, and the message body. A card longer than one screen truncates with
a fade and "Read more"; `Enter` on the focused card expands it in place, `Enter` again collapses.
A hairline reading "You left off here" marks the newest card that was on screen at the end of the
last visit.

- No read state, no counts, no New for you. Time is the only order.
- Card actions: Read more, Save clip, Unsubscribe, Move (to Inbox or Paper Trail, for this
  sender), Set aside, Reply later, Archive, Trash. All with keys.
- `j` and `k` move between cards and scroll the focused card into view.
- Remote images obey the same rules as the reading pane. Trackers are stripped and the top of the
  Feed says how many today.
- Feed threads older than a configurable age (default never; HEY defaults to 90 days) can be
  trashed automatically per sender from the contact card.

## 5. The Paper Trail

A flat list, newest first, of threads routed to it. No read state. A sender with more than one
thread in the last seven days is bundled into one row showing the count and the latest subject;
`Enter` expands the bundle in place, and bundling can be turned off per sender from the contact
card and off entirely in settings. The reading pane works as in the Inbox. Verbs are the same as
the Inbox minus the seen toggle, plus Move to Inbox.

## 6. The piles

### Reply later

`l` on a thread, or on a selection, moves it out of its list into the Reply later pile. The pile
is a stack of cards at the foot of the list column showing the top thread's subject and sender;
`4` or a click opens the Reply later place, which lists the pile with the reading pane. `l` again
returns the thread to where it came from. Sending a reply on a Reply later thread clears it from
the pile with an undo toast.

### Set aside

`s` does the same into the Set aside pile at the bottom right. `5` opens the place. `s` again
returns the thread. Set aside is for reference, so a thread can sit there indefinitely; nothing
nags.

### Focus & Reply

`Shift+F`, the palette, or the button in the Reply later place. A page listing every Reply later
thread, each with its latest message on the left and a reply box on the right. `Tab` moves to the
next item, `Cmd+Enter` sends and collapses the item to a "Sent to" line, `Esc` leaves the page.
Items you skip stay in the pile.

State: pile membership and order are local, keyed on the thread key. Provider: nothing. A piled
thread keeps its Gmail labels; it simply does not render in the Inbox list.

## 7. Snooze and reminders

`b` opens the snooze picker on a thread or selection: Later today (in three hours), Tomorrow
(8:00), This weekend (Saturday 9:00), Next week (Monday 8:00), Pick a date and time, and If no
reply by (a date; default tomorrow). The thread leaves its list and appears in the Snoozed place
with its return time.

When a device opens the app, comes to the foreground, or wakes from sleep, it evaluates every
snooze whose time has passed and returns those threads to the Back group at the top of the Inbox
(or the top of the Feed or Paper Trail if that is where they live). If no reply by returns the
thread only if nobody but the account has written to it since; a reply cancels the reminder and
the reply lands as normal. A returned thread stays in Back until opened.

Nothing runs in the background and nothing fires at an exact time. The Snoozed place shows the
return time so the user can see what is pending, and a thread that returns late says "Due
yesterday" rather than pretending.

In compose and inline reply, Remind me if no reply is a toggle in the footer with the same date
picker. It applies to the sent thread after the send completes.

State: snooze entries are local (thread key, return time, kind). Provider: nothing.

## 8. Reading

The reading pane shows the selected thread. Subject in the text face, participants and message
count beneath, then messages separated by hairlines. Older messages collapse to a one-line
preview; the latest is open. `n` and `p` move between messages, `o` expands or collapses the
focused message, `Shift+O` expands all. Quoted text is collapsed behind a pill.

- Message bodies render in a sandboxed webview with scripts, forms and external styles removed.
  Remote images are blocked by default; a banner says how many trackers were stripped and names
  the vendor; Show images loads them for this message, and the contact card can allow them for a
  sender always. Attachments are chips; images and PDFs preview inline on demand.
- Attachments are fetched when the thread is opened, not during sync, and cached.
- A calendar invite (`text/calendar` with `METHOD:REQUEST`) renders as a card: date, title,
  time, location, organiser, and Accept (`y`), Maybe (`m`), Decline (`n`), plus Open in Margin
  Calendar. RSVP goes through the Calendar API on the invited calendar; when the event is not
  there yet it is imported first. Without the Calendar scope the card still renders read-only.
- Links show their real destination on hover and open with known tracking parameters removed.
- Read together: select several threads with `x` and press `Enter`; the pane shows them one
  after another with a heading each.
- The pane can be hidden (`Cmd+\`); the list then takes the width and `Enter` opens a thread in
  place, HEY style, with `Esc` returning to the list.

## 9. Writing

### Reply and new mail

`r` replies to the sender, `a` replies to all, `f` forwards; each opens a box under the last
message with the recipients shown as chips and reply-all as a one-key switch. `c` opens the
compose card floating over the list, bottom right; the expand button or `Cmd+Shift+P` makes it
the whole window. Drafts save locally as you type and to the provider every few seconds, so a
draft roams the way Gmail drafts always have.

The editor is TipTap, as in margin: paragraphs, bold, italic, links, lists, quotes, code. No
colours, no fonts. Plain-text mail is sent as plain text. The signature comes from the provider's
settings for the sending address and is editable in settings.

### Sending

`Cmd+Enter` sends. Every send is held for ten seconds (five, twenty or thirty in settings) with
a toast "Sent to X · Undo"; `z` or the toast cancels and reopens the draft. `Cmd+Shift+Enter`
sends immediately. Sends that fail or happen offline wait in the outbox and retry, and the thread
shows a "Waiting to send" line until they go. Threading headers (`In-Reply-To`, `References`,
the provider's thread id, a matching subject) are always set so replies land in the thread on
both ends.

### Remind me if no reply

A toggle in the send footer with a date. See section 7.

### Instant intro

`Cmd+Shift+I` in a reply to an introduction moves the introducer to Bcc and inserts a thank-you
line from a template that can be edited in settings. Pressing it again reverts.

### Attachments

Drag and drop, paste, or `Cmd+Shift+A`. The composer refuses anything that would push the
encoded message over the provider's limit (Gmail: 35 MB total) and says so before you try to
send.

### Not in this version

Snippets, send later, and any AI assistance. The compose footer leaves room for them.

## 10. The things HEY owns, kept beside the mail

### Notes

`y` on a thread adds a private note. In the pane it is a block after the message that was latest
when it was written, dated; in the list it is one line under the row. Text only in this version.
Local, keyed on the thread key.

### Rename

Click the subject or run Rename from the palette. The pane shows the new name with "renamed ·
was …" beside it, the list shows the new name, and replies still carry the real subject so
threading holds on both ends. Local.

### Merge

Select two or more threads and press `g`. They become one thread in every list and in the pane,
named after the longest one or a name you type. A banner on the merged thread says where it came
from and offers Unmerge. Replies and new messages in any underlying thread appear in the merged
one. Local: a mapping from the underlying thread keys to a merged key.

### Clips

Select text in any message and the Save clip button appears; `Cmd+Shift+C` also works. The Clips
place lists every clip with its sender, thread and date, and each links back. Local.

### All files

The All files place lists every attachment in the mirror as a card with name, type, size, sender
and thread, newest first, with filters by type (images, PDFs, documents, spreadsheets,
presentations, calendar invites, archives, other) and by sender. Signature junk (images under
10 KB referenced inline) is excluded. Opening a card opens the thread with the attachment
focused. Built from the local index; nothing is fetched until you open one.

### Ignore

`m` on a thread. New messages still arrive and append, but the thread never returns to New for
you and never notifies. A banner on the thread says "You are ignoring this thread" with Stop
ignoring. Local.

### Notifications

Off by default everywhere. `Shift+N` on a thread turns them on for that thread; the contact card
turns them on for a person. A notification shows the sender and subject, and opening it opens the
thread. There is no badge unless the setting is turned on. Local.

## 11. Contacts and the contact card

`i` on a thread, or clicking a name or avatar anywhere, opens the contact card as a popover:
avatar, name, address, then Delivers to (the destination, changeable), Notify, Screened (in or
out, with the date), Note, Recent threads, Files, and Unsubscribe when the sender's mail carries
`List-Unsubscribe`. The Contacts place lists every sender with a rule and lets you search them.

Autocomplete in compose draws first on the addresses in the mirror (everyone you have written to
or received from, ranked by recency and frequency) and second on the provider's contacts
(Google: People API `otherContacts` and `connections`).

State: notes, delivery, notify are local. Provider: nothing.

## 12. Unsubscribe, block, spam, trash

- Unsubscribe (`Cmd+U`, the Feed card, or the contact card): with an RFC 8058 one-click header
  the app POSTs and confirms; with a `mailto:` header it sends the message; otherwise it opens the
  link. Either way it offers "and trash everything from them" and "and screen them out".
- Screen out from the contact card is the block: future mail goes to Screened out. Nothing is sent.
- `!` marks spam (provider), `#` trashes (provider), both with undo. Trash empties after 30 days
  on the provider's schedule; the Trash place has an Empty button.

## 13. Selection and bulk actions

`x` selects the focused row, `Shift+J` and `Shift+K` extend, `Cmd+A` selects all from here,
`Esc` clears. While a selection exists the piles are replaced by an action bar with the same verbs
and keys: Reply later, Set aside, Snooze, Mark seen, Archive, Move, Merge, Ignore, Trash, and
Enter for Read together. Every bulk action is one undo.

## 14. Search

`/` focuses search. Results replace the list column and the reading pane works as usual. Search
is local over the full mirror: subject, participants, snippet and body text, with operators
`from:`, `to:`, `subject:`, `has:attachment`, `filename:`, `in:` (any place), `before:` and
`after:`, `label:`. When the query touches mail that is not yet hydrated, the provider's search
runs as a second pass and its results append with a note. Results open in place and `Esc`
returns to the previous place.

## 15. Accounts

Add as many Gmail accounts as you like. Each has its own places, sender rules, piles and
Screener. The account chip in the title bar switches (`Ctrl+1` to `Ctrl+9`) and offers All
accounts (`Ctrl+0`), which merges every account's version of the current place into one list with
a coloured edge on each row. Compose picks the account from the thread you are replying to, or
the account you are looking at, and the From field switches it. Sent mail goes through the
sending account; an alias verified on the provider can be chosen in From.

## 16. Labels

The provider's labels or folders are places, listed under Labels in the palette. `Shift+L`
applies or removes one on the selected threads, `v` moves (applies a label and archives). They
are the provider's, they roam with the mailbox, and they are not how Margin organises anything.

## 17. Settings and export

`Cmd+,` opens settings as a panel: Accounts (add, remove, signature, aliases), Backup (Google
Drive or Cloudflare R2, and the recovery phrase), Appearance (theme, reading pane, row density
for the phone), Sending (undo delay, reply-all default, instant intro text), Privacy (remote
images, link cleaning, per-sender allowances), Notifications (badge, sound), Keyboard (the keymap
file), Data (export mail as mbox per account, export app state as JSON, import app state).

## 18. Not in version one

Send later. Snippets. Any AI. Shared threads, team comments, read statuses. Workflows, collections,
cover art. A calendar sidebar. Unified search across accounts (search is per account until the
unified view is proven). IMAP and JMAP providers (the trait is there; the implementations come
after Gmail is solid).
