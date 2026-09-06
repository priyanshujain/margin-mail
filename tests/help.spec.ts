// The question mark in the corner and the small menu behind it.
//
// It is the only thing the app draws over every screen and it sits in the corner the compose card
// already owns, so most of what is asserted here is negative: it covers none of the pane's verbs,
// it does not run off the window, and it is gone the moment the card wants the corner. The menu
// opens upwards, which is the whole reason the Popover primitive learned a second placement, and
// the only honest way to check it is to measure the panel against the viewport.

import { expect, test, type Page } from "@playwright/test";
import { box, listReady, MIDDAY, openApp, openDialog, openRow, settle } from "./app";

const launcher = (page: Page) => page.locator(".help-launcher");
const button = (page: Page) => page.locator(".help-launcher .button");
const menu = (page: Page) => page.locator(".help-menu");

async function openMenu(page: Page): Promise<void> {
  await button(page).click();
  await expect(menu(page)).toBeVisible();
  await settle(page);
}

/** The rows as somebody reads them: the label, and the key printed on the right where there is one. */
function menuRows(page: Page): Promise<{ label: string; key: string }[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>(".help-option")].map((el) => ({
      label: (el.querySelector(".help-label")?.textContent ?? "").trim(),
      key: (el.querySelector(".key")?.textContent ?? "").trim(),
    })),
  );
}

test("the question mark sits in the bottom right corner and covers none of the pane", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  // With a thread open the pane has its bar of verbs, which is what there is to be in the way of.
  await openRow(page, 0);

  const viewport = page.viewportSize()!;
  const corner = await box(launcher(page));

  expect(Math.round(viewport.width - corner.right)).toBe(20);
  expect(Math.round(viewport.height - corner.bottom)).toBe(20);
  expect(corner.right).toBeLessThanOrEqual(viewport.width);
  expect(corner.bottom).toBeLessThanOrEqual(viewport.height);

  // Round, and the size of the app's other icon-only controls rather than a bubble.
  expect(corner.width).toBe(corner.height);
  expect(corner.width).toBeGreaterThanOrEqual(26);
  expect(corner.width).toBeLessThanOrEqual(44);

  const verbs = await page.locator(".pane .button").count();
  expect(verbs).toBeGreaterThan(0);

  const covered = await page.evaluate(() => {
    const help = document.querySelector(".help-launcher")!.getBoundingClientRect();
    return [...document.querySelectorAll<HTMLElement>(".pane .button")]
      .filter((el) => {
        const r = el.getBoundingClientRect();
        return (
          r.width > 0 &&
          r.left < help.right &&
          r.right > help.left &&
          r.top < help.bottom &&
          r.bottom > help.top
        );
      })
      .map((el) => (el.textContent ?? "").replace(/\s+/g, " ").trim());
  });
  expect(covered).toEqual([]);
});

test("the menu opens above the button and is wholly on the screen", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openMenu(page);

  const viewport = page.viewportSize()!;
  const corner = await box(launcher(page));
  const panel = await box(page.locator(".popover"));

  // Above the button, which is what the top-end placement is for: hung below it, a panel this tall
  // would start 20px from the bottom edge and every row of it would be past the window.
  expect(panel.bottom).toBeLessThanOrEqual(corner.top);
  expect(panel.top).toBeGreaterThan(0);
  expect(panel.bottom).toBeLessThanOrEqual(viewport.height);
  expect(panel.right).toBeLessThanOrEqual(viewport.width);
  expect(panel.left).toBeGreaterThan(0);

  expect(await menuRows(page)).toEqual([
    { label: "Take the tour", key: "" },
    { label: "Guide", key: "" },
    { label: "Keyboard shortcuts", key: "?" },
  ]);
});

test("Keyboard shortcuts opens the sheet", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openMenu(page);

  await page.locator(".help-option", { hasText: "Keyboard shortcuts" }).click();

  expect(await openDialog(page)).toBe("Keyboard shortcuts");
  await expect(menu(page)).toHaveCount(0);
  // The sheet has the window, so the corner button is not floating over its scrim.
  await expect(launcher(page)).toHaveCount(0);
});

test("Guide opens the guide over the window", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openMenu(page);

  await page.locator(".help-option", { hasText: "Guide" }).click();

  await expect.poll(() => openDialog(page)).toBe("Guide");
  await expect(page.locator(".guide-title")).toBeVisible();
  // Offering the way there again from on top of it would say nothing, and the button would be
  // under the scrim anyway.
  await expect(launcher(page)).toHaveCount(0);
});

test("Take the tour opens the tour", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openMenu(page);

  await page.locator(".help-option", { hasText: "Take the tour" }).click();

  expect(await openDialog(page)).toBe("Getting started");
  await expect(menu(page)).toHaveCount(0);
});

test("the compose card takes the corner back, and gives it up again", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await expect(launcher(page)).toBeVisible();

  await page.keyboard.press("c");
  await expect(page.locator(".compose")).toBeVisible();
  await expect(launcher(page)).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(page.locator(".compose")).toHaveCount(0);
  await expect(launcher(page)).toBeVisible();
});

test("Escape closes the menu and leaves the app where it was", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openRow(page, 0);
  const subject = await page.locator(".thread-subject").textContent();

  await openMenu(page);
  await page.keyboard.press("Escape");

  await expect(menu(page)).toHaveCount(0);
  await expect(launcher(page)).toBeVisible();
  expect(await openDialog(page)).toBeNull();
  await expect(page.locator(".thread-subject")).toHaveText(subject ?? "");
});
