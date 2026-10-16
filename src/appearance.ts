// The appearance settings that are a fact about the page rather than a fact in a store: the two
// font slots and the reading size.
//
// Out here rather than inside the settings store for the reason src/theme.ts and src/pane.ts are
// out here: a store may not touch the DOM, and both of these have to be on the root before a
// paint rather than after one. The blocking script in index.html restores them before the first
// paint, so the shapes written to localStorage here are the shapes that script reads back.

import { fontStack, type FontRef } from "margin-shared/fonts";

const FONTS_KEY = "marginmail-fonts";
const SIZE_KEY = "marginmail-text-size";

/** The interface face and the text face, as the two CSS stacks the boot script assigns. */
export function applyFonts(ui: FontRef, text: FontRef): void {
  const stacks = { ui: fontStack(ui), text: fontStack(text) };
  const root = document.documentElement;
  root.style.setProperty("--font-ui", stacks.ui);
  root.style.setProperty("--font-heading", stacks.text);
  localStorage.setItem(FONTS_KEY, JSON.stringify(stacks));
}

/** Message bodies and nothing else. The chrome is already at the size it wants to be. */
export function applyTextSize(px: number): void {
  document.documentElement.style.setProperty("--body-size", `${px}px`);
  localStorage.setItem(SIZE_KEY, String(px));
}
