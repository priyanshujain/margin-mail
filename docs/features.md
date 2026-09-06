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
| `0` | Everything | Every thread on the device, including archived, spam and screened out, in date order |
| palette | Sent, Drafts, Starred | The usual folders |
| palette | Screened out, Spam, Trash | Under Other: the places you go looking in rather than read |
| palette | All files, Clips, Contacts | The libraries |
| palette | Labels | The provider's labels or folders, one place each |

Every place except Feed, Screener and Focus & Reply is a list column beside the reading pane.
Feed and Screener take the whole stage because their content is inline. A place remembers its
scroll position and selection while the app is open.

Every place is a view over what is on the device, and what is on the device is a window of the
mailbox: the last 30 days by default, or 90, 180, 365 days or everything, set per account. Threads
you have done something to are kept whatever their age. Only Everything says any of this out loud,
in one quiet line at the foot of the list, "Showing the last month. Older mail is on Gmail.", with
the setting one click away. The mechanism is in [architecture.md](architecture.md) and the setting
is in [settings.md](settings.md).

## 2. Routing and the Screener

### Destinations

Every sender has exactly one destination: Inbox, Feed, Paper Trail, or Screened out. The rule is
keyed on the sender's address, or on the domain when the user chose "everyone at this domain".
Address rules beat domain rules. Consumer domains (gmail.com, outlook.com, yahoo.com, icloud.com,
proton.me, hey.com and a maintained list) cannot carry a domain rule.

Two overrides apply before the sender rule:

- A message whose `In-Reply-To` or `References` points at a thread the account is already in goes
  where that thread is, or to the Inbox if the thread was screened. A reply is never held.
- A message from someone the account already knows is screened in on first run and routed by the
  suggestion rules, never held. Who counts as known is settled under First run below.

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

Screened out mail sits in the Screened out place for as long as the storage window keeps it and
falls off the device with everything else of that age. There is no second retention rule to
remember, and a wider window means a longer memory. Reversing a decision is done from the sender's
contact card, and re-screening someone in brings back whatever they sent that is still here.

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

When an account is added, everyone it already knows is screened in with a rule set by the same
suggestion function, silently. A month of mail is not by itself a good answer to who a person
knows, so the seed is drawn from three cheap sources at once: every sender and every recipient
inside the storage window, the People API's `connections` and `otherContacts` lists, and the Sent
mail inside the window. Anyone in any of the three is screened in. All three are already fetched
or already on the way, so this costs a pair of extra calls and no waiting.

The pass runs at the end of the first sync rather than when the first-run panel is dismissed, so
the Inbox fills as the mail arrives instead of sitting empty for as long as somebody takes to read
a panel. It runs once per account and is guarded, which is the whole of what makes the Screener a
gate: if it ran again on a later sync it would screen in every new sender the moment they wrote.
The guard cuts the other way too: asked before the crawl has finished, as it is the moment an
account is added from Settings, the seed answers "not yet" rather than marking itself done over a
mirror with nothing in it, and the sync's own idle asks again. A seed on record as having
screened in nobody is run once more when the mirror is ready, which repairs an account that was
marked that way before the rule existed without anybody removing it and connecting it again.

The user can move any sender from the contact card, and the move applies to that sender's existing
threads immediately. Only senders whose first message arrives after the account was added are held
in the Screener.

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
- There are no counts on the groups or on the place. The one count anywhere is the dock badge,
  which is the size of New for you across every account, and it is on by default and turns off in
  Notifications. It is not the mailbox's unread count: a thread held in the Screener, routed to the
  Feed or the Paper Trail, piled, snoozed or ignored is unread and is not waiting for you. Zero
  takes the badge off rather than showing a nought. macOS and Linux carry it; Windows would need a
  drawn overlay icon and does not have one yet.
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
  sender always. Attachments are chips; pressing one opens the file with whatever owns its type.
- Attachments are fetched when the thread is opened, not during sync, and cached.
- A calendar invite (`text/calendar` with `METHOD:REQUEST`) renders as a card: date, title,
  time, location, organiser, and Accept (`y`), Maybe (`m`), Decline (`n`), plus Open in Margin
  Calendar. RSVP goes through the Calendar API on the invited calendar; when the event is not
  there yet it is imported first. The Calendar permission is not asked for when the account is
  added, so the first RSVP says it needs it and runs the consent page again; until then the card
  renders read-only.
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

`m` on a thread. New messages still arrive and append, and the thread rises with them because the
Inbox is in time order, but it never reads as new, never counts on the badge and never notifies. A
banner on the thread says "You are ignoring this thread" with Stop ignoring. Local.

### Notifications

Off by default everywhere. `Shift+N` on a thread turns them on for that thread; the contact card
turns them on for a person; Settings turns them on for a place, and has one switch over all of it
for the machine, which is what "nothing on this laptop" means without touching a single thread. The
sync pass that brings a message in is what posts the notification, and only for mail that arrived
after the app came up, so a first sync, a rebuild or a week away says nothing about the backlog; one
message is three lines, the app's name, the sender and the subject, and several in one pass are one
notification counting them and naming the senders. On macOS the system asks once whether the app
may notify at all, the first time anything here is turned on or the test button is pressed, and a
refusal is undone in System Settings rather than here. Clicking one brings the app to the front and
opens the thread in the list it shows in; a click on the grouped one opens the account's Inbox, and
a click on the sample from Settings only brings the app to the front. The dock badge is a setting
in the same section rather than a notification, it counts New for you, and it is the one thing here
that is on. Local.


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
- `!` marks spam and `#` trashes, both provider changes, both with an undo toast.

### Screened out, Spam and Trash

