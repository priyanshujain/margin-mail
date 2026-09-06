# Architecture

Tauri 2, React 19, Vite, TypeScript and zustand on the front, Rust behind. The same stack as
margin and Margin Calendar, so OAuth, token sealing, the build and bundle setup, the overlay
title bar and the phone chrome carry over rather than being invented again. The house rules for
writing it are in [conventions.md](conventions.md), which is the calendar's file with its
examples pointed here.

The split is strict. Rust owns authentication, every byte to and from a mail provider, the local
mirror, the sync loop, MIME parsing, HTML sanitising, the portable state database, the journal
and the backup store. TypeScript owns rendering and interaction. The frontend never talks to a
provider, which keeps the content security policy locked to `ipc:` as in the siblings.

The facts about the Gmail API that this design rests on are in
[research/gmail-api.md](research/gmail-api.md), checked against Google's pages on 3 September
2026. The milestones, the libraries and the order they land in are in [plan.md](plan.md).

## Two databases, one boundary

There are two SQLite databases per account and the boundary between them is the product.

The **mirror** is a copy of the mailbox: messages, threads, labels, bodies, attachment metadata,
an FTS5 index, the provider's sync cursor, and an outbox. It holds a window of the mailbox rather
than all of it, which the next section explains. It is derived from the provider and can be
thrown away and rebuilt. Its keys are the provider's ids.

The **state** database is everything the user decided: sender rules, pile membership, snoozes,
notes, renames, merges, clips, ignore flags, notification opt-ins, contact notes. It is never
derived, it is never sent to the provider, and its keys are portable. A sender is their address
or their domain. A message is its own `Message-ID`. A thread is the **thread key**: the first
entry of the message's `References` header, or its `In-Reply-To` when there is no `References`,
or its own `Message-ID` when it starts the conversation.

That definition matters more than it looks. A key derived from the earliest message the mirror
happens to hold changes when the window moves, so the same thread would carry two keys on two
devices and lose its pile on the one that had less of it. A key read off the headers of any
message in the thread is the same everywhere: on a phone holding a month, on a laptop holding a
year, and in an IMAP mailbox reached two providers from now. Provider ids appear in the state
database nowhere.

A view is the mirror joined to the state. The Inbox is "threads whose sender rule says Inbox, or
that carry a reply to a thread we are in, minus piles, minus snoozes, minus archived", grouped by
seen state. The join is computed in Rust and served to the frontend as a flat, ordered list of
thread summaries; the frontend never sees a label id or a rule.

## The window

Mail on the device is a window, not the mailbox. Each account carries a `window` setting of 30,
90, 180 or 365 days, or everything, and it is 30 days out of the box. Thirty days is what a
mail client is actually used for, it turns a first sync from an afternoon into a few minutes,
and everything outside it is still one search away on the provider.

The mirror holds every thread whose latest message falls inside the window. On top of that it
holds, at any age: every thread that carries app state (a pile, a snooze, a note, a rename, a
merge, a clip, ignore, notify), every starred thread, every draft, and the outbox. A thread the
user has touched is a thread the user expects to find, and the state database would otherwise
point at rows that are not there.

Eviction runs once a day and again whenever the window shrinks. It removes bodies, attachments
and rows for threads that fall outside, leaving the state database untouched; nothing a person
decided is ever evicted. Widening the window is the mirror image: the newly covered range is
backfilled in the background, newest first, through the same hydration path as the first sync, so
there is one code path for filling the mirror and not two.

The arithmetic of the first sync is the reason for all of it. `messages.list` with an `after:`
term returns the ids for the window cheaply, and metadata hydration runs in batches of 50 at 20
units a message, which is about 300 messages a minute inside the per-user budget. A month of a
busy mailbox is minutes. The whole of it, at the same rate, is hours for twenty thousand messages
and most of a working day for a hundred thousand, which is what the setting exists to let someone
choose deliberately rather than discover.

