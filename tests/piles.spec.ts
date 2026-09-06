// The two piles, and the page that is a walk down one of them.
//
// A pile is its own query rather than a slice of the list, so the assertions here are deliberately
// about both at once: the row has to leave the Inbox and the card at the foot has to be showing it,
// in the same frame, before the fixture has answered.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { box, listReady, MIDDAY, openApp, rows, settle, toast } from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

/** Focuses the row a spec means to act on, by walking to it rather than by clicking it open. */
async function focusRow(page: Page, sender: string): Promise<void> {
  for (let i = 0; i < 20; i++) {
    const at = (await rows(page)).find((row) => row.selected);
    if (at?.sender === sender) return;
    await page.keyboard.press("j");
  }
  throw new Error(`never reached ${sender}`);
}

const pile = (page: Page, index: number) => page.locator(".pile").nth(index);

test("the piles show their top thread and how many are under it", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const later = pile(page, 0);
  await expect(later).toContainText("Reply later");
  await expect(later.locator(".pile-subject")).toHaveText("Re: Cooper's parent-teacher conference");
  await expect(later.locator(".pile-who")).toHaveText("Jeff Wolfe, and 2 more");

  const aside = pile(page, 1);
  await expect(aside).toContainText("Set aside");
  await expect(aside.locator(".pile-subject")).toHaveText("Les Misérables tickets");
  await expect(aside.locator(".pile-who")).toHaveText("Caroline Bauhaus, and 1 more");

  // Three threads on the left and two on the right, which is what the edges behind the card say.
  await expect(later.locator(".pile-edge")).toHaveCount(2);
  await expect(aside.locator(".pile-edge")).toHaveCount(1);

  // The stack stays inside the footprint the action bar takes over.
  const stack = await box(later);
  const card = await box(later.locator(".pile-card"));
  expect(card.bottom).toBe(stack.bottom);
  expect(card.top).toBeGreaterThan(stack.top);
});

test("l takes the thread out of the Inbox and puts it on the pile, and l again brings it back", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("l");

  // The row is gone and the card is showing it before the fixture has answered: the call goes
  // behind the change, not in front of it.
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(0);
  await expect(pile(page, 0).locator(".pile-subject")).toHaveText("Dinner on Thursday?");
  await expect(pile(page, 0).locator(".pile-who")).toHaveText("Maya Raghunathan, and 3 more");
  await expect.poll(async () => (await toast(page))?.text).toBe("Moved to Reply later");
  expect((await toast(page))?.action).toContain("Undo");

  // And it is really in the place behind the pile, at the top of it.
  await page.keyboard.press("4");
  await expect(page.locator(".list-title")).toHaveText("Reply later");
  await expect.poll(async () => (await rows(page))[0]?.subject).toBe("Dinner on Thursday?");

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("l");
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toBe("Taken out of Reply later");

  await page.keyboard.press("1");
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(1);
  await expect(pile(page, 0).locator(".pile-subject")).toHaveText(
    "Re: Cooper's parent-teacher conference",
  );
});

test("the toast undoes the pile it is about", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  const before = (await rows(page)).map((row) => row.sender);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("s");
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(0);
  await expect(pile(page, 1).locator(".pile-subject")).toHaveText("Dinner on Thursday?");
  await expect.poll(async () => (await toast(page))?.text).toBe("Moved to Set aside");

  await page.locator(".toast-action").click();
  await expect.poll(async () => (await rows(page)).map((row) => row.sender)).toEqual(before);
  await expect(pile(page, 1).locator(".pile-subject")).toHaveText("Les Misérables tickets");
  // And it says so, the way `z` does, with nothing left on it to press twice.
  await expect.poll(async () => (await toast(page))?.text).toBe("Undone: Moved to Set aside");
  expect((await toast(page))?.action).toBeNull();
});

test("clicking a pile opens its place", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await pile(page, 1).click();
  await expect(page.locator(".list-title")).toHaveText("Set aside");
  await expect.poll(async () => (await rows(page)).map((row) => row.subject)).toEqual([
    "Les Misérables tickets",
    "Weekend in Coorg, places to stay",
  ]);
});

test("Shift+F is one item per Reply later thread, Tab moves and Escape leaves", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("Shift+F");
  await expect(page.locator(".focus-title")).toHaveText("Focus & Reply");

  const items = page.locator(".focus-item");
  await expect(items).toHaveCount(3);
  await expect(page.locator(".focus-hint")).toContainText("3 left");
  await expect(items.nth(0).locator(".focus-subject")).toHaveText(
    "Re: Cooper's parent-teacher conference",
  );
  await expect(items.nth(0).locator(".focus-reply-head")).toContainText("Reply to Jeff Wolfe");

  // The item the keyboard is on takes the ring, and Tab hands it on.
  await expect(items.nth(0)).toHaveAttribute("data-active", "");
  await page.keyboard.press("Tab");
  await expect(items.nth(1)).toHaveAttribute("data-active", "");
  await page.keyboard.press("Shift+Tab");
  await expect(items.nth(0)).toHaveAttribute("data-active", "");

  // The box is real and holds what is typed into it.
  await items.nth(0).locator(".focus-box").fill("Wednesday at 4pm in person would suit us.");
  await page.keyboard.press("Tab");
  await expect(items.nth(0).locator(".focus-box")).toHaveValue(
    "Wednesday at 4pm in person would suit us.",
  );

  await page.keyboard.press("Escape");
  await expect(page.locator(".focus")).toHaveCount(0);
  await expect(page.locator(".list-title")).toHaveText("Inbox");

  // And a thread that was skipped is still on the pile, which is the whole promise of skipping.
  await expect(pile(page, 0).locator(".pile-who")).toHaveText("Jeff Wolfe, and 2 more");
});

test("the Reply later place offers the page over it", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("4");
  await page.locator(".pill", { hasText: "Focus & Reply" }).click();
  await expect(page.locator(".focus-title")).toHaveText("Focus & Reply");
});

test("the piles and the page they lead to, for the eye", async ({ page }) => {
  mkdirSync(shots, { recursive: true });
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "piles.png") });

  await page.keyboard.press("Shift+F");
  await expect(page.locator(".focus-item").first()).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "focus-reply.png") });
});
