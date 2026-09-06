// Selection and the bar that comes with it.
//
// The measurement that matters here is the footprint: the action bar takes the piles' place and
// their exact box, because the foot of the list jumping by a few pixels every time a checkbox
// appears is the kind of thing nobody reports and everybody feels.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";
import { actionBar, box, checkedRows, listReady, MIDDAY, openApp, rows, settle, toast } from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

test("x shows the checkbox and hands the piles' footprint to the action bar", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await expect(page.locator(".row-check")).toHaveCount(0);
  const piles = await box(page.locator(".piles"));

  await page.keyboard.press("j");
  await page.keyboard.press("x");

  // The gutter turns into a checkbox on every row, not only the one that was picked.
  await expect(page.locator(".row-check").first()).toBeVisible();
  expect(await checkedRows(page)).toEqual(["Karthik Rao"]);

  await expect(page.locator(".piles")).toHaveCount(0);
  const bar = await box(page.locator(".action-bar"));
  expect(bar.top).toBe(piles.top);
  expect(bar.bottom).toBe(piles.bottom);
  expect(bar.left).toBe(piles.left);
  expect(bar.width).toBe(piles.width);

  // The six verbs that fit, in the order docs/features.md section 13 lists them, each printing its
  // key. The other four in that section keep their keys and lose their buttons, which the bar says
  // in a comment: 420 pixels is three labelled buttons to a row.
  expect(await actionBar(page)).toEqual([
    { label: "Reply later", key: "l" },
    { label: "Set aside", key: "s" },
    { label: "Snooze", key: "b" },
    { label: "Mark seen", key: "u" },
    { label: "Archive", key: "e" },
    { label: "Trash", key: "#" },
  ]);
});

test("Shift+J extends, Cmd+A takes the rest, and Escape gives it all back", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  const all = (await rows(page)).map((row) => row.sender);

  await page.keyboard.press("j");
  await page.keyboard.press("x");
  await page.keyboard.press("Shift+J");
  expect(await checkedRows(page)).toEqual(all.slice(0, 2));

  await page.keyboard.press("Shift+J");
  expect(await checkedRows(page)).toEqual(all.slice(0, 3));

  // Back up the way it came: an extension is a range from the anchor, not a run of toggles.
  await page.keyboard.press("Shift+K");
  expect(await checkedRows(page)).toEqual(all.slice(0, 2));

  await page.keyboard.press("Meta+a");
  expect(await checkedRows(page)).toEqual(all.slice(1));

  await page.keyboard.press("Escape");
  await expect(page.locator(".action-bar")).toHaveCount(0);
  await expect(page.locator(".pile")).toHaveCount(2);
  await expect(page.locator(".row-check")).toHaveCount(0);
});

test("a bulk archive is one toast, one undo and one call", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  const before = (await rows(page)).map((row) => row.sender);

  await page.keyboard.press("j");
  await page.keyboard.press("x");
  await page.keyboard.press("Shift+J");
  await page.keyboard.press("Shift+J");
  await page.keyboard.press("e");

  for (const sender of before.slice(0, 3)) {
    await expect(page.locator(".row", { hasText: sender })).toHaveCount(0);
  }
  await expect(page.locator(".toast")).toHaveCount(1);
  await expect.poll(async () => (await toast(page))?.text).toBe("3 threads archived");

  // The rows are gone, so the selection that pointed at them is gone with them.
  await expect(page.locator(".action-bar")).toHaveCount(0);

  // One `z` puts all three back, which is the only way to see from out here that the three rows
  // went out as one call and came back as one entry on the undo stack.
  await page.keyboard.press("z");
  await expect.poll(async () => (await rows(page)).map((row) => row.sender)).toEqual(before);
});

test("the toast undoes its own action rather than the latest one", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  const before = (await rows(page)).map((row) => row.sender);

  // Two archives in a row: the focus lands on whatever took the first row's place, so the second
  // key archives the row under it without having to be aimed again.
  await page.keyboard.press("j");
  await page.keyboard.press("e");
  await page.keyboard.press("e");
  await expect(page.locator(".toast")).toHaveCount(1);

  // The toast on screen belongs to the second archive and carries its own token, so its button
  // takes back the second and leaves the first exactly where it put it.
  await expect.poll(async () => (await toast(page))?.text).toBe("Archived");
  await page.locator(".toast-action").click();
  await expect(page.locator(".row", { hasText: before[1] })).toHaveCount(1);
  await expect(page.locator(".row", { hasText: before[0] })).toHaveCount(0);
});

test("the list with a selection", async ({ page }) => {
  mkdirSync(shots, { recursive: true });
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("j");
  await page.keyboard.press("x");
  await page.keyboard.press("Shift+J");
  await page.keyboard.press("Shift+J");
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "selection.png") });
});
