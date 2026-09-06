// The command registry.
//
// A key, a menu item, a palette row and a button all end up here, which is the point: the native
// menu emits an id and that id is a command, not a second code path.
//
// The calendar's version of this file holds the dispatch table itself and therefore imports every
// store in the app. This one is a registry instead: `bindings.ts` declares what commands exist and
// what they are called, and whichever screen owns a verb registers the function that performs it
// while it is mounted. That keeps this whole directory free of the stores, which is what lets the
// binding table, the shortcut sheet and the palette be tested without starting the app, and it
// means a command whose screen is not on screen is simply not registered rather than being a
// function that has to check.

import type { CommandId } from "./bindings";

export type Handler = () => void;

const handlers = new Map<CommandId, Handler[]>();

/**
 * Registers handlers for as long as the caller is mounted, and returns the function that takes
 * them away again. The most recently registered handler for a command wins, so a screen that
 * takes over a verb while it is open puts it back on the way out.
 */
export function registerCommands(map: Partial<Record<CommandId, Handler>>): () => void {
  const added: [CommandId, Handler][] = [];
  for (const [id, handler] of Object.entries(map) as [CommandId, Handler | undefined][]) {
    if (!handler) continue;
    const stack = handlers.get(id) ?? [];
    stack.push(handler);
    handlers.set(id, stack);
    added.push([id, handler]);
  }
  return () => {
    for (const [id, handler] of added) {
      const stack = handlers.get(id);
      if (!stack) continue;
      const at = stack.lastIndexOf(handler);
      if (at !== -1) stack.splice(at, 1);
      if (stack.length === 0) handlers.delete(id);
    }
  };
}

/** True when something is listening, which is how the palette dims a row it cannot run. */
export function isRegistered(id: CommandId): boolean {
  return (handlers.get(id)?.length ?? 0) > 0;
}

/**
 * Runs a command, or does nothing at all when nobody owns it.
 *
 * Doing nothing is deliberate and it is the rule docs/keyboard.md sets: a key that would act on
 * nothing does nothing and shows nothing. Pressing `l` with no thread selected is not an error and
 * must not produce a toast saying so.
 */
export function runCommand(id: CommandId): void {
  const stack = handlers.get(id);
  if (!stack || stack.length === 0) return;
  stack[stack.length - 1]();
}

/** Case-insensitive subsequence, so `pt` finds "Paper Trail" and `arc` finds "Archive". */
export function commandMatches(label: string, query: string): boolean {
  const needle = query.toLowerCase().replace(/\s+/g, "");
  if (!needle) return true;
  const hay = label.toLowerCase();
  let at = 0;
  for (const ch of needle) {
    at = hay.indexOf(ch, at);
    if (at === -1) return false;
    at += 1;
  }
  return true;
}
