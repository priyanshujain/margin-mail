// Writing. The card, the reply box in the thread, and the send that is held long enough to change
// your mind about.
//
// The undo delay is real time here rather than a faked clock: `page.clock.install` replaces
// `requestAnimationFrame` along with the timers, and `settle` in app.ts waits on two frames, so a
// faked clock would hang every helper in the suite. Ten seconds of waiting in one test is the
// cheaper of the two prices.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { listReady, MIDDAY, openApp, settle, toast } from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

/**
 * The Inbox, with the clock pinned or running.
 *
 * Every other spec pins it, because the fixture is anchored to the local day and a row that says
 * "11:42" says it because of when the suite ran. The send tests are the exception: `setFixedTime`
 * stops `Date.now()` moving, and a countdown read off a stopped clock counts down from ten to ten.
 */
const openInbox = async (page: Page, pinned = true) => {
  await openApp(page, pinned ? { now: MIDDAY() } : {});
  await listReady(page);
};

/** The card, with a recipient, a subject and a line of body in it. */
async function writeOne(page: Page, to = "maya", subject = "Thursday") {
  await page.keyboard.press("c");
  await expect(page.locator(".compose")).toBeVisible();
  await page.locator(".compose .chip-input").first().pressSequentially(to);
  await expect(page.locator(".suggest-row").first()).toBeVisible();
  await page.keyboard.press("Enter");
  await page.locator(".compose-subject").fill(subject);
  await page.locator(".compose .editor-body").click();
  await page.keyboard.type("Church Street at eight?");
  await settle(page);
}

/** The number the toast is counting down, or null when it is not counting anything. */
async function countdown(page: Page): Promise<number | null> {
  const said = await toast(page);
  const match = said ? /·\s*(\d+)\s*s/.exec(said.text) : null;
  return match ? Number(match[1]) : null;
}

test("c opens the card over the list, and Cmd+Shift+P makes it the window", async ({ page }) => {
  await openInbox(page);

  await page.keyboard.press("c");
  const card = page.locator(".compose");
  await expect(card).toBeVisible();
  await expect(card.locator("h2")).toHaveText("New message");

  // 600 wide in the bottom right, with the list still on screen behind it.
  const box = (await card.boundingBox())!;
  expect(Math.round(box.width)).toBe(600);
  expect(box.x + box.width).toBeGreaterThan(1400 - 40);
  await expect(page.locator(".row").first()).toBeVisible();

  await page.keyboard.press("Meta+Shift+P");
  await expect(card).toHaveAttribute("data-expanded", "");
  const wide = (await card.boundingBox())!;
  expect(wide.width).toBeGreaterThan(1300);

  await page.keyboard.press("Meta+Shift+P");
  await expect(card).not.toHaveAttribute("data-expanded", "");
  expect(Math.round((await card.boundingBox())!.width)).toBe(600);
});

test("typing a recipient offers suggestions and Enter makes a chip", async ({ page }) => {
  await openInbox(page);
  await page.keyboard.press("c");

  await page.locator(".compose .chip-input").first().pressSequentially("maya");
  const suggestions = page.locator(".suggest-row");
  await expect(suggestions.first()).toBeVisible();
  await expect(suggestions.first()).toContainText("Maya Raghunathan");
  await expect(suggestions.first()).toContainText("maya.raghunathan@example.com");

  await page.keyboard.press("Enter");
  await expect(page.locator(".compose .chip")).toHaveCount(2); // the From chip and this one
  await expect(page.locator(".compose-field", { hasText: "To" }).locator(".chip")).toContainText(
    "Maya Raghunathan",
  );
  // The list closes once it has been taken, and what was typed goes with it.
  await expect(suggestions).toHaveCount(0);
  await expect(page.locator(".compose .chip-input").first()).toHaveValue("");
});

