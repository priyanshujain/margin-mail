// The contact card and the Contacts place.
//
// The card is where a decision about a sender is reversed, so most of what is asserted here is that
// a control changed something and that the change is still there after the card has been closed and
// asked for again. Nothing reads the store: a destination is what the picker says, and a note is
// what the field holds after a round trip through the backend.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { MIDDAY, failCommands, listReady, openApp, openDialog, settle, toast } from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

const SAM = "sam@sunnydaymusic.example";
const MAYA = "maya.raghunathan@example.com";

const card = (page: Page) => page.locator(".popover");

/** Opens a thread and asks for its sender's card the way the keyboard does. */
async function cardFor(page: Page, subject: string): Promise<void> {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: subject }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await settle(page);
  await page.keyboard.press("i");
  await expect(card(page)).toBeVisible();
}

test("i on a focused thread opens the card for its sender", async ({ page }) => {
  await cardFor(page, "Piano on Wednesdays");

  expect(await openDialog(page)).toBe("Contact card");
  await expect(card(page).locator(".popover-name")).toHaveText("Sam Okafor");
  await expect(card(page).locator(".popover-sub")).toHaveText(SAM);
  // Where their mail delivers, and when that was decided. Both are the whole point of the card.
  await expect(card(page).locator(".contact-pick")).toHaveValue("inbox");
  await expect(card(page).getByText(/^In, on /)).toBeVisible();
  // The two lists at its foot are the sender's own.
  await expect(card(page).getByText("Recent threads")).toBeVisible();
});

test("changing Delivers to writes it and moves the mail that is already here", async ({ page }) => {
  await cardFor(page, "Piano on Wednesdays");

  await card(page).locator(".contact-pick").selectOption("paper-trail");
  await expect(card(page).locator(".contact-pick")).toHaveValue("paper-trail");

  // A move that only affected future mail would read as a move that did not work, so the thread
  // that was in the Inbox is in the Paper Trail now.
  await page.keyboard.press("Escape");
  await expect(card(page)).toBeHidden();
  await page.keyboard.press("3");
  await expect(page.locator(".list-title")).toHaveText("Paper Trail");
  const moved = page.locator(".row", { hasText: "Piano on Wednesdays" });
  await expect(moved).toBeVisible();

  // Closed and asked for again, which is the only way to tell a written change from a drawn one.
  await moved.click();
  await page.keyboard.press("i");
  await expect(card(page).locator(".contact-pick")).toHaveValue("paper-trail");
});

test("the domain toggle is offered for a company and not for a consumer address", async ({
  page,
}) => {
  await cardFor(page, "Piano on Wednesdays");
  await expect(card(page).getByRole("switch", { name: "Everyone at sunnydaymusic.example" })).toBeVisible();

  await page.keyboard.press("Escape");
  await page.locator(".row", { hasText: "Dinner on Thursday?" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await page.keyboard.press("i");
  await expect(card(page).locator(".popover-sub")).toHaveText(MAYA);

  // Everyone at gmail.com is not one sender, so the toggle is not offered rather than offered and
  // then refused.
  await expect(card(page).getByRole("switch", { name: /^Everyone at/ })).toHaveCount(0);
  await expect(card(page).getByRole("switch", { name: "Notify" })).toBeVisible();
});

test("Notify and the note round trip", async ({ page }) => {
  await cardFor(page, "Piano on Wednesdays");

  const notify = card(page).getByRole("switch", { name: "Notify" });
  const was = await notify.getAttribute("aria-checked");
  await notify.click();
  await expect(notify).toHaveAttribute("aria-checked", was === "true" ? "false" : "true");

  const note = card(page).getByLabel("Note");
  await expect(note).toHaveValue("Cooper's teacher. Prefers email over calls.");
  await note.fill("Lessons moved to Thursdays");
  await note.blur();

  await page.keyboard.press("Escape");
  await expect(card(page)).toBeHidden();
  await page.keyboard.press("i");
  await expect(card(page).getByLabel("Note")).toHaveValue("Lessons moved to Thursdays");
  await expect(card(page).getByRole("switch", { name: "Notify" })).toHaveAttribute(
    "aria-checked",
    was === "true" ? "false" : "true",
  );
});

test("Escape closes the card and leaves the thread where it was", async ({ page }) => {
  await cardFor(page, "Piano on Wednesdays");

  await page.keyboard.press("Escape");

  await expect(card(page)).toBeHidden();
  expect(await openDialog(page)).toBeNull();
  // The card is a layer over the thread, so unwinding it gives the thread back rather than closing
  // it too.
  await expect(page.locator(".thread-subject")).toBeVisible();
});

test("the Contacts place lists senders with rules and its search narrows them", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("Meta+k");
  await page.locator(".palette-input").fill("contacts");
  await page.keyboard.press("Enter");

  await expect(page.locator(".app")).toHaveAttribute("data-stage", "contacts");
  await expect(page.locator(".list-title")).toHaveText("Contacts");

  const senders = page.locator(".contact-item .row-sender");
  await expect(page.locator(".contact-item", { hasText: "Sam Okafor" })).toBeVisible();
  await expect.poll(() => senders.count()).toBeGreaterThan(5);
  // The row is the mail list's row, which is what "the same anatomy" means.
  await expect(page.locator(".contact-item").first().locator(".avatar")).toBeVisible();

  await page.getByLabel("Search contacts").fill("sam");
  await expect(senders).toHaveCount(1);
  await expect(senders.first()).toHaveText("Sam Okafor");

  // The card's controls are on the row, so a decision can be changed without opening anything.
  const row = page.locator(".contact-item").first();
  await row.locator(".contact-pick").selectOption("paper-trail");
  await expect(row.locator(".contact-pick")).toHaveValue("paper-trail");

  await page.getByLabel("Search contacts").fill("nobodyatall");
  await expect(page.locator(".contact-item")).toHaveCount(0);
  await expect(page.locator(".empty-state")).toBeVisible();
});

test("a row in the Contacts place opens the card", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("Meta+k");
  await page.locator(".palette-input").fill("contacts");
  await page.keyboard.press("Enter");
  await expect(page.locator(".contact-item").first()).toBeVisible();

  await page.locator(".contact-item", { hasText: "Sam Okafor" }).locator(".row").click();
  await expect(card(page)).toBeVisible();
  await expect(card(page).locator(".popover-sub")).toHaveText(SAM);
});

