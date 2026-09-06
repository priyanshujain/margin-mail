// The keyboard, which is the way this app is meant to be used.
//
// Every assertion here presses a key and then looks at the page, because the point of the keymap is
// not that a handler ran: it is that the focus moved, the place changed, or the panel opened.

import { expect, test } from "@playwright/test";
import { MIDDAY, box, listReady, openApp, openDialog, paletteRows, place, rows, settle } from "./app";

test("j and k move the focus, and it stays on screen", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("j");
  expect((await rows(page)).find((row) => row.selected)?.sender).toBe("Karthik Rao");

  await page.keyboard.press("j");
  await page.keyboard.press("j");
  expect((await rows(page)).find((row) => row.selected)?.sender).toBe("Arun Kulkarni");

  await page.keyboard.press("k");
  expect((await rows(page)).find((row) => row.selected)?.sender).toBe("Maya Raghunathan");

  // Down past the fold: the focused row has to be brought into the list's own viewport.
  for (let i = 0; i < 12; i++) await page.keyboard.press("j");
  await settle(page);
  const list = await box(page.locator(".list"));
  const focused = await box(page.locator(".row[data-selected]"));
  expect(focused.top).toBeGreaterThanOrEqual(list.top - 1);
  expect(focused.bottom).toBeLessThanOrEqual(list.bottom + 1);
});

test("Enter opens the focused thread", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await expect(page.locator(".thread-subject")).toHaveCount(0);
  await page.keyboard.press("j");
  await page.keyboard.press("j");
  await page.keyboard.press("Enter");
  await expect(page.locator(".thread-subject")).toHaveText("Dinner on Thursday?");

  // Opening it marks it seen, so the dot goes.
  await expect
    .poll(async () => (await rows(page)).find((row) => row.sender === "Maya Raghunathan")?.unseen)
    .toBe(false);
});

test("1, 2 and 3 change place", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  // The Feed takes the whole stage rather than the list column, so the place on the root is what
  // says where you are; the list head only exists for the places that have a list.
  await page.keyboard.press("2");
  await expect.poll(() => place(page)).toBe("feed");
  await page.keyboard.press("3");
  await expect(page.locator(".list-title")).toHaveText("Paper Trail");
  await page.keyboard.press("1");
  await expect(page.locator(".list-title")).toHaveText("Inbox");
  await expect.poll(() => place(page)).toBe("inbox");

  // The header's segment says the same thing the list head does.
  await expect(page.locator(".segment-option[data-active]")).toHaveText(/Inbox/);
});

test("Cmd+backslash hides the pane and the list takes the width", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  expect((await box(page.locator(".list-col"))).width).toBe(420);

  await page.keyboard.press("Meta+\\");
  await settle(page);
  await expect(page.locator(".pane")).toHaveCount(0);
  const wide = await box(page.locator(".list-col"));
  expect(wide.width).toBeGreaterThan(1000);

  // With no pane a thread opens in place, and Escape gives the list back.
  await page.keyboard.press("j");
  await page.keyboard.press("Enter");
  await expect(page.locator(".thread-subject")).toBeVisible();
  await expect(page.locator(".list-col")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.locator(".list-col")).toHaveCount(1);

  await page.keyboard.press("Meta+\\");
  await settle(page);
  expect((await box(page.locator(".list-col"))).width).toBe(420);
});

test("the palette opens on Cmd+K, filters, and prints the key of every row that has one", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("Meta+k");
  expect(await openDialog(page)).toBe("Command palette");

  const all = await paletteRows(page);
  expect(all.map((row) => row.label)).toContain("Inbox");
  expect(all.map((row) => row.label)).toContain("Paper Trail");
  expect(all.find((row) => row.label === "Inbox")?.keys).toEqual(["1"]);
  expect(all.find((row) => row.label === "Paper Trail")?.group).toBe("Places");
  // The invariant is not that every row has a key. Several deliberately do not: the folders, the
  // provider's labels, a setting the palette is the only way to reach, an action you run once a
  // month. What must hold is that a row either prints a real key or prints no cap at all, because
  // an empty keycap is a promise the keyboard does not keep.
  for (const row of all) {
    for (const key of row.keys) expect(key.trim()).not.toBe("");
  }
  // And the rows that do answer to a key print the one docs/keyboard.md gives them.
  expect(all.find((row) => row.label === "Feed")?.keys).toEqual(["2"]);
  expect(all.find((row) => row.label === "Reply later")?.keys).toEqual(["4"]);
  expect(all.find((row) => row.label === "Sync now")?.keys).toEqual(["⌘R"]);
  expect(all.find((row) => row.label === "Toggle dark mode")?.keys).toEqual([]);
  expect(all.find((row) => row.label === "Contacts")?.keys).toEqual([]);

  await page.locator(".palette-input").fill("paper");
  const filtered = await paletteRows(page);
  expect(filtered.map((row) => row.label)).toEqual(["Paper Trail"]);

  await page.keyboard.press("Enter");
  await expect(page.locator(".list-title")).toHaveText("Paper Trail");
  expect(await openDialog(page)).toBeNull();
});

test("the shortcuts sheet is behind the question mark", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("?");
  expect(await openDialog(page)).toBe("Keyboard shortcuts");
  await expect(page.locator(".shortcuts-group").first()).toBeVisible();

  await page.keyboard.press("Escape");
  expect(await openDialog(page)).toBeNull();
});

test("an open panel shadows the view's keys", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("?");
  expect(await openDialog(page)).toBe("Keyboard shortcuts");
  await page.keyboard.press("3");
  await expect(page.locator(".list-title")).toHaveText("Inbox");

  await page.keyboard.press("Escape");
  await page.keyboard.press("3");
  await expect(page.locator(".list-title")).toHaveText("Paper Trail");
});

test("n and p walk the messages and o folds one away", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();

  await expect(page.locator(".msg[data-collapsed]")).toHaveCount(1);
  await page.keyboard.press("Shift+O");
  await expect(page.locator(".msg[data-collapsed]")).toHaveCount(0);

  await page.keyboard.press("n");
  await expect(page.locator(".msg[data-focus]")).toHaveCount(1);
  await page.keyboard.press("o");
  await expect(page.locator(".msg[data-collapsed]")).toHaveCount(1);
});

test("Ctrl and a number switch account", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await expect(page.locator(".account-chip")).toContainText("pj@73ai.org");
  await page.keyboard.press("Control+2");
  await expect(page.locator(".account-chip")).toContainText("priyanshujain@gmail.com");
  await page.keyboard.press("Control+0");
  await expect(page.locator(".account-chip")).toContainText("All accounts");
});
