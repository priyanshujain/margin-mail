// The keymap, declared once, and the command ids it dispatches.
//
// `keymap.ts` dispatches from this table, `Shortcuts.tsx` renders the `?` sheet from it, and the
// palette lists it, so a binding that exists but is undocumented is not something you can write:
// the sheet is generated, never maintained.
//
// The command ids and their labels live here rather than in `commands.ts` on purpose. `commands.ts`
// reaches for the stores and for Tauri the moment it is loaded, and the sheet, the palette and this
// table's own tests all want to name a command without starting the app.
//
// A combo is canonical: modifiers in `cmd+ctrl+alt` order, then `KeyboardEvent.key` verbatim. `cmd`
// means the platform's primary modifier, Command on macOS and Control everywhere else, which is
// what the native menu's `CmdOrCtrl` accelerators mean too. Shift is not a modifier here: it is
// already baked into the key, so `H` is the shifted `h` and reads that way in the table.
//
// Nothing is chorded and nothing is modal. Two keys never combine into a third meaning.

/**
 * Which frame of the context stack a binding belongs to.
 *
 * `view` is the list and the pane, which is where most of the app lives. The three card contexts
 * exist for the one exception docs/keyboard.md allows: `y`, `v` and `n` mean one thing on a
 * Screener card, another on an invite, and a third in the list, and the card is the only thing that
 * can receive them. `editor` carries no commands of ours; it exists so the editor's own keys are
 * documented and so this dispatcher stands out of TipTap's way. `overlay` carries none either:
 * pushing it is how an open panel shadows the whole view keymap while leaving `global` reachable.
 */
export type KeyContext =
  | "global"
  | "view"
  | "screener"
  | "invite"
  | "focus"
  | "editor"
  | "overlay";

export type BindingGroup =
  | "Navigation"
  | "Places"
  | "Triage"
  | "Screener"
  | "Reading"
  | "Writing"
  | "Focus & Reply"
  | "App";

export type CommandId =
  // navigation
  | "select-next"
  | "select-prev"
  | "open-selection"
  | "message-next"
  | "message-prev"
  | "message-toggle"
  | "message-expand-all"
  | "search"
  | "toggle-pane"
  // places
  | "place-inbox"
  | "place-feed"
  | "place-paper-trail"
  | "place-reply-later"
  | "place-set-aside"
  | "place-screener"
  | "place-snoozed"
  | "place-everything"
  // triage
  | "archive"
  | "toggle-seen"
  | "toggle-star"
  | "trash"
  | "spam"
  | "reply-later"
  | "set-aside"
  | "snooze"
  | "note"
  | "ignore"
  | "notify"
  | "merge"
  | "rename"
  | "contact-card"
  | "label"
  | "move"
  | "unsubscribe"
  | "select"
  | "select-extend-down"
  | "select-extend-up"
  | "select-all"
  | "undo"
  | "more"
  | "mark-all-seen"
  // screener
  | "screen-yes"
  | "screen-elsewhere"
  | "screen-no"
  | "screen-reply"
  // invites
  | "invite-accept"
  | "invite-maybe"
  | "invite-decline"
  // writing
  | "compose"
  | "reply"
  | "reply-all"
  | "forward"
  | "send"
  | "send-now"
  | "compose-expand"
  | "attach"
  | "instant-intro"
  | "remind-if-no-reply"
  | "save-clip"
  | "discard-draft"
  // focus and reply
  | "focus-reply"
  | "focus-next"
  | "focus-prev"
  // the libraries
  | "open-contacts"
  | "open-clips"
  | "open-files"
  // app
  | "command-palette"
  | "shortcuts"
  | "tour"
  | "guide"
  | "settings"
  | "sync-now"
  | "accounts"
  | "account-next"
  | "toggle-theme"
  | "check-updates"
  | "report-issue";

interface BindingBase {
  /** Every combo that runs it. The sheet shows them all; the dispatcher accepts any. */
  keys: readonly string[];
  context: KeyContext;
  group: BindingGroup;
  /** Off by default: a key must never be stolen from a text field. */
  allowInInput?: boolean;
}

export interface CommandBinding extends BindingBase {
  command: CommandId;
  /** The words the sheet, the palette and any tooltip all use. */
  label: string;
  /** Whether the palette lists it. Moving one row at a time is a key, not a menu entry. */
  palette?: boolean;
}