test("an address nobody suggested becomes a chip on a comma", async ({ page }) => {
  await openInbox(page);
  await page.keyboard.press("c");

  await page.locator(".compose .chip-input").first().pressSequentially("nobody@elsewhere.test,");
  await expect(
    page.locator(".compose-field", { hasText: "To" }).locator(".chip"),
  ).toContainText("nobody@elsewhere.test");
});

test("Escape leaves the draft alone, and c brings it back", async ({ page }) => {
  await openInbox(page);
  await writeOne(page);

  await page.keyboard.press("Escape");
  await expect(page.locator(".compose")).toHaveCount(0);

  await page.keyboard.press("c");
  await expect(page.locator(".compose")).toBeVisible();
  await expect(page.locator(".compose-subject")).toHaveValue("Thursday");
  await expect(page.locator(".compose .editor-body")).toContainText("Church Street at eight?");
  await expect(page.locator(".compose-field", { hasText: "To" }).locator(".chip")).toContainText(
    "Maya Raghunathan",
  );
});

test("Cmd+Shift+, throws the draft away, and c opens a blank one", async ({ page }) => {
  await openInbox(page);
  await writeOne(page);

  await page.locator(".compose .editor-body").click();
  await page.keyboard.press("Meta+Shift+,");
  await expect(page.locator(".compose")).toHaveCount(0);

  await page.keyboard.press("c");
  await expect(page.locator(".compose-subject")).toHaveValue("");
  await expect(page.locator(".compose-field", { hasText: "To" }).locator(".chip")).toHaveCount(0);
});

test("Cmd+Enter sends, the toast counts down, and z inside the delay brings the draft back", async ({
  page,
}) => {
  await openInbox(page, false);
  await writeOne(page);

  await page.keyboard.press("Meta+Enter");
  await expect(page.locator(".compose")).toHaveCount(0);

  // The queue is a round trip, so the toast is a beat behind the key.
  await expect.poll(async () => (await toast(page))?.text).toContain("Sent to Maya Raghunathan");
  expect((await toast(page))?.action).toContain("Undo");

  // It is a countdown and not a label: the number goes down on its own.
  const started = await countdown(page);
  expect(started).not.toBeNull();
  await page.waitForTimeout(2_400);
  const later = await countdown(page);
  expect(later).toBeLessThan(started!);

  await page.keyboard.press("z");
  await expect(page.locator(".compose")).toBeVisible();
  await expect(page.locator(".compose-subject")).toHaveValue("Thursday");
  await expect(page.locator(".compose .editor-body")).toContainText("Church Street at eight?");
  expect(await toast(page)).toBeNull();
});

test("the toast stops offering Undo once the delay has run out", async ({ page }) => {
  test.setTimeout(45_000);
  await openInbox(page, false);
  await writeOne(page);
  await page.keyboard.press("Meta+Enter");
  await expect.poll(async () => (await toast(page))?.action).toContain("Undo");

  // The delay is ten seconds in the fixture's settings, and the toast goes with it.
  await expect.poll(async () => await toast(page), { timeout: 20_000 }).toBeNull();

  // `z` now means whatever the list means by it, and the send is not coming back.
  await page.keyboard.press("z");
  await settle(page);
  await expect(page.locator(".compose")).toHaveCount(0);
});

test("Cmd+Shift+Enter skips the wait", async ({ page }) => {
  await openInbox(page);
  await writeOne(page);

  await page.keyboard.press("Meta+Shift+Enter");
  await expect.poll(async () => (await toast(page))?.text).toContain("Sent to Maya Raghunathan");
  // Nothing to take back, so nothing is offered.
  expect((await toast(page))?.action).toBeNull();
});

test("Cmd+Shift+Enter from the card says it is sending while the provider is asked", async ({
  page,
}) => {
  // The card closes at once and there is no thread row to say "Waiting to send" for it, so the
  // toast is the only place the wait can be seen. The dev flag makes the wait long enough to see.
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await writeOne(page);

  await page.keyboard.press("Meta+Shift+Enter");
  await expect(page.locator(".compose")).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toContain("Sending to Maya Raghunathan");
  await expect.poll(async () => (await toast(page))?.text).toContain("Sent to Maya Raghunathan");
  expect((await toast(page))?.action).toBeNull();
});

