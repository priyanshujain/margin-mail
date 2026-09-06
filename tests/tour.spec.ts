// The tour, driven the two ways somebody meets it: after the panel the first run ends on, and from
// the palette every time after that.
//
// The keycap assertions never type a letter. What a slide prints is compared with what the app
// prints for the same verb somewhere else, on the Screener's own buttons and in the palette, and
// both of those are generated from the binding table. So a remap moves all of them together and a
// letter written into the tour's copy fails here.

import { expect, test, type Page } from "@playwright/test";
import { MIDDAY, listReady, openApp, openDialog, paletteRows, settle } from "./app";

const TITLE = "Getting started";

const dialog = (page: Page) => page.getByRole("dialog", { name: TITLE });
const heading = (page: Page) => page.locator(".tour-title");

/** Which slide is up, read off the DOM rather than asked of the app. */
function slide(page: Page): Promise<number> {
  return page.evaluate(() =>
    Number(document.querySelector(".tour")?.getAttribute("data-slide") ?? -1),
  );
}

/** Which dot is filled, which has to be the same answer. */
function dot(page: Page): Promise<number> {
  return page.evaluate(() =>
    [...document.querySelectorAll(".tour-dot")].findIndex((el) => el.hasAttribute("data-on")),
  );
}

/** The keycaps in the slide's copy, in the order they are read. */
function caps(page: Page, selector: string): Promise<string[]> {
  return page.evaluate(
    (sel) => [...document.querySelectorAll(sel)].map((el) => (el.textContent ?? "").trim()),
    selector,
  );
}

/** The palette route: the only way in once the first run is over. */
async function openTour(page: Page): Promise<void> {
  await page.keyboard.press("Meta+k");
  await page.locator(".palette-input").fill("tour");
  await page.keyboard.press("Enter");
  await expect(dialog(page)).toBeVisible();
  await settle(page);
}

test("the palette opens the tour, on the first slide", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("Meta+k");
  await page.locator(".palette-input").fill("tour");
  // One row matches, so Return runs it rather than whatever happened to be first.
  expect((await paletteRows(page)).map((row) => row.label)).toEqual(["Take the tour"]);
  await page.keyboard.press("Enter");

  expect(await openDialog(page)).toBe(TITLE);
  expect(await slide(page)).toBe(0);
  await expect(heading(page)).toHaveText("Nobody new reaches you until you say so");
  await expect(page.locator(".tour-dot")).toHaveCount(9);
  expect(await dot(page)).toBe(0);

  // The primary control holds the focus, which is the whole of why Return and Space advance.
  await expect(page.locator(".tour-foot .button[data-variant='primary']")).toBeFocused();
  await expect(page.getByRole("button", { name: "Back" })).toBeDisabled();
});

test("the arrows, the buttons and the dots all move it", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openTour(page);

  await page.keyboard.press("ArrowRight");
  expect(await slide(page)).toBe(1);
  await expect(heading(page)).toHaveText("Three boxes, not one inbox");

  await page.getByRole("button", { name: "Next" }).click();
  expect(await slide(page)).toBe(2);
  await expect(heading(page)).toHaveText("Reply later and Set aside, instead of flags");

  // The click left the focus on Next, so Return is the same press without the mouse.
  await page.keyboard.press("Enter");
  expect(await slide(page)).toBe(3);

  await page.getByRole("button", { name: "Back" }).click();
  expect(await slide(page)).toBe(2);
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  expect(await slide(page)).toBe(0);

  // Nowhere before the first slide, and the panel stays up rather than closing on the way past it.
  await page.keyboard.press("ArrowLeft");
  expect(await slide(page)).toBe(0);
  await expect(dialog(page)).toBeVisible();

  // The dots track it, and they jump.
  await page.locator(".tour-dot").nth(6).click();
  expect(await slide(page)).toBe(6);
  expect(await dot(page)).toBe(6);
  await expect(heading(page)).toHaveText("The things kept beside the mail");
});

test("Skip closes it, and so does Escape", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await openTour(page);
  await page.getByRole("button", { name: "Skip" }).click();
  expect(await openDialog(page)).toBeNull();

  // And the way out of everything else in the app is the way out of this.
  await openTour(page);
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Escape");
  expect(await openDialog(page)).toBeNull();

  // Reopening starts at the beginning rather than where it was left.
  await openTour(page);
  expect(await slide(page)).toBe(0);
});

test("the last slide ends it", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openTour(page);

  await page.locator(".tour-dot").last().click();
  expect(await slide(page)).toBe(8);
  await expect(heading(page)).toHaveText("Undo covers everything");
  await expect(page.getByRole("button", { name: "Next" })).toHaveCount(0);

  await page.getByRole("button", { name: "Done" }).click();
  expect(await openDialog(page)).toBeNull();
});

test("it follows the first-run panel", async ({ page }) => {
  await openApp(page, { firstRun: true });
  await page.getByLabel("Email address").fill("pj@gmail.com");
  await page.keyboard.press("Enter");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.locator(".row").first()).toBeVisible();

  const setUp = page.getByRole("dialog", { name: "You are set up" });
  await expect(setUp).toBeVisible();
  await page.getByRole("button", { name: "Done" }).click();
  await expect(setUp).toHaveCount(0);

  // The one moment the app has somebody's attention, so the tour is what the panel hands over to.
  await expect(dialog(page)).toBeVisible();
  expect(await slide(page)).toBe(0);
});

test("every keycap is the one the binding table gives that verb", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  // The Screener's three buttons, which print their keys the way every button in the app does.
  await page.keyboard.press("6");
  await expect(page.locator(".screen-card").first()).toBeVisible();
  const answers = (
    await page.locator(".screen-card").first().locator(".screen-actions .key").allTextContents()
  ).map((text) => text.trim());
  expect(answers).toHaveLength(3);

  await page.keyboard.press("1");
  await listReady(page);
  await page.keyboard.press("Meta+k");
  const rows = await paletteRows(page);
  await page.keyboard.press("Escape");
  const undo = rows.find((row) => row.label === "Undo")?.keys ?? [];
  const sheet = rows.find((row) => row.label === "Keyboard shortcuts")?.keys ?? [];
  expect(undo).toHaveLength(1);
  expect(sheet).toHaveLength(1);

  await openTour(page);
  // Yes, Elsewhere and No, in the card's order, in the figure and again in the sentence.
  expect(await caps(page, ".tour-choices .key")).toEqual(answers);
  expect(await caps(page, ".tour-copy .key")).toEqual(answers);

  await page.locator(".tour-dot").nth(4).click();
  await expect(heading(page)).toHaveText("One key per verb");
  expect(await caps(page, ".tour-copy .key")).toEqual(sheet);

  await page.locator(".tour-dot").last().click();
  expect(await caps(page, ".tour-copy .key")).toEqual(undo);
});