/** A key the keymap deliberately does not own, documented so the sheet is not a half-truth. */
export interface NoteBinding extends BindingBase {
  command: null;
  label: string;
}

export type Binding = CommandBinding | NoteBinding;

export const BINDINGS: readonly Binding[] = [
  // -- Navigation ---------------------------------------------------------------------------
  { keys: ["j"], command: "select-next", label: "Next thread", context: "view", group: "Navigation" },
  { keys: ["k"], command: "select-prev", label: "Previous thread", context: "view", group: "Navigation" },
  {
    keys: ["Enter"],
    command: "open-selection",
    label: "Open the thread, or read the selection together",
    context: "view",
    group: "Navigation",
  },
  { keys: ["n"], command: "message-next", label: "Next message", context: "view", group: "Reading" },
  { keys: ["p"], command: "message-prev", label: "Previous message", context: "view", group: "Reading" },
  {
    keys: ["o"],
    command: "message-toggle",
    label: "Expand or collapse this message",
    context: "view",
    group: "Reading",
  },
  {
    keys: ["shift+o"],
    command: "message-expand-all",
    label: "Expand every message",
    context: "view",
    group: "Reading",
  },
  {
    keys: [" "],
    command: null,
    label: "Scroll the reading pane down",
    context: "view",
    group: "Reading",
  },
  {
    keys: ["shift+ "],
    command: null,
    label: "Scroll the reading pane up",
    context: "view",
    group: "Reading",
  },
  { keys: ["/"], command: "search", label: "Search", context: "view", group: "Navigation", palette: true },
  {
    keys: ["cmd+\\"],
    command: "toggle-pane",
    label: "Show or hide the reading pane",
    context: "view",
    group: "Navigation",
    palette: true,
  },

  // -- Places -------------------------------------------------------------------------------
  { keys: ["1", "cmd+1"], command: "place-inbox", label: "Inbox", context: "view", group: "Places", palette: true },
  { keys: ["2", "cmd+2"], command: "place-feed", label: "Feed", context: "view", group: "Places", palette: true },
  {
    keys: ["3", "cmd+3"],
    command: "place-paper-trail",
    label: "Paper Trail",
    context: "view",
    group: "Places",
    palette: true,
  },
  { keys: ["4"], command: "place-reply-later", label: "Reply later", context: "view", group: "Places", palette: true },
  { keys: ["5"], command: "place-set-aside", label: "Set aside", context: "view", group: "Places", palette: true },
  { keys: ["6"], command: "place-screener", label: "Screener", context: "view", group: "Places", palette: true },
  { keys: ["7"], command: "place-snoozed", label: "Snoozed", context: "view", group: "Places", palette: true },
  { keys: ["0"], command: "place-everything", label: "Everything", context: "view", group: "Places", palette: true },

  // -- Triage -------------------------------------------------------------------------------
  { keys: ["e"], command: "archive", label: "Archive", context: "view", group: "Triage" },
  { keys: ["u"], command: "toggle-seen", label: "Mark seen or unseen", context: "view", group: "Triage" },
  { keys: ["shift+s"], command: "toggle-star", label: "Star", context: "view", group: "Triage" },
  { keys: ["#", "shift+#"], command: "trash", label: "Trash", context: "view", group: "Triage" },
  { keys: ["!", "shift+!"], command: "spam", label: "Mark as spam", context: "view", group: "Triage" },
  { keys: ["l"], command: "reply-later", label: "Reply later", context: "view", group: "Triage" },
  { keys: ["s"], command: "set-aside", label: "Set aside", context: "view", group: "Triage" },
  { keys: ["b"], command: "snooze", label: "Snooze", context: "view", group: "Triage" },
  { keys: ["y"], command: "note", label: "Add a note", context: "view", group: "Triage" },
  { keys: ["m"], command: "ignore", label: "Ignore this thread", context: "view", group: "Triage" },
  { keys: ["shift+n"], command: "notify", label: "Notify me on this thread", context: "view", group: "Triage" },
  { keys: ["g"], command: "merge", label: "Merge the selected threads", context: "view", group: "Triage" },
  { keys: ["i"], command: "contact-card", label: "Contact card", context: "view", group: "Triage" },
  { keys: ["shift+l"], command: "label", label: "Label", context: "view", group: "Triage" },
  { keys: ["v"], command: "move", label: "Move to Inbox, Feed or Paper Trail", context: "view", group: "Triage" },
  { keys: ["cmd+u"], command: "unsubscribe", label: "Unsubscribe", context: "view", group: "Triage" },
  { keys: ["x"], command: "select", label: "Select this thread", context: "view", group: "Triage" },
  { keys: ["shift+j"], command: "select-extend-down", label: "Extend the selection down", context: "view", group: "Triage" },
  { keys: ["shift+k"], command: "select-extend-up", label: "Extend the selection up", context: "view", group: "Triage" },
  { keys: ["cmd+a"], command: "select-all", label: "Select all from here", context: "view", group: "Triage" },
  { keys: ["z"], command: "undo", label: "Undo", context: "view", group: "Triage", palette: true },
  { keys: ["."], command: "more", label: "More actions", context: "view", group: "Triage" },

  // -- The Screener -------------------------------------------------------------------------
  { keys: ["y"], command: "screen-yes", label: "Yes, to the suggested place", context: "screener", group: "Screener" },
  {
    keys: ["v"],
    command: "screen-elsewhere",
    label: "Elsewhere: pick the place",
    context: "screener",
    group: "Screener",
  },
  { keys: ["n"], command: "screen-no", label: "No, screen them out", context: "screener", group: "Screener" },
  {
    keys: ["r"],
    command: "screen-reply",
    label: "Screen in to the Inbox and reply",
    context: "screener",
    group: "Screener",
  },

  // -- Invites ------------------------------------------------------------------------------
  { keys: ["y"], command: "invite-accept", label: "Accept the invitation", context: "invite", group: "Reading" },
  { keys: ["m"], command: "invite-maybe", label: "Maybe", context: "invite", group: "Reading" },
  { keys: ["n"], command: "invite-decline", label: "Decline", context: "invite", group: "Reading" },

  // -- Writing ------------------------------------------------------------------------------
  { keys: ["c", "cmd+n"], command: "compose", label: "New message", context: "view", group: "Writing", palette: true },
  { keys: ["r"], command: "reply", label: "Reply", context: "view", group: "Writing" },
  { keys: ["a"], command: "reply-all", label: "Reply all", context: "view", group: "Writing" },
  { keys: ["f"], command: "forward", label: "Forward", context: "view", group: "Writing" },
  {
    keys: ["cmd+Enter"],
    command: "send",
    label: "Send",
    context: "editor",
    group: "Writing",
    allowInInput: true,
  },
  {
    keys: ["cmd+shift+Enter"],
    command: "send-now",
    label: "Send now, with no undo",
    context: "editor",
    group: "Writing",
    allowInInput: true,
  },
  {
    keys: ["cmd+shift+p"],
    command: "compose-expand",
    label: "Expand the compose card to the window",
    context: "editor",
    group: "Writing",
    allowInInput: true,
  },
  {
    keys: ["cmd+shift+a"],
    command: "attach",
    label: "Attach a file",
    context: "editor",
    group: "Writing",
    allowInInput: true,
  },
  {
    keys: ["cmd+shift+i"],
    command: "instant-intro",
    label: "Instant intro: move the introducer to Bcc and thank them",
    context: "editor",
    group: "Writing",
    allowInInput: true,
  },
  {
    keys: ["cmd+shift+h"],
    command: "remind-if-no-reply",
    label: "Remind me if no reply",
    context: "editor",
    group: "Writing",
    allowInInput: true,
  },
  {
    keys: ["cmd+shift+c"],
    command: "save-clip",
    label: "Save the selection as a clip",
    context: "global",
    group: "Writing",
    allowInInput: true,
    palette: true,
  },
  {
    keys: ["cmd+shift+,", "cmd+shift+<"],
    command: "discard-draft",
    label: "Discard the draft",
    context: "editor",
    group: "Writing",
    allowInInput: true,
  },
  // The editor's own keys. Ours to document and TipTap's to handle, which is why the command is
  // null: the dispatcher sees the frame, finds no command, and stands out of the way. Cmd+K is the
  // one real collision in the app, and inside the editor the link wins, because a palette is one
  // Escape away and a link is not.
  {
    keys: ["cmd+b"],
    command: null,
    label: "Bold",
    context: "editor",
    group: "Writing",
    allowInInput: true,
  },
  {
    keys: ["cmd+i"],
    command: null,
    label: "Italic",
    context: "editor",
    group: "Writing",
    allowInInput: true,
  },
  {
    keys: ["cmd+k"],
    command: null,
    label: "Link",
    context: "editor",
    group: "Writing",
    allowInInput: true,
  },

  // -- Focus & Reply ------------------------------------------------------------------------
  {
    keys: ["shift+f"],
    command: "focus-reply",
    label: "Focus & Reply",
    context: "view",
    group: "Focus & Reply",
    palette: true,
  },
  { keys: ["Tab"], command: "focus-next", label: "Next item", context: "focus", group: "Focus & Reply" },
  // The three libraries. No keys: the number keys are the seven daily places and these are not
  // daily, so the palette is how they are reached and the palette is enough.
  {
    keys: [],
    command: "open-contacts",
    label: "Contacts",
    context: "view",
    group: "Places",
    palette: true,
  },
  {
    keys: [],
    command: "open-clips",
    label: "Clips",
    context: "view",
    group: "Places",
    palette: true,
  },
  {
    keys: [],
    command: "open-files",
    label: "All files",
    context: "view",
    group: "Places",
    palette: true,
  },
  {
    keys: ["shift+Tab"],
    command: "focus-prev",
    label: "Previous item",
    context: "focus",
    group: "Focus & Reply",
  },

  // -- The app ------------------------------------------------------------------------------
  // The palette is the one thing a text field may not swallow: it is how you get out of anywhere.
  // The editor shadows it with its own Cmd+K, which is the exception that proves it.
  {
    keys: ["cmd+k"],
    command: "command-palette",
    label: "Command palette",
    context: "global",
    group: "App",
    allowInInput: true,
  },
  { keys: ["?", "shift+?", "cmd+/"], command: "shortcuts", label: "Keyboard shortcuts", context: "view", group: "App", palette: true },
  // Escape unwinds the layer stack in `src/escape.ts`, which knows about nested confirmations.
  {
    keys: ["Escape"],
    command: null,
    label: "Dismiss whatever is open",
    context: "global",
    group: "App",
  },
  { keys: ["cmd+r"], command: "sync-now", label: "Sync now", context: "view", group: "App", palette: true },
  { keys: ["cmd+,"], command: "settings", label: "Settings", context: "view", group: "App", palette: true },
  // No key: renaming a thread is a thing you do to one thread once, and every letter worth a key
  // is already spoken for. The subject in the pane is the control; this row is how somebody finds
  // out that it is one.
  {
    keys: [],
    command: "rename",
    label: "Rename this thread",
    context: "view",
    group: "Triage",
    palette: true,
  },
  // A link on the New for you heading and a palette row, with no key of its own: it is the sort of
  // thing you do once a month, and a key for it would be a key you press by accident.
  {
    keys: [],
    command: "mark-all-seen",
    label: "Mark all as seen",
    context: "view",
    group: "Triage",
    palette: true,
  },
  // No key of its own. The theme is a setting rather than a verb, so it lives in the palette until
  // the Appearance section exists, and a binding with no keys is how the palette lists a command
  // the keyboard does not reach.
  {
    keys: [],
    command: "toggle-theme",
    label: "Toggle dark mode",
    context: "view",
    group: "App",
    palette: true,
  },
  {
    keys: ["ctrl+0"],
    command: "accounts",
    label: "All accounts",
    context: "view",
    group: "App",
    palette: true,
  },
  // The two help rows. Neither has a key, because the corner button and the palette are both one
  // gesture already and a letter spent on the thing you need twice is a letter taken from a verb
  // you need hourly. The shortcut sheet above keeps `?`, which is the one help key anybody guesses.
  {
    keys: [],
    command: "tour",
    label: "Take the tour",
    context: "view",
    group: "App",
    palette: true,
  },
  {
    keys: [],
    command: "guide",
    label: "Guide",
    context: "view",
    group: "App",
    palette: true,
  },
] as const;

