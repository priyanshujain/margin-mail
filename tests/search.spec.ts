// Search, which is not a place you go to but a thing that takes the stage and gives it back.
//
// The two assertions worth making are the ones a store on its own cannot promise: that the results
// behave like a list (the pane still opens a thread from them) and that Escape puts back exactly
// what was there, scroll and selection included.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";
import { checkedRows, listReady, listScroll, MIDDAY, openApp, rows, settle } from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

const senders = (page: import("@playwright/test").Page) =>
  rows(page).then((all) => all.map((row) => row.sender));

test("/ focuses search, and Escape gives the Inbox back with its scroll and its selection", async ({
  page,
}) => {
  // Shorter than the suite's default on purpose. What is being proved is that Escape puts the
  // scroll back, which needs a list with somewhere to scroll to, and tying that to the number of
  // rows that happen to fit at the current row height is how this test broke when the row height
  // changed.
  await page.setViewportSize({ width: 1440, height: 560 });
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("j");
  await page.keyboard.press("x");
  await page.locator(".list").evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await settle(page);
  const parked = await listScroll(page);
  expect(parked).toBeGreaterThan(0);

  await page.keyboard.press("/");
  expect(await page.evaluate(() => document.activeElement?.className)).toBe("search-input");

  await page.locator(".search-input").fill("lisbon");
  await expect(page.locator(".list-title")).toHaveText("Search");
  await expect.poll(() => senders(page)).toEqual(["Airbnb"]);
  await expect(page.locator(".search-note")).toHaveText(
    "Searching what is on this device. Older mail is on Gmail.",
  );

  // The pane works as usual over the results.
  await page.locator(".row").first().click();
  await expect(page.locator(".thread-subject")).toHaveText("Your reservation in Lisbon is confirmed");

  await page.keyboard.press("Escape");
  await expect(page.locator(".list-title")).toHaveText("Inbox");
  await expect.poll(() => listScroll(page)).toBeGreaterThan(parked - 4);
  expect(await listScroll(page)).toBeLessThan(parked + 4);
  expect(await checkedRows(page)).toEqual(["Karthik Rao"]);
});

test("search reaches spam, and the row says which place it came out of", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  // The message you most need to find is the one something else decided you should not see, so a
  // search with no `in:` in it reaches spam. Nothing in the Inbox would have found this.
  await page.keyboard.press("/");
  await page.locator(".search-input").fill("verification");
  await expect.poll(() => senders(page)).toEqual(["Aćcount Security"]);
  await expect(page.locator(".row .row-mark[title='Spam']")).toHaveCount(1);

  // And `in:` narrows to one place when that is what was meant.
  await page.locator(".search-input").fill("notice in:trash");
  await expect.poll(() => senders(page)).toEqual(["Parcel Notice"]);
  await expect(page.locator(".row .row-mark[title='Trash']")).toHaveCount(1);
});

test("an operator in the query reads as one", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("/");
  await page.locator(".search-input").fill("from:maya dinner");
  await expect.poll(() => senders(page)).toEqual(["Maya Raghunathan"]);

  // The query is printed at the head of its results, with the operator drawn as one.
  await expect(page.locator(".search-terms .pill")).toHaveText("from:maya");
  await expect(page.locator(".search-terms .search-word")).toHaveText("dinner");
});

test("the foot of a short answer offers the provider's search", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("/");
  await page.locator(".search-input").fill("lease");
  await expect.poll(() => senders(page)).not.toEqual([]);

  const older = page.locator(".search-foot .button", { hasText: "Search older mail on Gmail" });
  await expect(older).toBeVisible();
  await older.click();

  // Asking the provider replaces the note with what came back, and the offer is not made twice.
  await expect(page.locator(".search-note")).toContainText("from Gmail");
  await expect(older).toHaveCount(0);
});

test("the field says it is looking until the answer lands", async ({ page }) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);

  await page.keyboard.press("/");
  await expect(page.locator(".search")).toHaveAttribute("data-phase", "idle");
  await page.locator(".search-input").fill("lisbon");
  // The Inbox's rows are still under the Search title for most of a second, and the phase on the
  // field is the one thing that says they are not the answer yet.
  await expect(page.locator(".search")).toHaveAttribute("data-phase", "searching");
  await expect.poll(() => senders(page)).toEqual(["Airbnb"]);
  await expect(page.locator(".search")).toHaveAttribute("data-phase", "idle");
});

test("search results", async ({ page }) => {
  mkdirSync(shots, { recursive: true });
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("/");
  await page.locator(".search-input").fill("from:arun lease");
  await expect.poll(() => senders(page)).toEqual(["Arun Kulkarni"]);
  await page.locator(".row").first().click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "search.png") });
});