test("r opens a reply box under the last message with the sender as a chip, and a adds the rest", async ({
  page,
}) => {
  await openInbox(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await settle(page);

  await page.keyboard.press("r");
  const box = page.locator(".reply");
  await expect(box).toBeVisible();
  await expect(box.locator(".reply-head")).toContainText("Reply to Arun Kulkarni");

  // Under the last message, in the thread, and not the compose card.
  await expect(page.locator(".compose")).toHaveCount(0);
  const last = (await page.locator(".msg").last().boundingBox())!;
  expect((await box.boundingBox())!.y).toBeGreaterThan(last.y);

  const chips = box.locator(".chip");
  await expect(chips).toHaveCount(1);
  await expect(chips.first()).toContainText("Arun Kulkarni");

  // Escape leaves the editor and keeps the draft, which is what makes `a` mean reply all again.
  await page.keyboard.press("Escape");
  await expect(box).toBeVisible();
  await page.keyboard.press("a");
  await expect(box.locator(".chip")).toHaveCount(2);
  await expect(box).toContainText("Meridian Leasing");
});

test("a reply box that was closed comes back with what was in it", async ({ page }) => {
  await openInbox(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();

  await page.keyboard.press("r");
  await page.locator(".reply .editor-body").click();
  await page.keyboard.type("Thursday afternoon suits me.");
  await page.locator(".reply-close").click();
  await expect(page.locator(".reply")).toHaveCount(0);

  await page.keyboard.press("r");
  await expect(page.locator(".reply .editor-body")).toContainText("Thursday afternoon suits me.");
});

test("f forwards with nobody in the To field and the message quoted under it", async ({ page }) => {
  await openInbox(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();

  await page.keyboard.press("f");
  const box = page.locator(".reply");
  await expect(box.locator(".reply-head")).toContainText("Forward");
  await expect(box.locator(".chip")).toHaveCount(0);
  await expect(box.locator(".editor-body blockquote")).toContainText("Attached the revised draft");
});

test("a reply that is sent leaves the thread waiting to send until it goes", async ({ page }) => {
  await openInbox(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();

  await page.keyboard.press("r");
  await page.locator(".reply .editor-body").click();
  await page.keyboard.type("Thursday afternoon suits me.");
  await page.keyboard.press("Meta+Enter");

  await expect(page.locator(".reply")).toHaveCount(0);
  await expect(page.locator(".sending")).toContainText("Waiting to send");

  await page.locator(".sending-now").click();
  await expect(page.locator(".sending")).toHaveCount(0);
});

test("Send now says it is sending, and takes one press while it is", async ({ page }) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  // Under this flag the bodies land behind the head, and `r` answers a message, not a skeleton.
  await expect(page.locator(".msg-pending")).toHaveCount(0, { timeout: 10_000 });

  await page.keyboard.press("r");
  await page.locator(".reply .editor-body").click();
  await page.keyboard.type("Thursday afternoon suits me.");
  await page.keyboard.press("Meta+Enter");
  const line = page.locator(".sending");
  await expect(line).toContainText("Waiting to send");
  await expect(line).toHaveAttribute("data-phase", "idle");

  // The push is a round trip, and the line says so rather than looking the same for all of it.
  const now = page.locator(".sending-now");
  await now.click();
  await expect(line).toHaveAttribute("data-phase", "sending");
  await expect(now).toHaveText("Sending");
  await expect(now).toBeDisabled();

  await expect(line).toHaveCount(0);
});

test("Focus & Reply sends an item, collapses it, and the toast brings it back", async ({ page }) => {
  await openInbox(page, false);
  await page.keyboard.press("Shift+F");
  await expect(page.locator(".focus-item").first()).toBeVisible();

  const first = page.locator(".focus-item").first();
  const subject = await first.locator(".focus-subject").textContent();
  await first.locator(".focus-box").fill("Wednesday at four suits us.");
  await page.keyboard.press("Meta+Enter");

  // The page shortens as it is worked through: the item it answered is one line now.
  const sent = page.locator(".focus-sent");
  await expect(sent).toContainText("Sent to");
  await expect(sent).toContainText(subject!.trim());
  await expect.poll(async () => (await toast(page))?.action).toContain("Undo");

  // Sending moves the caret on to the next box, which is what "send this reply and move on" means,
  // so `z` there is a letter and the toast is the way back. `z` is still registered on this stage
  // for when the caret is not in a box, because the reading pane is not mounted behind it.
  await page.locator(".toast-action").click();
  await expect(page.locator(".focus-sent")).toHaveCount(0);
  await expect(page.locator(".focus-item").first().locator(".focus-box")).toHaveValue(
    "Wednesday at four suits us.",
  );
});

test("the composer refuses an attachment that would not fit, and says so", async ({ page }) => {
  await openInbox(page);
  await page.keyboard.press("c");

  // Dropped rather than picked: a webview's file picker hands back no path, and drag and drop is
  // the route that carries one on a desktop.
  await page.locator(".compose").evaluate((card) => {
    const transfer = new DataTransfer();
    transfer.items.add(new File([new Uint8Array(64)], "notes.txt", { type: "text/plain" }));
    card.dispatchEvent(new DragEvent("drop", { dataTransfer: transfer, bubbles: true }));
  });
  await expect(page.locator(".compose .attachment")).toContainText("notes.txt");

  await page.locator(".compose").evaluate((card) => {
    const transfer = new DataTransfer();
    const huge = new File([new Uint8Array(8)], "raw-scan.tiff", { type: "image/tiff" });
    // A real 40 MB file in a test is 40 MB of memory for one number.
    Object.defineProperty(huge, "size", { value: 40 * 1024 * 1024 });
    transfer.items.add(huge);
    card.dispatchEvent(new DragEvent("drop", { dataTransfer: transfer, bubbles: true }));
  });

  await expect.poll(async () => (await toast(page))?.text).toContain("35 MB");
  await expect(page.locator(".compose .attachment")).toHaveCount(1);
});

test("Cmd+Shift+I moves the introducer to Bcc and thanks them, and again puts them back", async ({
  page,
}) => {
  await openInbox(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await page.keyboard.press("r");
  await page.locator(".reply .editor-body").click();

  await page.keyboard.press("Meta+Shift+I");
  const bcc = page.locator(".reply .compose-field", { hasText: "Bcc" });
  await expect(bcc.locator(".chip")).toContainText("Arun Kulkarni");
  await expect(page.locator(".reply .editor-body")).toContainText(
    "Thank you for the introduction",
  );

  await page.keyboard.press("Meta+Shift+I");
  await expect(bcc.locator(".chip")).toHaveCount(0);
  await expect(page.locator(".reply .editor-body")).not.toContainText(
    "Thank you for the introduction",
  );
});

test("Remind me if no reply is a toggle with a date on it", async ({ page }) => {
  await openInbox(page);
  await page.keyboard.press("c");

  await expect(page.locator(".compose-date")).toHaveCount(0);
  await page.locator(".compose-foot .button", { hasText: "Remind me if no reply" }).click();
  await expect(page.locator(".compose-date")).toHaveValue(/\d{4}-\d{2}-\d{2}/);
  await page.locator(".compose-foot .button", { hasText: "Remind me if no reply" }).click();
  await expect(page.locator(".compose-date")).toHaveCount(0);
});

test("the foot says how long a send is held for", async ({ page }) => {
  await openInbox(page);
  await page.keyboard.press("c");
  await expect(page.locator(".compose-delay")).toHaveText("Undo send: 10 s");
});

test("the compose card over the Inbox", async ({ page }) => {
  mkdirSync(shots, { recursive: true });
  await openInbox(page);
  await page.keyboard.press("c");
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
  await settle(page);

  await page.screenshot({ path: join(shots, "compose.png") });
});