/**
 * `Ctrl+1` to `Ctrl+9` switch account, which is nine bindings that are one idea. They are generated
 * rather than typed out, because nine near-identical rows in the sheet is nine chances to get one
 * wrong and no chance at all of noticing.
 */
export const ACCOUNT_KEYS: readonly string[] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"].map(
  (n) => `ctrl+${n}`,
);

export const GROUPS: readonly BindingGroup[] = [
  "Navigation",
  "Places",
  "Triage",
  "Reading",
  "Screener",
  "Writing",
  "Focus & Reply",
  "App",
];

const isMac =
  typeof navigator !== "undefined" && /mac|iphone|ipad/i.test(navigator.userAgent ?? "");

/** The primary modifier as the platform names it. */
export const PRIMARY_LABEL = isMac ? "⌘" : "Ctrl+";

/** True when the event holds the platform's primary modifier, whatever the hardware calls it. */
export const primaryHeld = (e: { metaKey: boolean; ctrlKey: boolean }): boolean =>
  isMac ? e.metaKey : e.ctrlKey;

export const secondaryHeld = (e: { metaKey: boolean; ctrlKey: boolean }): boolean =>
  isMac ? e.ctrlKey : e.metaKey;

/**
 * The canonical form of a combo, which is what the dispatcher builds and the table is read into.
 *
 * Modifiers in `cmd+ctrl+alt+shift` order, then the key: lowercased when it is a single character,
 * verbatim when it is a named key like `Enter`. Shift is always an explicit modifier rather than
 * being baked into the letter, because this app binds both `Cmd+A` and `Cmd+Shift+A` and a table
 * that folded shift into the key could not tell them apart.
 *
 * A bare capital in the table is an affordance for writing `H` rather than `shift+h`, and it means
 * the same thing.
 */