The window is visible in two places and nowhere else. The Everything place ends with one quiet
line, "Showing the last month. Older mail is on Gmail.", with the setting one click away. Search
results end with "Search older mail on Gmail", which runs the provider's search and hydrates the
hits as transient rows; the next eviction pass removes them again unless they gained state in the
meantime. A query carrying a `before:` or `after:` that lands outside the window skips the local
index and goes straight to the provider, because a local answer would be confidently wrong.

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

Gmail implements it with the REST API. A mailbox reached over IMAP and SMTP with a password is the
second implementation and is what every account that is not Google uses; JMAP is a third if anyone
ever ships it. The trait is shaped by what every provider can do, and everything the trait cannot
do is done in the state database instead, which is why the state database exists.

An account carries a `kind` saying which of the two it is. Almost nothing branches on it: the
places, the piles, the Screener, snoozes, notes and search are all above the trait and cannot tell
the difference. The screens that do differ are the ones about the account itself, because an IMAP
account has no scopes to grant and no Google account page to revoke from, and it has a server and
a port to show that a Google account does not.

IMAP specifics that live only inside that implementation: a per-mailbox cursor of UIDVALIDITY,
UIDNEXT and HIGHESTMODSEQ in place of a change log, CONDSTORE with QRESYNC for the fast path and
a UID scan when the server has neither, mailbox roles read from SPECIAL-USE and XLIST flags before
falling back to names, and folders standing in for labels so a label change is a MOVE. A UIDVALIDITY
change is reported as a full resync rather than remapped, which reuses a recovery path the engine
already had rather than inventing a second one.

Three consequences of IMAP that are worth knowing before they surprise somebody. There is no
server-side snippet, so the preview line on a list row is derived from the body the first time it
is fetched and is empty until then. There is no thread id, so the portable thread key does all the
work; it is computed by the same rule on both sides, so the two agree by construction. And there is
no equivalent of archiving: a mailbox with no `\Archive` special use and no folder named like one
gets an Archive mailbox created on first sync, because a keybinding that means something different
per account is worse than a folder somebody did not ask for.

Sign-in is a password, not OAuth. Both Thunderbird and Mailspring ship their own OAuth client
credentials in the binary for Google and Microsoft, and Mailspring's source says outright that
anyone can extract theirs. Doing OAuth over IMAP would mean registering another client for
`AUTH XOAUTH2`, so a provider that publishes OAuth as its first choice, which Gmail and Fastmail
both do, is offered its own second choice instead: an app password.

Certificates are decided in one place. The loopback is trusted without asking, because a local
bridge listens there with a certificate it generated for itself and nothing sits between this
process and that socket to impersonate anybody. Every other host is verified against the webpki
roots, and a failure becomes a question carrying a fingerprint, remembered per host and port. The
one exception is a certificate naming a different host: that is refused outright and never offered
as a choice, because it is the single failure indistinguishable from an interception.

Gmail specifics that live only inside the Gmail implementation: `history.list` as the change log
and its 404 recovery, the 6,000 units per minute per user budget, `format=metadata` with a fixed
header list for hydration, `format=raw` for bodies, batch requests of 25 to 50, truncated
exponential backoff on 429, threading rules on send, `CATEGORY_*` labels read as a hint for the
suggestion function, and the People API for contact autocomplete.

## One client for the whole suite

Margin, Margin Calendar and Margin Mail share one Google Cloud project, `margin-500217`, and one
OAuth client of type installed. A person who opens the third party access page of their Google
account sees a single entry called "Margin", and revoking it revokes the suite. That is the right
shape: three apps by the same author, on the same machine, holding the same person's data, should
not look like three vendors.

Mechanically it is the calendar's arrangement unchanged. `build.rs` copies
`google-credentials.json` from the repository root into `OUT_DIR`, falling back to
`google-credentials.example.json` when the real file is absent, so a fresh clone compiles and
fails at runtime with a readable "not set up yet" rather than at build time with a missing file.
Loopback OAuth with PKCE, also from the calendar: the consent page opens in the system browser,
never in a webview the app owns, and the code comes back on `127.0.0.1`. Refresh tokens are
sealed with XChaCha20-Poly1305 in the app data directory as the calendar does, for the same
reasons (no `keyring` on Android, code-signature churn on macOS, no Secret Service on minimal
Linux).