test("the card draws its wait, and the wait goes when the card comes", async ({ page }) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await page.locator(".row", { hasText: "Piano on Wednesdays" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await page.keyboard.press("i");

  await expect(card(page).locator(".contact-waiting")).toHaveAttribute("data-phase", "loading");
  await expect(card(page).locator(".popover-name")).toHaveText("Sam Okafor");
  await expect(card(page).locator(".contact-waiting")).toHaveCount(0);
});

test("a card that will not come says so and does not stay open empty", async ({ page }) => {
  await failCommands(page, ["contact_card"]);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: "Piano on Wednesdays" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await page.keyboard.press("i");

  await expect.poll(async () => (await toast(page))?.text).toContain("Could not open that contact");
  await expect(card(page)).toHaveCount(0);
});

test("the Contacts place keeps what it had while the next answer is out", async ({ page }) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await page.keyboard.press("Meta+k");
  await page.locator(".palette-input").fill("contacts");
  await page.keyboard.press("Enter");

  // Before any answer: the row's shape rather than nothing, and not a claim that there is nobody.
  const list = page.locator(".contacts-list");
  await expect(list).toHaveAttribute("data-phase", "loading");
  await expect(list.locator(".contacts-waiting")).toBeVisible();
  await expect(page.locator(".empty-state")).toHaveCount(0);
  await expect.poll(() => page.locator(".contact-item").count()).toBeGreaterThan(5);
  await expect(list).toHaveAttribute("data-phase", "idle");

  // Typing asks again, and the last answer stays on the page until the next one lands.
  await page.getByLabel("Search contacts").fill("sam");
  await expect(list).toHaveAttribute("data-phase", "loading");
  expect(await page.locator(".contact-item").count()).toBeGreaterThan(5);
  await expect(page.locator(".contact-item")).toHaveCount(1);

  // The same when the last answer was nobody: the line stays rather than blinking blank.
  await page.getByLabel("Search contacts").fill("nobodyatall");
  await expect(page.locator(".empty-state")).toHaveText("Nobody by that name");
  await page.getByLabel("Search contacts").fill("sam");
  await expect(list).toHaveAttribute("data-phase", "loading");
  await expect(page.locator(".empty-state")).toHaveText("Nobody by that name");
  await expect(page.locator(".contact-item")).toHaveCount(1);
});

test("the Contacts place says so when the list will not come", async ({ page }) => {
  await failCommands(page, ["contacts_list"]);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.keyboard.press("Meta+k");
  await page.locator(".palette-input").fill("contacts");
  await page.keyboard.press("Enter");

  await expect(page.locator(".contacts-list")).toHaveAttribute("data-phase", "error");
  await expect(page.locator(".contacts-note")).toHaveText("Could not load your contacts");
  // Not "Nobody yet", which would be a claim about a list nobody has seen.
  await expect(page.locator(".empty-state")).toHaveCount(0);
});

test("Unsubscribe on the card says it is working, and the toast is the way back", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await page.keyboard.press("Meta+k");
  await page.locator(".palette-input").fill("contacts");
  await page.keyboard.press("Enter");
  // Narrowed to the one row first, so the card hangs off the top of the list and its foot, where
  // the button is, is on screen. A popover hangs below its anchor and does not climb.
  await page.getByLabel("Search contacts").fill("browser");
  await expect(page.locator(".contact-item")).toHaveCount(1);
  await page.locator(".contact-item", { hasText: "The Browser" }).locator(".row").click();
  await expect(card(page).locator(".popover-sub")).toHaveText("hello@thebrowser.example");

  const button = card(page).locator(".contact-unsub");
  await expect(button).toHaveText("Unsubscribe");
  await button.click();
  await expect(button).toHaveText("Unsubscribing");
  await expect(button).toHaveAttribute("data-phase", "unsubscribing");
  await expect(button).toBeDisabled();

  await expect
    .poll(async () => (await toast(page))?.text)
    .toBe("Unsubscribed from hello@thebrowser.example");
  expect((await toast(page))?.action).toContain("Undo");
  await expect(button).toHaveText("Unsubscribe");
  await expect(button).toBeEnabled();
});

test("the card over a thread", async ({ page }) => {
  mkdirSync(shots, { recursive: true });
  await cardFor(page, "Piano on Wednesdays");
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "contact-card.png") });
});
