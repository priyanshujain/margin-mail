// The three appearance settings that are a fact about the page rather than a fact in a store: the
// two font slots, the reading size, and the theme when it is set to follow the system.
//
// Out here rather than inside the settings store for the reason src/theme.ts and src/pane.ts are
// out here: a store may not touch the DOM, and all three of these have to be on the root before a
// paint rather than after one. The blocking script in index.html restores them before the first
// paint, so the shapes written to localStorage here are the shapes that script reads back.

import { fontStack, type FontRef } from "margin-shared/fonts";
import type { Theme } from "./theme";

const FONTS_KEY = "marginmail-fonts";
const SIZE_KEY = "marginmail-text-size";
const THEME_KEY = "marginmail-theme";

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

export const systemTheme = (): Theme =>
  window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";

/**
 * Follow the system, which is the absence of a choice rather than a third value.
 *
 * `applyTheme` writes light or dark, and the boot script prefers what it finds there over the
 * media query, so leaving the key behind would pin the app to whatever the system happened to be
 * on the evening somebody chose to follow it.
 */
export function forgetThemeChoice(): void {
  localStorage.removeItem(THEME_KEY);
}
