# Architecture

Tauri 2, React 19, Vite, TypeScript and zustand on the front, Rust behind. The same stack as
margin and Margin Calendar, so OAuth, token sealing, the build and bundle setup, the overlay
title bar and the phone chrome carry over rather than being invented again. The conventions are
the calendar's, in `../margin-caledar/docs/conventions.md`, and apply here unchanged.

The split is strict. Rust owns authentication, every byte to and from a mail provider, the local
mirror, the sync loop, MIME parsing, HTML sanitising, the portable state database, the journal
and the backup store. TypeScript owns rendering and interaction. The frontend never talks to a
provider, which keeps the content security policy locked to `ipc:` as in the siblings.

The facts about the Gmail API that this design rests on are in
[research/gmail-api.md](research/gmail-api.md), checked against Google's pages on 3 September
2026.

## Two databases, one boundary

There are two SQLite databases per account and the boundary between them is the product.

The **mirror** is a copy of the mailbox: messages, threads, labels, bodies, attachment metadata,
an FTS5 index, the provider's sync cursor, and an outbox. It is derived from the provider and can
be thrown away and rebuilt. Its keys are the provider's ids.

The **state** database is everything the user decided: sender rules, pile membership, snoozes,
notes, renames, merges, clips, ignore flags, notification opt-ins, contact notes. It is never
derived, it is never sent to the provider, and its keys are portable: a thread is identified by
the RFC `Message-ID` of its earliest message (the thread key), a message by its own `Message-ID`,
a sender by their address or domain. Provider ids appear in the state database nowhere. When the
user moves to another provider and the same mail arrives through IMAP with the same
`Message-ID`s, every decision reattaches.

A view is the mirror joined to the state. The Inbox is "threads whose sender rule says Inbox, or
that carry a reply to a thread we are in, minus piles, minus snoozes, minus archived", grouped by
seen state. The join is computed in Rust and served to the frontend as a flat, ordered list of
thread summaries; the frontend never sees a label id or a rule.

## The provider trait

```
trait Provider {
    fn authenticate(...)                 // interactive, returns a sealed credential
    fn full_list(...)                    // ids and thread ids, newest first, paged
    fn changes_since(cursor)             // the change log, or NeedsFullSync
    fn fetch_headers(ids)                // batched metadata
    fn fetch_body(id)                    // raw RFC 2822 bytes
    fn fetch_attachment(id, part)        // bytes
    fn set_flags(ids, seen, starred, archived, trashed, spam)
    fn labels() / apply_label / remove_label
    fn send(raw, thread_hint)            // returns the provider's id and thread id
    fn drafts(): create / update / delete
    fn search(query, page)               // provider-side search, ids only
    fn settings(): aliases, signature
}
```

Gmail implements it with the REST API. IMAP and SMTP will implement it next with folders mapped
onto the flag set and labels onto `X-GM-LABELS` or keywords, and JMAP after that with its own
change log. The trait is shaped by what every provider can do, and everything the trait cannot do
is done in the state database instead, which is why the state database exists.

Gmail specifics that live only inside the Gmail implementation: `history.list` as the change log
and its 404 recovery, the 6,000 units per minute per user budget, `format=metadata` with a fixed
header list for hydration, `format=raw` for bodies, batch requests of 25 to 50, truncated
exponential backoff on 429, threading rules on send, `CATEGORY_*` labels read as a hint for the
suggestion function, and the People API for contact autocomplete.

## Authentication and distribution

Loopback OAuth with PKCE from the calendar, unchanged: the consent page opens in the system
browser, never in a webview the app owns, and the code lands on `127.0.0.1`. Refresh tokens are
sealed with XChaCha20-Poly1305 in the app data directory as the calendar does, for the same
reasons (no `keyring` on Android, code-signature churn on macOS, no Secret Service on minimal
Linux). Scopes: `gmail.modify`, `gmail.settings.basic`, `contacts.other.readonly`,
`contacts.readonly`, and `calendar.events` for RSVP. No `mail.google.com` until IMAP is real.

Every Gmail scope that reads mail is restricted. The plan is the one in the research: push the
consent screen to production unverified for the friends release (100 lifetime users, no weekly
re-login), file restricted-scope verification at once with the statement that there is no server
and all Google user data stays on the device, ask in writing whether the security assessment
applies, and budget for it anyway. Bring-your-own OAuth client stays as an escape hatch in
settings for the technical.

## Sync

Polling, no push. `users.watch` needs a Pub/Sub subscription, which needs a server or an IAM
grant no desktop app should hold, and a relay is the thing that might drag the app into an
annual security assessment. `history.list` costs 2 units; polling every 12 seconds in the
foreground and 60 seconds in the background is a rounding error against the budget.

