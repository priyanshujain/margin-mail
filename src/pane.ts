// The reading pane's visibility, on `data-no-pane` on the root and in localStorage, exactly the
// shape `src/theme.ts` uses for the theme and for the same reason: the boot script in index.html
// has already applied the stored value before React mounts, so the DOM is the answer and storage
// is only the fallback.
//
// It lives out here rather than inside `useMail` because the store may not touch the DOM, and it
// has to be an attribute rather than a class on a component: whether there is a second column is a
// layout fact the stylesheet needs on the first paint, not after a render.

const KEY = "marginmail-pane";

export function initialPane(): boolean {
  if (document.documentElement.hasAttribute("data-no-pane")) return false;
  return localStorage.getItem(KEY) !== "0";
}

export function applyPane(open: boolean): void {
  document.documentElement.toggleAttribute("data-no-pane", !open);
  localStorage.setItem(KEY, open ? "1" : "0");
}
