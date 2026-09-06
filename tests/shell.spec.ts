// The window: the header, the list column and the two piles. What a person looks at before they
// have read anything.
//
// Everything here is measured off the page rather than asked of the app. The 420px column and the
// 46px row are the two numbers the whole layout is built from, and they are the two the mockups
// pin down.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";
import {
  box,
  groups,
  listReady,
  MIDDAY,
  openApp,
  paletteRows,
  rows,
  settle,
  toast,
  token,
} from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

test("the list column is 420px and a row is 46px", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  expect(await token(page, "--list-w")).toBe("420px");
  expect(await token(page, "--row-h")).toBe("46px");

  const column = await box(page.locator(".list-col"));
  expect(column.width).toBe(420);

  const first = (await rows(page))[0];
  expect(first.height).toBe(46);
});

test("a row carries sender and time over subject and snippet", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const maya = (await rows(page)).find((row) => row.sender === "Maya Raghunathan");
  expect(maya).toBeTruthy();
  expect(maya!.subject).toBe("Dinner on Thursday?");
  expect(maya!.snippet).toContain("Church Street");
  expect(maya!.time).toBe("11:42");
  // More than one message in the thread, so the count sits between the sender and the time.
  expect(maya!.count).toBe("3");

  // Two lines, in that order: the sender's line above the subject's.
  const lines = await page.evaluate(() => {
    const row = [...document.querySelectorAll<HTMLElement>(".row")].find((el) =>
      el.textContent?.includes("Maya Raghunathan"),
    )!;
    const rect = (selector: string) => row.querySelector(selector)!.getBoundingClientRect();
    return {
      senderTop: rect(".row-sender").top,
      timeTop: rect(".row-time").top,
      subjectTop: rect(".row-subject").top,
      snippetTop: rect(".row-snippet").top,
    };
  });
  expect(Math.abs(lines.senderTop - lines.timeTop)).toBeLessThan(4);
  expect(Math.abs(lines.subjectTop - lines.snippetTop)).toBeLessThan(4);
  expect(lines.subjectTop).toBeGreaterThan(lines.senderTop);
});

test("the Inbox puts New for you above Previously seen, and only new mail has the dot", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const heads = await groups(page);
  expect(heads).toContain("New for you");
  expect(heads).toContain("Previously seen");
  expect(heads.indexOf("New for you")).toBeLessThan(heads.indexOf("Previously seen"));

  const list = await rows(page);
  const arun = list.find((row) => row.sender === "Arun Kulkarni")!;
  const lena = list.find((row) => row.sender === "Lena Brandt")!;
  expect(arun.group).toBe("New for you");
  expect(arun.unseen).toBe(true);
  expect(lena.group).toBe("Previously seen");
  expect(lena.unseen).toBe(false);
  // A note on a thread is one line under its row, on the note surface.
  expect(lena.note).toBe("Ask about the oak finish before confirming");
});

test("the two piles sit at the foot of the list with their keys", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const piles = page.locator(".pile");
  await expect(piles).toHaveCount(2);
  await expect(piles.first()).toContainText("Reply later");
  await expect(piles.first()).toContainText("4");
  await expect(piles.nth(1)).toContainText("Set aside");
  await expect(piles.nth(1)).toContainText("5");

  const list = await box(page.locator(".list-col"));
  const pile = await box(piles.first());
  expect(pile.bottom).toBeLessThanOrEqual(list.bottom + 1);
});

test("the account chip opens the switcher, with All accounts behind it", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.locator(".account-chip").click();
  const options = page.locator(".account-option");
  await expect(options).toHaveCount(3);
  await expect(options.nth(0)).toContainText("pj@73ai.org");
  await expect(options.nth(2)).toContainText("All accounts");

  await options.nth(1).click();
  await expect(page.locator(".account-chip")).toContainText("priyanshujain@gmail.com");
});

test("Everything closes with the one line about the window", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("0");
  await expect(page.locator(".list-title")).toHaveText("Everything");
  await page.locator(".list").evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await expect(page.locator(".list-foot")).toHaveText(
    "Showing the last month. Older mail is on Gmail.",
  );
});

test("an empty place says one quiet thing", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  // The second account has nothing in either pile, which is the only empty list the fixture holds.
  // An install with no accounts at all is the connect screen now, not an empty Inbox.
  await page.keyboard.press("Control+2");
  await page.keyboard.press("4");
  await expect(page.locator(".list-title")).toHaveText("Reply later");
  await expect(page.locator(".list .empty-state")).toHaveText("Nothing here");
  await expect(page.locator(".row")).toHaveCount(0);
});

test("the Other group in the palette is the way into the three you go looking in", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("Meta+k");
  const other = (await paletteRows(page)).filter((row) => row.group === "Other");
  expect(other.map((row) => row.label)).toEqual(["Screened out", "Spam", "Trash"]);
  // None of the three prints a key, because none of them has one: where a place sits in this list
  // is the honest statement of how often you should be in it.
  expect(other.flatMap((row) => row.keys)).toEqual([]);

  // And each one is a place with a list of its own behind it. Trash and Spam say what Gmail will
  // do with them, which is the line that stands in for the Empty button this app cannot offer.
  await page.locator(".palette-input").fill("trash");
  await page.keyboard.press("Enter");
  await expect(page.locator(".list-title")).toHaveText("Trash");
  await expect(page.locator(".list-foot")).toHaveText("Gmail empties this after 30 days.");

  await page.keyboard.press("Meta+k");
  await page.locator(".palette-input").fill("screened");
  await page.keyboard.press("Enter");
  await expect(page.locator(".list-title")).toHaveText("Screened out");
  await expect(page.locator(".list-foot")).toHaveCount(0);
});

test("Sync now says so in the header while the pass is out, and nothing when it is back", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await expect(page.locator(".sync-busy")).toHaveCount(0);

  await page.keyboard.press("Meta+k");
  await page.locator(".palette-input").fill("sync now");
  await page.keyboard.press("Enter");

  // An ordinary pass has no sentence of its own, so the ask itself is the line.
  await expect(page.locator(".sync-busy")).toHaveText("Checking for mail");
  // And nothing at the end of it: a pass that found nothing new is not news.
  await expect(page.locator(".sync-busy")).toHaveCount(0, { timeout: 5_000 });
  expect(await toast(page)).toBeNull();
});

test("the app renders in dark", async ({ page }) => {
  await openApp(page, { theme: "dark", now: MIDDAY() });
  await listReady(page);

  expect(await page.evaluate(() => document.documentElement.getAttribute("data-theme"))).toBe(
    "dark",
  );
  expect(await token(page, "--paper")).toBe("#1d1a16");
  await expect(page.locator(".row").first()).toBeVisible();
});

// One picture per test rather than a loop, because `openApp` seeds the store once per context and
// a second call on the same page would take the dark picture in the light palette.
for (const theme of ["light", "dark"] as const) {
  test(`the Inbox in ${theme}`, async ({ page }) => {
    mkdirSync(shots, { recursive: true });
    await openApp(page, { theme, now: MIDDAY() });
    await listReady(page);
    await page.evaluate(() => document.fonts.ready);
    // The lease thread is the one the mockup has open, and it is the second row of New for you.
    await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
    await expect(page.locator(".thread-subject")).toBeVisible();
    await settle(page);
    await page.screenshot({ path: join(shots, `inbox-${theme}.png`) });
  });
}

test("the palette", async ({ page }) => {
  mkdirSync(shots, { recursive: true });
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await page.keyboard.press("Meta+k");
  await expect(page.locator(".palette")).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "palette.png") });
});