export function normalizeCombo(combo: string): string {
  // A combo whose key is `+` splits into a trailing empty part; a combo whose key is a space keeps
  // it, which is why this does not trim.
  const parts = combo.split("+");
  let key = parts.pop() ?? "";
  if (key === "" && parts.length > 0) key = "+";
  const mods = new Set(parts.filter((p) => p !== "").map((p) => p.toLowerCase()));

  // A bare capital means shift, because `H` is how a person writes the shifted `h`. A capital
  // behind a modifier does not, because `Cmd+K` is how a person writes Cmd and K, and reading that
  // as Cmd+Shift+K would break every combo in the table that is written the ordinary way.
  if (key.length === 1 && /[a-z]/i.test(key)) {
    if (mods.size === 0 && key !== key.toLowerCase()) mods.add("shift");
    key = key.toLowerCase();
  } else if (key.length === 1) {
    key = key.toLowerCase();
  }

  const prefix = ["cmd", "ctrl", "alt", "shift"].filter((m) => mods.has(m)).join("+");
  return prefix ? `${prefix}+${key}` : key;
}

const NAMED: Record<string, string> = {
  Enter: "↩",
  Escape: "⎋",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Tab: "⇥",
  " ": "Space",
};

/** `cmd+k` becomes ⌘K and `shift+h` becomes ⇧H. What the sheet, the palette and buttons print. */
export function keyLabel(combo: string): string {
  const parts = normalizeCombo(combo).split("+");
  let key = parts.pop() ?? "";
  if (key === "" && parts.length > 0) key = "+";
  const printed = parts
    .map((m) => (m === "cmd" ? PRIMARY_LABEL : m === "ctrl" ? "⌃" : m === "shift" ? "⇧" : "⌥"))
    .join("");
  const named = NAMED[key];
  if (named) return `${printed}${named}`;
  return `${printed}${key.length === 1 ? key.toUpperCase() : key}`;
}

/** The combos a command answers to, for a palette row or a button's title attribute. */
export function keysFor(id: CommandId): readonly string[] {
  return BINDINGS.find((b) => b.command === id)?.keys ?? [];
}

/** The first combo a command answers to, which is the one a button prints. */
export function keyFor(id: CommandId): string | null {
  const keys = keysFor(id);
  return keys.length > 0 ? keys[0] : null;
}

export function labelFor(id: CommandId): string {
  return BINDINGS.find((b) => b.command === id)?.label ?? id;
}

/** Every command the palette lists, in declaration order. */
export const PALETTE_COMMANDS: readonly CommandBinding[] = BINDINGS.filter(
  (b): b is CommandBinding => b.command !== null && b.palette === true,
);