Initial sync is the expensive part after the May 2026 quota change: `messages.get` is 20 units,
so hydration runs at about 300 messages a minute per account. A 20,000 message mailbox takes
about an hour of background work; a 100,000 message mailbox most of a working day. The order is
newest first, the app is usable as soon as the first page lands, and a thin bar in the account
chip says how far back the mirror reaches. Bodies are fetched on open and prefetched for the
last 90 days when idle. Attachments are fetched on open and cached with a size cap. The mirror
is the whole mailbox by decision; a setting caps the age for people who want less on disk.

Every write is optimistic: it lands in the mirror, renders, and is pushed behind. Consecutive
flag changes are coalesced into `batchModify`. Offline writes queue in the outbox and drain on
reconnect, with the thread showing "Waiting to send" until a send goes.

Seen, starred, archived, trashed and spam are provider flags and go through the trait. Nothing in
the piles, the Screener, snoozes or notes ever touches the provider, so a user who screens out
two hundred senders makes zero API calls.

## The state journal and the backup store

The state database is written through an append-only journal: every change is an event with a
device id, a per-device sequence number, a wall-clock timestamp, a kind, a portable key, and a
payload. The tables are a materialised view of the journal. This is what makes roaming possible
without a server and what makes the backup meaningful.

A **backup store** is a trait with three operations: put a blob at a name, get a blob by name,
list names under a prefix. Two implementations ship: Google Drive's app-data folder, which every
Gmail user already has and which margin's backup already uses, and Cloudflare R2 through S3
credentials for people who run their own. Each device uploads its own journal segments under its
device id and downloads every other device's. Merging is last-writer-wins per key by timestamp,
which is correct for every kind of state here (a pile toggle, a note, a rule), and a device that
has been offline for a month simply replays what it missed.

Everything uploaded is encrypted on the device with XChaCha20-Poly1305 under a key that is
generated on first backup, stored sealed like the tokens, and shown once as a recovery phrase.
Drive and R2 hold ciphertext and names; neither can read a note or a rule. The recovery phrase is
the only way to attach a second device or restore after a lost one, and the settings panel says
so in one sentence.

Version one on macOS alone does not need the merge. The journal shape is there from the first
commit so that the iPhone can join without a migration.

## Security

Message bodies are parsed from raw RFC 2822 with a real MIME parser (`mail-parser`), never from
the provider's pre-parsed payload beyond headers, because encoded words, parameter
continuations and legacy charsets appear daily. HTML is sanitised in Rust before it reaches the
webview: scripts, forms, event handlers, `<meta>` refreshes, external stylesheets, `javascript:`
and `data:` navigation are removed; `cid:` references are rewritten to a local resource scheme;
every remote `<img>` is replaced with a placeholder and its source recorded. The body renders in
an iframe with a strict CSP inside the app's webview so the message can never touch the app.

Tracker stripping happens in the same pass: images with a known tracking host (a maintained
list, shipped with the app and updated with it), images of one pixel or hidden by style, and
images whose URL carries a recipient token are removed and counted, and the vendor is named in
the banner. When the user asks to show images, Rust fetches them without cookies or referrer and
serves them from cache; the user's IP is exposed to the image host at that moment and only then,
which the privacy setting says plainly. Outgoing mail never contains a tracker and the app never
requests a read receipt. Links are rewritten on click to drop known tracking parameters, with a
setting to turn that off.

Refresh tokens and the backup key are sealed, never in SQLite. The mirror and the state database
are files in the app data directory and inherit the OS's disk encryption; encrypting them again
would cost search and buy nothing on a device that is already locked, and the export path is
the answer for anyone who wants their mail in a form they control.

## Platforms

macOS: overlay title bar with the traffic lights on the header's centre line, closing the window
hides it and Cmd-Q quits, all from the calendar. iOS second: the same code with the phone chrome
from the calendar's `data-phone` and `data-touch` scheme, overlays as bottom sheets, the OAuth
flow through `ASWebAuthenticationSession`, and background app refresh used only to run the
snooze evaluation and a short sync. Linux afterwards: no traffic lights, closing quits, deb and
AppImage.

## Order of work

The sync engine and the reading pane are the two hard things and neither proves the other, so the
first milestone is one account, authentication, the mirror, and a read-only Inbox with a
sanitised, tracker-stripped reading pane. Everything the app is for depends on those being
right.

Second, triage on the mirror: seen, archive, star, trash, spam, selection, the keyboard, the
palette, local search. At this point it is a fast Gmail client and nothing more.

Third, the state database and the four places: sender rules, the suggestion function, the
Screener, the first-run pass, Feed and Paper Trail. This is the milestone where it stops being a
Gmail client.

Fourth, the piles and their friends: Reply later, Set aside, Focus & Reply, snooze with lazy
evaluation, notes, rename, merge, clips, All files, ignore, per-thread notifications, the contact
card.

Fifth, writing: reply, compose, drafts, the outbox with undo send, attachments, remind me if no
reply, instant intro, calendar RSVP.

Sixth, more than one account, the unified view, settings, export, and the backup store with the
journal behind it.

Then the iPhone, then Linux, then the IMAP provider, in that order.