Connect asks for `openid email`, `gmail.modify`, `gmail.settings.basic`, `contacts.readonly` and
`contacts.other.readonly`. It does not ask for `calendar.events`, and this is the one place where
Google's rules cost the user something: installed apps get no incremental authorization, so a
scope cannot be added to a live token. The first time somebody answers an invite, the app runs
the whole authorization again with `calendar.events` in the list and replaces the stored token.
Backup does the same with `drive.file` when it is turned on. Both say so before they start.

Granular consent is always on for this client, which means the tick boxes on the consent page are
the user's to clear and the app cannot assume it got what it asked for. The `scope` field of the
token response is stored per account and is the truth. Every feature that needs a scope checks it
first and, when it is missing, renders one sentence and a Grant button that re-runs consent for
the full list. Without `gmail.modify` there is no app at all, so the account is not added and the
screen says which permission was declined and what it was for.

### What the shared client costs

`gmail.modify` and `gmail.settings.basic` are restricted scopes. The shared project must pass
restricted scope verification, and until it does, every Margin app shows the unverified screen and
the three of them share one pool of 100 lifetime users. Verification has to be renewed annually,
and a lapse blocks the whole suite rather than one app. That is the price of the single "Margin"
entry, and it is worth stating in the same breath as the benefit.

The plan is the one in the research: push to production unverified for the friends release, file
restricted scope verification immediately with the statement that there is no server and all
Google user data stays on the device, ask in writing whether the security assessment applies, and
budget for it anyway. Bring your own OAuth client stays in settings as the escape hatch for the
technical.

### The hedge

If verification is refused, or priced at a number that is not worth paying, Gmail over IMAP and
SMTP with a Google app password is the way in. It needs no Cloud project, no verification, and it
has no user cap. Gmail's IMAP extensions carry the pieces the REST API was giving us:
`X-GM-LABELS` for labels, `X-GM-THRID` for the provider's thread id, `X-GM-MSGID` for a stable
per-message id, all of which land in the same mirror columns. IDLE and CONDSTORE replace
`history.list` as the change log. RSVP goes out as an iMIP reply by mail instead of through the
Calendar API. Contacts come from the mirror, which is where autocomplete looks first anyway.

This is not a rewrite, it is the `Provider` trait's second implementation, and knowing that is
half the reason the trait is drawn where it is.

Removing an account therefore reaches further than this device. It hands the grant back to Google
first, and because the endpoint acts on the authorization rather than on the string it is handed,
that signs the person out of every Margin app on every machine they own; the confirmation says so
before it runs. Then the token and the registry entry go, and by default the account's mirror and
state database with them. The confirmation carries one box, ticked, for that last part: unticked,
the pair is moved from `accounts/<id>` to `kept/<id>`, where nothing that enumerates accounts can
see it, and it is moved back the day the same account is added again, decisions and all. An IMAP
account has nothing at Google to hand back, so removing one forgets its passwords and stops there.
The two actions used to be two buttons, Remove and Revoke, and read as a choice nobody could make.

## Sync

Polling, no push. `users.watch` needs a Pub/Sub subscription, which needs a server or an IAM
grant no desktop app should hold, and a relay is the thing that might drag the app into an
annual security assessment. `history.list` costs 2 units; polling every 12 seconds in the
foreground and 60 seconds in the background is a rounding error against the budget.

Initial sync fills the window, newest first, and the app is usable as soon as the first page
lands. A thin bar in the account chip says how far the mirror has got. Attachments are fetched on
open and cached under a size cap. An account that fails repeatedly is paused by a circuit breaker
rather than retried into a rate limit, and the account chip says so.