Three places under Other in the palette, below the daily ones and below the labels, with no number
keys of their own. Where a place sits is the honest statement of how often you should be in it, and
these are the ones you go looking in rather than the ones you read.

None of them is a filter of ours. Gmail's spam filter runs on Gmail's side before the app sees a
message, and the mirror takes what the mailbox holds: every list call sets `includeSpamTrash`, so a
junked message is already on the device with its body indexed whether or not anything shows it.
Screened out is the one we own, and it is a routing destination rather than a folder, so its rules
are in section 2.

Getting mail back out is the point of all three, and the verb that puts it back is the verb that
put it there, the way the piles already work. `#` in Trash puts a thread back, `!` in Spam takes
the spam mark off. Gmail restores a message's labels when the `TRASH` label comes off, so a thread
put back lands where it was rather than in the Inbox. A thread taken out of Spam is routed like any
other: to its sender's box if that sender has a rule, and to the Screener if they do not, which is
the decision you still owe them.

There are three ways back, in the order you will want them. A wrong keystroke is the toast that is
already up, "Trashed · Undo" and `z`. A rescue a week later is the place and the verb. After thirty
days Gmail empties its own trash, the message leaves the mirror with it, and nothing local changes
that.

There is no Empty button. Permanently deleting through the Gmail API needs the
`https://mail.google.com/` scope, which is total access to the mailbox, and asking every account
for that so a button can destroy things thirty days earlier than Gmail will anyway is a bad trade.
Each place states the rule instead, in one line at the foot of the list: "Gmail empties this after
30 days." Screened out carries no such line, because it falls off with the storage window like
everything else of its age and there is no second retention rule to learn.

## 13. Selection and bulk actions

`x` selects the focused row, `Shift+J` and `Shift+K` extend, `Cmd+A` selects all from here,
`Esc` clears. While a selection exists the piles are replaced by an action bar with the same verbs
and keys: Reply later, Set aside, Snooze, Mark seen, Archive, Move, Merge, Ignore, Trash, and
Enter for Read together. Every bulk action is one undo.

## 14. Search

`/` focuses search. Results replace the list column and the reading pane works as usual. Search
is local over what is on the device: subject, participants, snippet and body text, with operators
`from:`, `to:`, `subject:`, `has:attachment`, `filename:`, `in:` (any place), `before:` and
`after:`, `label:`.

Because the device holds a window, a local result set is a partial answer and says so. Every list
of results ends with "Search older mail on Gmail", which runs the provider's search, appends the
hits and hydrates them as they arrive. Those rows behave like any other row, and the next eviction
pass takes them away again unless they picked up a pile, a note or some other decision in the
meantime. A query whose `before:` or `after:` falls outside the window skips the local index
entirely and goes to the provider, because a local answer to that question would be wrong rather
than merely short. Results open in place and `Esc` returns to the previous place.

Search reaches Spam, Trash and Screened out, and names the place on the row when it does. The
message you most need to find is the one something else decided you should not see, and a search
that skipped those three would be one you had to already know the answer to use. `in:` narrows to
a single place when that is what you meant.

## 15. Accounts

Add as many Gmail accounts as you like. Each has its own places, sender rules, piles, Screener,
storage window and granted permissions, so a work account can keep a year while a personal one
keeps a month. The account chip in the title bar switches (`Ctrl+1` to `Ctrl+9`) and offers All
accounts (`Ctrl+0`), which merges every account's version of the current place into one list with
a coloured edge on each row. Compose picks the account from the thread you are replying to, or
the account you are looking at, and the From field switches it. Sent mail goes through the
sending account; an alias verified on the provider can be chosen in From.

## 16. Labels

The provider's labels or folders are places, listed under Labels in the palette. `Shift+L`
applies or removes one on the selected threads, `v` moves (applies a label and archives). They
are the provider's, they roam with the mailbox, and they are not how Margin organises anything.

## 17. Settings and export

Settings is a place, not a panel. `Cmd+,`, the palette, the account chip and the app menu all lead
to the same full-stage screen, with a rail of sections down the left and one section at a time on
the right. Twelve sections cover the accounts and their permissions, appearance, the storage
window, privacy, the Screener, the piles and snooze, writing, notifications, the keymap, backup and
the recovery phrase, every export the app offers, and the version. Each is specified in
[settings.md](settings.md), including which of them live on the device and which roam.

## 18. Help

The app is unlike the mail clients people arrive from, and none of the differences are
discoverable by poking at the interface, so there is a tour, a question mark in the corner, and a
guide behind it. Each is specified in [help.md](help.md).

The tour is nine slides in a sheet, over the Inbox, run once for every account that is added and
skipped with one key. It follows the first-run panel above and names the Screener, the three
boxes, the two piles, snooze, the keyboard, the palette, the things kept beside the mail, what is
off by default, and undo.

The corner button opens the tour again, the guide, and the keyboard shortcuts. It is not drawn
while the compose card is open, while an overlay is up, in the guide itself, or on a phone.

The guide is a panel over the whole window: a search field, a rail of sections, and one article at
a time. How to do each thing the app does, and last, the questions people actually ask. Every
article leads with its answer and shows it in a drawn figure, a screenshot or a table of keys, and
a search nothing answers offers to file the question against the repository. Closing it puts back
the place, the open thread and the scroll. Its pictures come from the dev fixture through
`just guide-shots` and are committed, because they ship in the bundle.

State: which accounts have had their first run, and therefore their tour, is a device fact in
localStorage beside the first-run panel's own flag. Provider: nothing.

## 19. Not in version one

Send later. Snippets. Any AI. Shared threads, team comments, read statuses. Workflows, collections,
cover art. A calendar sidebar. Unified search across accounts (search is per account until the
unified view is proven). IMAP and JMAP providers (the trait is there; the implementations come
after Gmail is solid).
