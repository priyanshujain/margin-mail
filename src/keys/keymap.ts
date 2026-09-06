// One capture-phase listener for the whole app, and a context stack that decides what it is
// allowed to do.
//
// The stack starts empty, which means the view's keymap. A card, an editor or an overlay pushes a
// frame and the whole `view` context is shadowed until it pops, so whatever is in front owns the
// keyboard without any component having to remember to unbind anything. That is what lets `y` mean
// note in the list, yes on a Screener card and accept on an invite without any of the three
// knowing the others exist.
//
// Escape is not part of this: `src/escape.ts` already stacks Escape handlers and knows about nested
// confirmations, so this listener steps over the key entirely rather than racing it.
//
// A key is never taken from a text field unless the binding says so, and the two that say so are
// the palette, which is how you get out of anywhere, and the editor's own keys.

import { useEffect } from "react";
import { runCommand } from "./commands";
import {
  ACCOUNT_KEYS,
  BINDINGS,
  normalizeCombo,
  primaryHeld,
  secondaryHeld,
  type Binding,
  type KeyContext,
} from "./bindings";

const index = new Map<string, Binding[]>();
for (const binding of BINDINGS) {
  for (const key of binding.keys) {
    const combo = normalizeCombo(key);
    const found = index.get(combo);
    if (found) found.push(binding);
    else index.set(combo, [binding]);
  }
}

interface Frame {
  context: KeyContext;
}

const stack: Frame[] = [];

const activeContext = (): KeyContext => stack[stack.length - 1]?.context ?? "view";

/** Takes the keyboard until the returned function is called. Frames are identity, never by name. */
export function pushContext(context: KeyContext): () => void {
  const frame: Frame = { context };
  stack.push(frame);
  return () => {
    const at = stack.indexOf(frame);
    if (at !== -1) stack.splice(at, 1);
  };
}

/** The hook form, for a component that owns the keyboard while it is on screen. */
export function useKeyContext(context: KeyContext, active = true): void {
  useEffect(() => {
    if (!active) return;
    return pushContext(context);
  }, [context, active]);
}

/**
 * The combo an event means, in the same canonical form the table is read into.
 *
 * Shift is kept as a modifier rather than folded into the letter, so `Cmd+A` and `Cmd+Shift+A` are
 * different combos. A punctuation key that only exists shifted arrives with shift held and its own
 * character, so both spellings are in the table for those.
 */
export function comboOf(e: KeyboardEvent): string {
  const mods =
    (primaryHeld(e) ? "cmd+" : "") +
    (secondaryHeld(e) ? "ctrl+" : "") +
    (e.altKey ? "alt+" : "") +
    (e.shiftKey ? "shift+" : "");
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  return `${mods}${key}`;
}

/**
 * Which binding a combo means right now.
 *
 * The frames layer rather than replace. A Screener card declares `y`, `v`, `n` and `r`, and while
 * it is up those four mean what the card says; every other key still means what the view says, so
 * `j`, `k`, `Enter` and the place keys keep working on a screen that needs all of them. Only
 * `overlay` and `editor` shadow the view wholesale, because a panel and a text field really do own
 * the keyboard while they are in front.
 *
 * An earlier version of this function fell back from the top frame straight to `global`, which
 * meant pushing a card frame killed the list's keys. That is the bug this comment exists to stop
 * somebody reintroducing while tidying.
 */
const SHADOWS_VIEW: readonly KeyContext[] = ["overlay", "editor"];

function resolve(combo: string): Binding | null {
  const candidates = index.get(combo);
  if (!candidates) return null;
  const top = activeContext();

  const inTop = candidates.find((b) => b.context === top);
  if (inTop) return inTop;

  if (!SHADOWS_VIEW.includes(top)) {
    const inView = candidates.find((b) => b.context === "view");
    if (inView) return inView;
  }
  return candidates.find((b) => b.context === "global") ?? null;
}

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || typeof el.tagName !== "string") return false;
  if (el.isContentEditable) return true;
  return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT";
}

/**
 * A button the user can tab to activates itself on Enter, so the keymap leaves that alone. A list
 * row is `tabIndex={-1}` and only ever focused by a click, and it is the keymap's job to open it,
 * so the test is the tab index rather than the tag.
 */
function isActivatable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || typeof el.tagName !== "string" || el.tabIndex < 0) return false;
  return (
    el.tagName === "BUTTON" ||
    el.tagName === "A" ||
    el.tagName === "SUMMARY" ||
    el.getAttribute("role") === "button"
  );
}

/**
 * Switching account is nine keys and one idea, so it is not a command with nine bindings. The shell
 * hands over the one function that does it.
 */
let switchAccount: (index: number) => void = () => {};

export function setAccountSwitch(fn: (index: number) => void): () => void {
  switchAccount = fn;
  return () => {
    switchAccount = () => {};
  };
}

function onKeyDown(e: KeyboardEvent): void {
  if (e.isComposing || e.defaultPrevented) return;
  if (e.key === "Escape") return;
  if ((e.key === "Enter" || e.key === " ") && isActivatable(e.target)) return;

  const combo = comboOf(e);

  // The nine account keys, which are one idea rather than nine bindings.
  const account = ACCOUNT_KEYS.indexOf(combo);
  if (account !== -1 && activeContext() !== "overlay") {
    e.preventDefault();
    switchAccount(account);
    return;
  }

  const binding = resolve(combo);
  if (!binding) return;

  if (!binding.allowInInput && (isTyping(e.target) || isTyping(document.activeElement))) return;

  // A documented key with no command of ours: the editor's own, and Escape. Consume nothing.
  if (binding.command === null) return;

  e.preventDefault();
  e.stopPropagation();
  runCommand(binding.command);
}

let installs = 0;

/** Installs the one listener. Reference counted, so React's double effect in development is fine. */
export function installKeymap(): () => void {
  installs += 1;
  if (installs === 1) window.addEventListener("keydown", onKeyDown, true);
  return () => {
    installs -= 1;
    if (installs === 0) window.removeEventListener("keydown", onKeyDown, true);
  };
}

/** Mount once, at the top of the tree. */
export function useKeymap(): void {
  useEffect(() => installKeymap(), []);
}
