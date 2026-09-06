# Settings

Settings is a place, not a panel. `Cmd+,` opens it, so does the palette, so does the account chip,
so does the app menu, and all four land on the same screen: the whole stage, a rail of section
names down the left, one section on the right. No tabs, no modal, no nested pages. `Esc` goes back
to the place you came from, `j` and `k` walk the rail, and the screen is in
[mockups/settings.png](mockups/settings.png).

The reason it is a place is that a mail client accumulates decisions. Twelve sections of them is
too much for an overlay that steals the window and too much for a palette that shows one row at a
time, and both shapes make a person who is looking for the storage window read the whole list
twice. A rail is boring and it works.

What follows is the section list, in rail order, with what each one holds. Behaviour that is not
about the control itself is specified in [features.md](features.md).

## Accounts

The first section and the one people arrive at. Each linked account is a card: its colour (one of
the eight muted hues, and the same colour the account's rows carry in All accounts), the display
name, the signature for that address, and its aliases. The colour is pickable, the name and
signature are editable, and the aliases are read rather than edited here. An IMAP account says
plainly that it has none: neither IMAP nor SMTP has a way to publish them, so such an account only
ever sends as its own address.

What sits under the card depends on what kind of account it is, because the two have nothing in
common below the name.

A Google account shows the permissions it granted, one line each: mail, mail settings, contacts,
other contacts, calendar, backup. Granular consent means any of them can be missing, so each line
either reads as granted or carries a Grant button that runs the consent page again for the full
list and replaces the stored token. The wording of a missing line says what it costs in plain
terms, not what the scope is called: "Calendar is not connected, so invites are read-only."

An IMAP account has no permissions to show, because nothing was granted: it shows the two servers
it is actually using, incoming and outgoing, with the port and the security for each, the username
it logs in with, and where the password is kept. A permission list on such an account would be six
lines of Google vocabulary and a Grant button that opened a consent page for an account that has no
Google behind it. The footnote about the three Margin apps sharing one Google client is likewise
only shown when one of the accounts is Google.

Remove an account lists the accounts with one button each. The confirmation says what removing
costs, which for a Google account is Margin's access at Google for all three Margin apps on every
machine, because they share one client, and carries a single box, ticked, to delete the account's
mail and decisions from this computer as well. Unticked, they are set aside on disk and come back
if the account is added again. There is no separate Revoke: removing always revokes, and the box
is the only choice left.

Add account asks for the address and nothing else, and runs the flow the welcome screen runs, in a
sheet: a Google address hands over to the browser and the strip above the list waits for it, any
other address gets its sign-in step in the same sheet, and an Outlook address is told why it cannot
be added rather than given a button that fails. Once the account is written, a panel takes the
window: first the storage window, the same five spans as the row below with a month chosen
already, because the first sync reads it; then the sync itself, the engine's own line, a bar the
width of the panel and the count of messages in. Nothing closes it. When the pass ends it opens that account's Inbox, and the first-run
panel over it says how many senders were screened in. A first pass that stops shows the provider's
sentence with Try again and Go in anyway. The flow is specified in [ui.md](ui.md). Remove
account asks once, then deletes that
account's mirror, its state database and its stored secret, and says that the mail itself is
untouched on the provider. Last on the section, quiet and collapsed, is bring your own OAuth client: a
field for a client id and secret for someone who would rather not share the suite's, with one
sentence saying what it is for and that nothing else changes.

## Appearance

Theme: light, dark, or follow the system. Interface font and text font, from the catalogue
described at the foot of this document. Text size, which scales the reading pane's body copy and
nothing else, because the chrome is already at the size it wants to be. Reading pane on or off,
which is the same switch as `Cmd+\`. Density, which only appears on a phone and only chooses
between the comfortable row and a tighter one.

## Mail

The storage window, per account, as a row of choices: 30 days, 90 days, 180 days, a year,
everything. Under it, one line saying what the account currently holds and how far back it
reaches. Shrinking the window says how many threads will be evicted before it does it; widening
starts a backfill and shows the same thin bar the first sync uses. Threads carrying a pile, a
note, a snooze or any other decision are kept regardless, and the section says so.

Then two smaller things: the attachment cache cap, with the space currently used beside it, and
whether to prefetch message bodies inside the window when the app is idle. Both are about disk and
neither changes what is in a list.

## Privacy

Remote images: never, ask per message, or always. Link cleaning on or off. A list of the senders
allowed to load images, added from the contact card and removable here.

This section carries the one sentence the app owes the reader about what showing an image means:
loading a remote image tells the server that hosts it that you opened the message, from your IP
address, at that moment. Nothing else in the app reveals that, and the sentence sits under the
control rather than in a help page.

## Screener

The gate on or off. With it off, a sender with no rule is routed by the suggestion function and
nothing is held; with it on, first contact waits. Whether replies to threads you are already in
are held (they are not, by default, and the switch exists for the small number of people who want
absolutely everything screened). Suggestions on or off: with them off, the Screener card shows the
three destinations and no recommendation.

## Piles and snooze

The times behind the snooze choices: later today, tomorrow morning, the weekend, next week. Each
is a time of day or an offset the user can set once and forget. On a phone, the two swipe actions
and which direction each is on. And the Feed's automatic trash age, which is off by default and
can be set here for every Feed sender or from a contact card for one of them.

## Writing

The undo delay: five, ten, twenty or thirty seconds, ten by default. Whether `r` means reply or
reply all. The instant intro text, as an editable template with the placeholder it uses. And the
signature per account, which is the same field as the one on the account card in Accounts, shown
here because this is where somebody writing a signature will look for it.

## Notifications

New mail can arrive as a push notification on the machine the app is running on: a banner from the
system with the app's name, the sender and the subject, and a click on it opens the thread. Off by
default and the section says so first, with one

exception it names: the dock badge, which is on. One switch, Allow notifications, is the system's
permission and the app's own preference together, because nobody cares which of the two is saying
no. Turning it on asks the system when it has never been asked, and a yes turns the Inbox on with
it so the switch does something; a no leaves it off over one line saying notifications are turned
off for the app in System Settings and one button that opens them there, since the answer lives
there and not here. Only while the switch is on does the rest show: the three places, so somebody
who wants to be told about the Inbox and never about the Feed can say that once rather than thread
by thread, and a Send a test notification button. There is no sound setting. A notification comes
with the system's sound, and the system's own pane is where that is muted. The first place turned
on in the first run panel asks the same question. Last is the dock badge, with a note that it is
not a notification and needs no permission: it counts unseen Inbox threads across every account,
which is what is waiting for a decision rather than what is unread, and turning it off here takes
it off the dock at once.

## Keyboard

The keymap is a file, not a table of pickers. This section says where it is, opens it in the
system editor, and resets it to the defaults in [keyboard.md](keyboard.md). A file that fails to
parse is reported here with the line number and the defaults stay in force until it is fixed.

## Backup

Which store, if any: Google Drive or Cloudflare R2 with S3 credentials. The status of the
connection and when the last backup completed. The recovery phrase, shown once when backup is
first turned on and never again, with the sentence saying that it is the only way to attach a
second device or restore after a lost one, and that writing it down now is the whole of the
arrangement. Restore takes a recovery phrase and pulls the journal back down.

Never again is not a policy, it is the mechanism. The phrase is generated, the key it derives is
sealed, and the phrase itself is dropped; a device that could show it again would be a device that
had kept it, and then the phrase would protect nothing that the disk did not already give away.
The section says that in a line rather than apologising for it.

The store holds ciphertext and file names and nothing else, which the section says in a line
rather than a paragraph.

## Data

Export mail as mbox, per account, from the window that is on the device. Export app state as JSON
and import it back, which is the portable form of every decision the app holds. Storage used,
broken down into the mirror, the attachment cache and the state database. Clear the mirror, which
deletes the local copy of the mail, keeps every decision, and resyncs the window from scratch; it
is the answer to a corrupt database and it asks once.

## About

The version, the licence (FSL-1.1-MIT, with the line about each release turning MIT after two
years), check for updates on the desktop, and the "packaged by" note that names who built the
binary. Nothing else. This section is where somebody files a bug from, so the version string is
selectable.

## Where a setting lives

Two places, and the split is not arbitrary.

**Device settings** live in a `settings.json` in the app data directory, written atomically by
replacing the file rather than editing it, so a crash mid-write leaves the old file rather than
half a new one. Theme, both fonts, text size, the storage window, the cache caps, notifications,
the keymap path and the backup store's configuration are all device settings. They describe this
machine: the window that suits a laptop with a small disk is not the window that suits a desktop,
and a notification preference that roamed to a phone would be wrong on arrival.

**Roaming settings** are per account decisions that should follow the person rather than the
hardware: sender rules, signatures, the instant intro text. They live in the state database and go
through its journal like every other decision, so they reach a second device through the backup
store and survive a reinstall. The rule of thumb is that anything a person would be annoyed to
retype on a new machine roams, and anything about this machine's screen or disk does not.

## Fonts

The two font controls reuse margin-shared's catalogue rather than defining one here, because a
face named in one Margin app and missing in another is the bug that catalogue exists to prevent. A
`FontRef` is stored, not a family name, so a bundled face and a system face that happen to share a
name stay distinct.

Six families are bundled: Hanken Grotesk, Literata, EB Garamond, Lora, Source Serif 4 and
Fraunces. Alongside them the picker lists every family `fontdb` finds on the machine. Interface
font writes `--font-ui` on the root and text font writes `--font-heading`, which is the same pair
of variables the stylesheet already reads, so changing a face is a token change and touches
nothing else.
