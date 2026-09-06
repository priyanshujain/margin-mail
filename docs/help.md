# Help

Three things, and they are separate on purpose: a tour that runs once when an account is added, a
question mark in the corner that is there for good, and a guide behind it that answers the
questions this app raises by not working like the last one. Behaviour that is being explained is
specified in [features.md](features.md); the keys are in [keyboard.md](keyboard.md) and every
keycap printed anywhere here is generated from the same binding table, so a remapped key is what
the help says.

The premise is that this app is unusual and that pretending otherwise is what makes people leave.
Mail from somebody new does not arrive. There are three boxes rather than one. Flags are two piles
with keys on them. None of that is discoverable by poking at it, and none of it is a reason to
turn the first hour into a wizard either.

## The tour

Nine slides in a sheet, over the Inbox it is talking about. It follows the first-run panel, which
is the moment the account is set up and the mail is in, and it ends where the app expects you to
carry on: the Screener, the three boxes, the two piles, snooze, the keyboard, the palette, the
things kept beside the mail, what is quiet by default, and undo.

Skip is the first control on it and Escape is the same answer, so it costs one keystroke to
refuse. The arrows and the return key move through it, the dots at the foot say how far in you are
and can be pressed to jump, and the last slide says where to find all of this again.

It runs for every account that is added rather than only the first. The panel it follows is per
account too: a second mailbox on a shared machine is somebody else's first look at the app, and
the flag that remembers is on the device rather than in the state that roams. Somebody adding
their own second account has seen it before and skips it, which is one key.

The slides state facts and nothing else. No animation beyond a fade, nothing bouncing, no
illustration that is not a diagram of the real thing, and no screenshot: the app is behind the
sheet, so a picture of it would be a picture of what is already there.

## The corner

A small round question mark fixed in the bottom right, and the only permanent chrome the app has
outside the header. It opens three rows: the tour again, the guide, and the keyboard shortcuts
with its key printed.

It gets out of the way rather than floating over everything. It is not drawn while the compose
card is open, because they share that corner and compose is the thing you are doing; not while any
overlay is up, because it would be under the scrim; not in the guide, which is where it goes; and
not on a phone, where the tab bar owns that corner and the palette behind the places button is
what reaches all three.

The same three are in the native Help menu, above Report an Issue, because that is the first place
a Mac user looks and a menu bar item costs nothing.

## The guide

A panel over the whole window, with the app dimmed behind it. It is read about the app rather than
instead of it, so nothing behind it can be pressed while it is up and the close control puts back
exactly what was there: the place, the open thread, the scroll position. Escape is the same answer.
Inside, a search field across the whole width, then a rail of sections down the left and one
article at a time on the right at a reading measure.

Search is the first thing in the panel and it takes the keyboard the moment the guide opens,
because somebody who came here has a question rather than an appetite for a table of contents. It
reads what the articles say and not only what they are called. A query nothing answers is the one
moment the guide has failed, so that is where it offers to open a question against the repository,
labelled `question` and titled with what was typed, and it says out loud that the page it opens is
public.

Articles are written to be looked at before they are read. The answer is the first paragraph, a
drawn figure or a screenshot carries the idea under it, and a verb that would otherwise be a
sentence in a list of six is a table of the key and what it does, generated from the binding table.
The rule is enforced rather than hoped for: `guide.test.ts` fails on a paragraph over seventy five
words, and on an article over a hundred and twenty words with nothing in it to look at.

The figures are drawn from the app's own parts rather than exported from a drawing program, in
`src/screens/guide/Figures.tsx`. A picture is the app photographed and a figure is an idea drawn,
and the ideas are the things no camera can be pointed at: where mail goes, what a pile does, how
long a send is held.

It was a stage first, and that was wrong twice over. A stage wins over a place, so the header sat
above it with three box buttons that changed a place nobody could see; and a library about the app
is not somewhere you go instead of your mail, it is something you hold up in front of it. Making it
a panel found the second half of the same bug: the title bar carries a stacking order of its own so
that a popover can hang off the account chip, and it was above every scrim in the app, which left
the palette, the shortcut sheet and the tour all reachable past their own scrim at the top of the
window. Overlays now sit above it, which is the rule the phone stylesheet had already worked out
for its tab bar.

The sections run in the order somebody meets the problem: getting started, the Screener, reading,
triage, writing, organising, accounts, and then the questions. An article is prose with steps
where there are steps and the key printed where there is a key. The questions are the last section
and they are the ones people actually ask: where a newsletter went, whether a screened-out sender
is told (they are not), why mail from last year is not here, why there is no Empty Trash button,
whether anything reads the mail with AI (nothing does), and what happens to the mailbox if the app
is deleted.

The pictures come from the browser suite rather than from anybody's mailbox. `just guide-shots`
drives the dev fixture and writes ten PNGs into `public/guide/`, at twice the display size for a
retina screen, and they are committed because they ship inside the bundle. The recipe is not part
of `just test-ui`: an ordinary run of the suite must not rewrite files that are in the tree.

## Not here

No coach marks over the interface, no tooltip that follows you around, no "did you know" after the
third launch, no checklist of things to finish, no progress bar over your own mailbox. The tour is
one sheet you can refuse, and after that help is somewhere you go and find rather than something
that arrives while you are reading your mail.