An account joins the engine the moment it is connected, and its first pass starts then rather
than at the next tick, which can be a minute away while the consent browser has the focus. One
pass at a time per account: a pass somebody asked for while the loop's is running gets the status
the running one is producing, and a first sync is never listed twice. A first sync that a quit or
a tunnel cut short is picked up by the next pass and reported the same way, with the count
carrying on from where it stopped, so an account still arriving never looks idle.

Bodies are the exception, and the rule is worth stating plainly: **opening a thread never waits on
the network.** `thread_view` is a local read. A message whose body has not arrived comes back with
`bodyPending` and the pane draws a placeholder where the text goes, then `thread_hydrate` fetches
what is missing eight at a time and the bodies appear on a `store-changed` of scope `thread`, which
refreshes the open thread and deliberately does not touch the list.

Behind that, the cache warms itself. Once the first sync has finished, each foreground pass takes
the forty newest bodies it does not have, which is about two hundred a minute against Gmail's six
thousand units and leaves room for roughly a hundred explicit opens in the same minute. The phase
is `caching` while it runs and the header says so. Two exclusions keep the queue moving: a body the
provider will not give up is remembered for the life of the process, so a handful of unfetchable
messages at the head cannot stall everything behind them, and transient rows pulled in by a
provider search are skipped, because a body fetched for a row the next eviction pass deletes is
twenty units spent on nothing. Both are excluded from the progress count as well as the fetch: work
nobody is going to do is not work outstanding.

Every write is optimistic: it lands in the mirror, renders, and is pushed behind. Consecutive
flag changes are coalesced into `batchModify`. Offline writes queue in the outbox and drain on
reconnect, with the thread showing "Waiting to send" until a send goes. A failure that is about
the connection or the account holds the queue; one the provider raised about the row itself (an
id it no longer has, a label it never had) is retried once and then dropped with its reason said
once, so a dead row never blocks the rows behind it. While a flag or label change is still queued,
the change log's word on those labels is applied under it rather than over it: what this device
decided is the truth about a message until the server has heard it.

Failures are handled the way Mailspring handles them, which is why nobody using Mailspring has seen
a sync error. A connection that dropped under a request (reset, closed before the answer, cut off in
the body) is tried again at once and then after a second, on a fresh connection, at the call site;
the pool keeps its connections alive with HTTP/2 pings so one the machine slept through is found
dead before a request lands on it. A pass that still fails is written down and otherwise kept
quiet: the chip does not move for one failure, because the next poll is twelve seconds away and is
the retry. It moves on the second in a row ("Offline" for the network, "Sync trouble" for the
rest) and the account pauses for five minutes on the fourth. Only three things are ever toasted:
the pause, because pressing sync is the way out of it; a refused token or a missing scope, because
nothing else mends them; and a write the provider refused for good, once, because the change did
not take. Sending a message and creating a draft are the two calls never repeated on a dropped
connection, since the first attempt may have gone through.

Every failure is also appended to `margin-mail.log` in the app data directory, capped at 256 KB:
each failed pass with the provider's sentence, each body that would not come, each command the
frontend called that answered with an error, and each uncaught error in the webview. An app
launched from the Finder has no stderr anybody will read, and a report of "sync failed" with
nothing behind it cannot be debugged.

Seen, starred, archived, trashed and spam are provider flags and go through the trait. Nothing in
the piles, the Screener, snoozes or notes ever touches the provider, so a user who screens out
two hundred senders makes zero API calls.

## The state journal and the backup store

The state database is written through an append-only journal: every change is an event with a
device id, a per-device sequence number, a wall-clock timestamp, a kind, a portable key, and a
payload. The tables are a materialised view of the journal. This is what makes roaming possible
without a server and what makes the backup meaningful.

