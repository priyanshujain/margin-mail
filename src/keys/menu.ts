// The native menu emits `menu-action` with the item id it was built with. Those ids are command
// ids, so this is a guard and a lookup rather than a second dispatch table: a menu item and a
// keystroke run the same function or the build fails.
//
// The Tauri listener itself is mounted at the top of the tree; this is what it calls.

import type { CommandId } from "./bindings";
import { runCommand } from "./commands";

/** Exactly the ids `src-tauri/src/lib.rs` emits, and every one of them is a command. */
const MENU_IDS: readonly CommandId[] = [
  "compose",
  "command-palette",
  "sync-now",
  "check-updates",
  "settings",
  "search",
  "shortcuts",
  "place-inbox",
  "place-feed",
  "place-paper-trail",
  "toggle-pane",
  "guide",
  "tour",
  "report-issue",
];

const known = new Set<string>(MENU_IDS);

export function handleMenuAction(id: string): void {
  if (known.has(id)) runCommand(id as CommandId);
}

/** The ids this module answers to, so a test can check them against the Rust side's list. */
export const MENU_COMMAND_IDS = MENU_IDS;
