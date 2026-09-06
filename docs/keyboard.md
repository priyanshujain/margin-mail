# Keyboard

One unmodified key per verb, no two-key chords, nothing modal. Where Gmail and Superhuman agree,
we use their key so a refugee's hands keep working. Where HEY has a verb they do not, we use
HEY's letter. The three conflicts that result are settled below and explained. The key is printed
on every button, the palette lists every command with its key, and `?` shows this table in the
app. Bindings live in a keymap file the user can edit; the defaults are what follows.

## Navigation

| Key | Action |
|---|---|
| `j` / `k` | Next / previous thread (or card in the Feed and Screener) |
| `Enter` | Open the focused thread; with several selected, Read together |
| `Esc` | Close what is open: a popover, then the palette, then the compose card, then the pane in one-column mode, then the selection |
| `n` / `p` | Next / previous message inside a thread |
| `o` / `Shift+O` | Expand or collapse the focused message / expand all |
| `Space` / `Shift+Space` | Scroll the pane down / up |
| `1` `2` `3` | Inbox, Feed, Paper Trail |
| `4` `5` | Reply later, Set aside |
| `6` `7` | Screener, Snoozed |
| `0` | Everything |
| `/` | Search |
| `Cmd+K` | Palette: places, commands, people, settings |
| `Ctrl+1` to `Ctrl+9`, `Ctrl+0` | Switch account, All accounts |
| `Cmd+\` | Show or hide the reading pane |
| `Cmd+,` | Settings |
| `?` | This table |

## Triage

| Key | Action | Provider |
|---|---|---|
| `e` | Archive | yes |
| `u` | Toggle seen | yes |
| `Shift+S` | Toggle star | yes |
| `#` | Trash, and put back in Trash (toggle) | yes |
| `!` | Spam, and not spam in Spam (toggle) | yes |
| `l` | Reply later (toggle) | no |
| `s` | Set aside (toggle) | no |
| `b` | Snooze… | no |
| `y` | Note | no |
| `m` | Ignore (toggle) | no |
| `Shift+N` | Notify me on this thread (toggle) | no |
| `g` | Merge the selected threads | no |
| `i` | Contact card for the sender | no |
| `Shift+L` | Label… | yes |
| `v` | Move to Inbox, Feed or Paper Trail (sets the sender's rule); in a label list, move to a label | rule: no, label: yes |
| `Cmd+U` | Unsubscribe… | sends or opens |
| `x` | Select the focused thread | |
| `Shift+J` / `Shift+K` | Extend the selection down / up | |
| `Cmd+A` | Select all from here | |
| `z` | Undo the last action, including a send within its delay | |
| `.` | More actions for the thread | |

## Screener

| Key | Action |
|---|---|
| `y` | Yes, to the suggested place |
| `v` | Elsewhere: pick the place, optionally for the whole domain |
| `n` | No, screen out |
| `Enter` | Expand the card to read the message |
| `r` | Screen in to Inbox and reply |

## Writing

| Key | Action |
|---|---|
| `c` | New message |
| `r` / `a` / `f` | Reply / reply all / forward |
| `Cmd+Enter` | Send (held for the undo delay) |
| `Cmd+Shift+Enter` | Send now, no undo |
| `Cmd+Shift+P` | Expand the compose card to the window, or back |
| `Cmd+Shift+A` | Attach |
| `Cmd+Shift+I` | Instant intro: move the introducer to Bcc and thank them |
| `Cmd+Shift+H` | Remind me if no reply… |
| `Cmd+Shift+C` | Save the selected text as a clip (also outside compose) |
| `Cmd+B` / `Cmd+I` / `Cmd+K` | Bold / italic / link inside the editor |
| `Cmd+Shift+7` / `8` / `9` | Numbered list / bulleted list / quote |
| `Cmd+Shift+,` | Discard the draft |
| `Esc` | Leave the editor, keeping the draft |

## Focus & Reply

| Key | Action |
|---|---|
| `Shift+F` | Open Focus & Reply from anywhere |
| `Tab` / `Shift+Tab` | Next / previous item |
| `Cmd+Enter` | Send this reply and move on |
| `Esc` | Back to Reply later |

## Calendar invites

| Key | Action |
|---|---|
| `y` / `m` / `n` | Accept / maybe / decline, when an invite card is focused |

## The conflicts, and how they were settled

`a` is reply all in Gmail and Superhuman and Set aside in HEY. Reply all is used every day, so it
keeps `a`; Set aside takes `s`, which is star in Gmail. Star is a vestige here (the piles do its
job), so star moves to `Shift+S` and stays reachable.

`l` is label in Gmail and Superhuman and Reply later in HEY. Reply later is a primary verb in this
app and labels are not, so `l` is Reply later and label is `Shift+L`.

`z` is undo in Gmail and Superhuman and Bubble Up in HEY. Undo is sacred; `z` stays undo. Snooze
takes `b`, which is Gmail's own snooze key and also reads as Bubble Up for HEY hands.

`m` is mute in Gmail and Ignore in HEY, the same verb, so it is `m`. `y` is a sticky note in HEY
and a rarely known "remove from view" in Gmail; it is a note here. `g` is HEY's merge; in Gmail it
begins a chord we do not have, so on its own it merges a selection and does nothing otherwise.

Keys are never reused with a different meaning in a different context, with one exception: `y`,
`v` and `n` on a Screener card and on an invite card, where the card is the only thing that can
receive them and the buttons print the letters. HEY reuses `o`, `r` and `i` contextually and it
is the one thing about its keyboard worth not copying.

## Rules

- Single keys act on the focused thread, or on the selection when there is one.
- A key that would act on nothing does nothing and shows nothing.
- Every destructive or hidden action (archive, trash, spam, snooze, pile, screen out, merge,
  send) produces a toast with Undo, and `z` undoes it.
- On a phone there are no keys. The same verbs are buttons on the action bar and swipes on rows,
  and the printed letters disappear.
- The keymap is a file in the app's data directory; the shortcuts sheet is generated from it, so
  a remapped key is what the buttons print.