A **backup store** is a trait with three operations: put a blob at a name, get a blob by name,
list names under a prefix. Two implementations ship: Google Drive and Cloudflare R2 through S3
credentials for people who run their own. Each device uploads its own journal segments under its
device id and downloads every other device's. Merging is last-writer-wins per key by timestamp,
which is correct for every kind of state here (a pile toggle, a note, a rule), and a device that
has been offline for a month simply replays what it missed.

The Drive implementation follows margin's, with one thing worth being accurate about: margin does
not use Drive's app-data space. It holds the `drive.file` scope and writes whole unencrypted files
into a visible folder named `margin`, which is deliberate, because a person should be able to see
their own backup. Margin Mail keeps the same scope, adds no new one, and writes encrypted journal
segments under `margin/mail/<account-hash>/<device-id>/`. Files created by the shared client are
visible to every Margin app, which is what makes one folder work for three of them. The HTTP
parts of margin's `gdrive.rs` port across; the encryption, the journal and the merge are new here.

Everything uploaded is encrypted on the device with XChaCha20-Poly1305 under a key that is
generated on first backup, stored sealed like the tokens, and shown once as a recovery phrase.
Drive and R2 hold ciphertext and names; neither can read a note or a rule. The recovery phrase is
the only way to attach a second device or restore after a lost one, and the Backup section of
settings says so in one sentence.

Version one on macOS alone does not need the merge. The journal shape is there from the first
commit so that the iPhone can join without a migration.

## Security

Message bodies are parsed from raw RFC 2822 with a real MIME parser (`mail-parser`), never from
the provider's pre-parsed payload beyond headers, because encoded words, parameter
continuations and legacy charsets appear daily. HTML is sanitised in Rust before it reaches the
webview: scripts, forms, event handlers, `<meta>` refreshes, external stylesheets, `javascript:`
and `data:` navigation are removed; `cid:` references are rewritten to inline data so the body
carries its own images. The body renders in a sandboxed iframe inside the app's webview so the
message can never touch the app, and it renders on the paper surface in both palettes when it
arrived as HTML: mail written for a white page usually sets a text colour and no background, and
inverting it breaks more than it fixes. Plain text is rendered by this app rather than by its
sender, so it follows the theme like everything else.

Tracker stripping happens in the same pass: images with a known tracking host (a maintained
list, shipped with the app and updated with it), images of one pixel or hidden by style, and
images whose URL carries a recipient token are removed and counted, and the vendor is named in
the banner. When the user asks to show images, Rust fetches them without cookies or referrer and
serves them from cache; the user's IP is exposed to the image host at that moment and only then,
which the Privacy section of settings says plainly. Outgoing mail never contains a tracker and the
app never requests a read receipt. Links are rewritten on click to drop known tracking parameters,
with a setting to turn that off.

Refresh tokens and the backup key are sealed, never in SQLite. The mirror and the state database
are files in the app data directory and inherit the OS's disk encryption; encrypting them again
would cost search and buy nothing on a device that is already locked, and the export path is
the answer for anyone who wants their mail in a form they control.

## Platforms

macOS: overlay title bar with the traffic lights on the header's centre line, closing the window
hides it and Cmd-Q quits, all from the calendar. iOS second: the same code with the phone chrome
from the calendar's `data-phone` and `data-touch` scheme, overlays as bottom sheets, the OAuth
flow through `ASWebAuthenticationSession`, and background app refresh used only to run the
snooze evaluation and a short sync.

Linux and Windows are the same code with the platform branches taken the other way: no traffic
lights, closing quits, and the menu bar built rather than adjusted, because neither is given the
File and View submenus macOS starts with. Linux ships as a deb, an AppImage, a flatpak and a Nix
package, Windows as an msi and an exe; which of the four a Linux reader should pick is in
[install.md](install.md).

## Order of work

The sync engine and the reading pane are the two hard things and neither proves the other, so the
first milestone after the scaffold is one account, authentication, the mirror, and a read-only
Inbox with a sanitised, tracker-stripped reading pane. Everything the app is for depends on those
being right, and everything after them is additive. The milestones and their work packages are in
[plan.md](plan.md).
