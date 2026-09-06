// The ten pictures the in-app guide ships with, taken from the dev fixture rather than from
// anybody's mailbox.
//
// They are committed and they go into the app bundle, so this file writes nothing unless it is
// asked: `pnpm test:ui` runs the whole suite many times a day and a spec that rewrote ten PNGs on
// each of those runs would leave the tree dirty for reasons nobody chose. `just guide-shots` sets
// the variable.
//
// Twice the scale, because the guide draws each picture at half its pixel width, and an element
// rather than the window wherever the picture is about one thing: a guide caption points at a
// palette or a pile, not at a window with a palette somewhere in it.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { bodyText, listReady, MIDDAY, openApp, settle } from "./app";

test.skip(
  () => !process.env.GUIDE_SHOTS,
  "the guide's pictures are committed: run `just guide-shots` to remake them",
);

test.use({ deviceScaleFactor: 2 });

const guide = fileURLToPath(new URL("../public/guide", import.meta.url));

function shot(name: string): string {
  mkdirSync(guide, { recursive: true });
  return join(guide, name);
}

/**
 * The Inbox on the pinned day, in the light palette, which is where every one of these starts.
 *
 * The clock is pinned for the same reason the other specs pin it: the fixture is anchored to the
 * local day, so an unpinned run would ship a picture whose times are whatever hour it was taken at.
 */
async function inbox(page: Page): Promise<void> {
  await openApp(page, { theme: "light", now: MIDDAY() });
  await listReady(page);
}

/** The text is in its real face and the layout has stopped moving. */
async function ready(page: Page): Promise<void> {
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
}

test("inbox.png", async ({ page }) => {
  await inbox(page);

  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await expect(page.locator(".msg-frame").first()).toBeVisible();
  // The bodies are in sandboxed frames, and a frame that has not loaded yet is a white rectangle
  // where the message goes.
  await expect
    .poll(async () => (await bodyText(page)).join(" "), { timeout: 10_000 })
    .toContain("Attached the revised draft");
  await expect(page.locator(".pile")).toHaveCount(2);
  await ready(page);

  // The whole window: the list on the left with the two piles at its foot, the thread on the right.
  await page.screenshot({ path: shot("inbox.png") });
});

test("screener.png", async ({ page }) => {
  await inbox(page);

  await page.keyboard.press("6");
  await expect(page.locator(".screen-card")).toHaveCount(3);
  await expect(page.locator(".screen-actions .button").first()).toBeVisible();
  await ready(page);

  // Three senders waiting, each card carrying Yes, Elsewhere and No with the letter that takes it.
  await page.locator(".screener").screenshot({ path: shot("screener.png") });
});

test("feed.png", async ({ page }) => {
  await inbox(page);

  await page.keyboard.press("2");
  await expect(page.locator(".feed-card").first()).toBeVisible();
  await expect(page.locator(".feed-body").first()).toBeVisible();
  await expect(page.locator(".feed .msg-pending")).toHaveCount(0);
  await expect
    .poll(async () => (await bodyText(page)).join(" "), { timeout: 10_000 })
    .toContain("Good morning.");
  await ready(page);

  // A column of newsletters whose bodies are already open, newest first, with no unread state on
  // any of them.
  await page.locator(".feed").screenshot({ path: shot("feed.png") });
});

test("piles.png", async ({ page }) => {
  await inbox(page);

  await expect(page.locator(".pile")).toHaveCount(2);
  await expect(page.locator(".pile-subject").first()).toBeVisible();
  await ready(page);

  // The foot of the list column on its own: two stacks, each showing the thread on top of it and
  // how many are underneath.
  await page.locator(".piles").screenshot({ path: shot("piles.png") });
});

test("palette.png", async ({ page }) => {
  await inbox(page);

  await page.keyboard.press("Meta+k");
  await expect(page.locator(".palette")).toBeVisible();
  // Typed into, because the groups are the point of the picture and an untouched palette is a
  // screenful of Places with the rest of them below the fold.
  await page.locator(".palette-input").fill("sn");
  await expect(page.locator(".palette-group").nth(1)).toBeVisible();
  await ready(page);

  // One box for everywhere you can go and everything you can run, its rows under the group each
  // belongs to, with the key beside the ones that answer to a key.
  await page.locator(".palette").screenshot({ path: shot("palette.png") });
});

test("compose.png", async ({ page }) => {
  await inbox(page);

  await page.keyboard.press("c");
  await expect(page.locator(".compose")).toBeVisible();
  await page.locator(".compose .chip-input").first().pressSequentially("maya");
  await expect(page.locator(".suggest-row").first()).toBeVisible();
  await page.keyboard.press("Enter");
  await page.locator(".compose-subject").fill("Thursday");
  // The first paragraph rather than the middle of the body: a draft opens with the signature under
  // an empty line, and clicking between the two is a click ProseMirror is entitled to read either
  // way.
  await page.locator(".compose .editor-body > p").first().click();
  await page.keyboard.type("Maya, Thursday works. Church Street at eight?");
  await page.keyboard.press("Enter");
  await page.keyboard.type("I will book it if you do not mind the walk from the station.");
  await ready(page);

  // The card being written in: the recipient as a chip, a subject, two lines of body over the
  // signature, and how long a send is held for.
  await page.locator(".compose").screenshot({ path: shot("compose.png") });
});

test("contact-card.png", async ({ page }) => {
  await inbox(page);

  await page.locator(".row", { hasText: "Piano on Wednesdays" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await settle(page);
  await page.keyboard.press("i");
  await expect(page.locator(".popover-name")).toHaveText("Sam Okafor");
  await expect(page.locator(".contact-waiting")).toHaveCount(0);
  await ready(page);

  // The card over a sender: where their mail delivers, when that was decided, the note kept about
  // them, and what they have written lately.
  await page.locator(".popover").screenshot({ path: shot("contact-card.png") });
});

test("snooze.png", async ({ page }) => {
  await inbox(page);

  await page.keyboard.press("j");
  await page.keyboard.press("b");
  await expect(page.locator(".snooze-option")).toHaveCount(6);
  await ready(page);

  // Six choices, each printing the key that takes it and the moment it actually means.
  await page.locator(".popover").screenshot({ path: shot("snooze.png") });
});

test("shortcuts.png", async ({ page }) => {
  await inbox(page);

  await page.keyboard.press("?");
  await expect(page.locator(".shortcuts-group").first()).toBeVisible();
  await ready(page);

  // The sheet behind the question mark, every binding grouped by what it acts on.
  await page.locator(".sheet").screenshot({ path: shot("shortcuts.png") });
});

test("settings.png", async ({ page }) => {
  await inbox(page);

  await page.keyboard.press("Meta+,");
  await expect(page.locator(".settings")).toBeVisible();
  await page.locator(".settings-tab", { hasText: "Appearance" }).click();
  await expect(page.locator('[role="tablist"][aria-label="Theme"]')).toBeVisible();
  await ready(page);

  // The rail of sections down the left with one of them open beside it, which is the shape of
  // every section.
  await page.locator(".settings").screenshot({ path: shot("settings.png") });
});
