// Light or dark, on `data-theme` on the root and never on a media query, so the setting can
// disagree with the system and stay disagreed with. The boot script in index.html has already
// applied the stored value before React mounts, which is why the attribute is read first here:
// the DOM is the answer, and localStorage is only the fallback.

export type Theme = "light" | "dark";

const KEY = "marginmail-theme";

export function initialTheme(): Theme {
  const attr = document.documentElement.getAttribute("data-theme");
  if (attr === "light" || attr === "dark") return attr;
  const saved = localStorage.getItem(KEY);
  if (saved === "light" || saved === "dark") return saved;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function applyTheme(theme: Theme) {
  document.documentElement.setAttribute("data-theme", theme);
  localStorage.setItem(KEY, theme);
}
