// The reading pane. A thread is a subject, the people in it, and the messages, with everything
// older than the last one folded down to a line.
//
// The bodies are in sandboxed iframes, so `bodyText` reads across frames: that is the point of the
// helper and it is the only way to prove a message actually rendered rather than merely arrived.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";
import {
  bodyText,
  box,
  failCommands,
  listReady,
  MIDDAY,
  openApp,
  openRow,
  paneMessages,
  settle,
  toast,
} from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

/**
 * The pane has a message in it, and not merely a thread.
 *
 * A body is a document of its own, created by the browser a beat after the thread it belongs to
 * arrives, and the height the parent writes on the frame is the one signal that the parent has
 * found it: sizing it and catching its clicks are the same moment. Waiting for the subject instead
 * is asking the browser about a document it has not made yet, which is a test that fails on a busy
 * machine and passes on a quiet one.
 */
async function bodyReady(page: import("@playwright/test").Page): Promise<void> {
  await expect
    .poll(async () =>
      page.locator(".msg-frame").first().evaluate((el) => (el as HTMLIFrameElement).style.height),
    )
    .not.toBe("");
}

const openLease = async (page: import("@playwright/test").Page) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await bodyReady(page);
  await settle(page);
};

test("the pane shows the subject, the people and the message count", async ({ page }) => {
  await openLease(page);

  await expect(page.locator(".thread-subject")).toHaveText("Lease renewal for the studio");
  await expect(page.locator(".thread-meta")).toContainText("Arun Kulkarni and you");
  await expect(page.locator(".thread-meta")).toContainText("2 messages");
  // One face per participant, stacked.
  await expect(page.locator(".thread-meta .avatar")).toHaveCount(2);
});

test("the latest message is open and the older ones are a line each", async ({ page }) => {
  await openLease(page);

  const messages = await paneMessages(page);
  expect(messages).toHaveLength(2);
  expect(messages[0].collapsed).toBe(true);
  expect(messages[0].name).toBe("You");
  expect(messages[0].preview).toContain("thanks for sending the renewal over");
  expect(messages[1].collapsed).toBe(false);
  expect(messages[1].name).toBe("Arun Kulkarni");
  expect(messages[1].address).toBe("arun@meridianproperties.in");
  expect(messages[1].time).toBe("Today 10:15");
});

test("a message body renders inside its own frame", async ({ page }) => {
  await openLease(page);

  await expect(page.locator(".msg-frame")).toHaveCount(1);
  await expect(page.locator(".msg-frame")).toHaveAttribute("sandbox", "allow-same-origin");

  const bodies = await bodyText(page);
  expect(bodies.join(" ")).toContain("Attached the revised draft");
  // The quoted half of the message is behind its pill and is not in the body.
  expect(bodies.join(" ")).not.toContain("On Mon, Priyanshu Jain wrote");

  // The face has to be injected too: a child document does not inherit the parent's @font-face
  // rules, and a body that fell back would render in Times without anything else looking wrong.
  const face = await page.locator(".msg-frame").evaluate((el) => {
    const doc = (el as HTMLIFrameElement).contentDocument!;
    return getComputedStyle(doc.body).fontFamily;
  });
  expect(face).toContain("Literata");

  // The frame is sized to its content, so the pane scrolls as one page.
  const height = await page.locator(".msg-frame").evaluate((el) => el.getBoundingClientRect().height);
  expect(height).toBeGreaterThan(100);
});

test("a plain text body in dark reads in the dark palette", async ({ page }) => {
  await openApp(page, { theme: "dark", now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: "Dinner on Thursday?" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();

  // The frame is a document of its own, so the app's tokens are written onto its root rather than
  // inherited. Light ink on the page and light ink in the frame is what proves that happened.
  await expect
    .poll(async () =>
      page.locator(".msg-frame").first().evaluate((el) => {
        const doc = (el as HTMLIFrameElement).contentDocument!;
        return getComputedStyle(doc.body).color;
      }),
    )
    .toBe("rgb(236, 230, 218)");
});

/** What a body actually renders as, which is the only thing worth asserting about a surface. */
async function surfaceOf(page: import("@playwright/test").Page): Promise<string> {
  return page.locator(".msg-frame").first().evaluate((el) => {
    const doc = (el as HTMLIFrameElement).contentDocument!;
    const style = getComputedStyle(doc.body);
    const root = getComputedStyle(doc.documentElement);
    return `${style.color} on ${style.backgroundColor}, ${root.colorScheme}`;
  });
}

const DARK = "rgb(236, 230, 218) on rgb(29, 26, 22), dark";
// `light only` and not `only light`: the browser reorders it on the way back out of the computed
// style, and the value written in public/message.css is the one the CSS grammar asks for.
const PAPER = "rgb(35, 32, 27) on rgb(252, 251, 247), light only";

test("a body that paints no surface of its own reads on the theme in dark", async ({ page }) => {
  await openApp(page, { theme: "dark", now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();

  // This message arrived as HTML, and that is not the question. It carries bold runs, links and a
  // signature and it paints nothing, so there is no page of the sender's to preserve and it reads
  // like the rest of the app. The rule this replaces put every HTML body on a white page, which
  // made a colleague's mail a slab of white in a dark window.
  await expect(page.locator(".msg-frame").first()).toHaveAttribute("data-surface", "theme");
  await expect.poll(() => surfaceOf(page)).toBe(DARK);
});

test("a body that paints its own page keeps it in dark", async ({ page }) => {
  await openApp(page, { theme: "dark", now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: "Your reservation in Lisbon is confirmed" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();

  // The other half. This one wraps itself in a tinted surface, which is a design rather than an
  // accident, so it keeps the light palette in both themes and the pane's chrome goes dark around
  // it. `only light` is what stops the browser putting its own dark canvas under a page we have
  // just painted white.
  await expect(page.locator(".msg-frame").first()).toHaveAttribute("data-surface", "paper");
  await expect.poll(() => surfaceOf(page)).toBe(PAPER);
});

test("the message head offers a way out when the rule guesses wrong", async ({ page }) => {
  await openApp(page, { theme: "dark", now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await expect.poll(() => surfaceOf(page)).toBe(DARK);

  // Every heuristic misses, so one press overrules it for this message. It is only offered in dark
  // and only on a body that arrived as HTML, because that is the only place the two surfaces look
  // different and the only place a sender's page could have been taken away.
  const flip = page.locator(".msg-head .button", { hasText: "" }).last();
  await expect(flip).toHaveAttribute("title", "Read this message on a light page");
  await flip.click();
  await expect.poll(() => surfaceOf(page)).toBe(PAPER);
  await expect(flip).toHaveAttribute("title", "Read this message in dark");

  await flip.click();
  await expect.poll(() => surfaceOf(page)).toBe(DARK);
});

test("the surface control is not offered in light, where it would do nothing", async ({ page }) => {
  await openLease(page);
  await expect(page.locator(".msg-head .button")).toHaveCount(0);
});

test("quoted text opens from its pill", async ({ page }) => {
  await openLease(page);

  await page.locator(".pill", { hasText: "Show quoted text" }).click();
  await expect(page.locator(".msg-frame")).toHaveCount(2);
  await expect
    .poll(async () => (await bodyText(page)).join(" "))
    .toContain("On Mon, Priyanshu Jain wrote");
});

test("the tracker banner names the vendor", async ({ page }) => {
  await openLease(page);

  const banner = page.locator(".banner");
  await expect(banner).toContainText("Blocked");
  await expect(banner).toContainText("1 tracker");
  await expect(banner).toContainText("HubSpot");
  await expect(banner).toContainText("Remote images are off for this sender");
  await expect(banner.locator(".banner-action")).toHaveText("Show images");
});

test("a link in a body is caught by the parent rather than followed", async ({ page }) => {
  await openLease(page);

  // No fixture body carries a link, so the anchor is put into the frame here. What is being tested
  // is the listener the parent attached to the frame's document, not the sanitiser's output.
  const caught = await page.locator(".msg-frame").evaluate((el) => {
    const doc = (el as HTMLIFrameElement).contentDocument!;
    const anchor = doc.createElement("a");
    anchor.href = "https://example.org/lease";
    anchor.textContent = "the lease";
    doc.body.appendChild(anchor);
    const event = new doc.defaultView!.MouseEvent("click", { bubbles: true, cancelable: true });
    anchor.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(caught).toBe(true);
});

test("Show images asks for the message again and the banner stops offering it", async ({ page }) => {
  await openLease(page);

  await page.locator(".banner-action", { hasText: "Show images" }).click();
  await expect(page.locator(".banner-action")).toHaveCount(0);
  // The tracker is still worth saying: it was stripped whatever happens to the images.
  await expect(page.locator(".banner")).toContainText("HubSpot");
});

test("Show images says it is loading, and refuses a second press, until the pictures land", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".banner-action")).toHaveText("Show images", { timeout: 10_000 });

  await page.locator(".banner-action").click();
  const action = page.locator(".banner-action");
  await expect(action).toHaveText("Loading images…");
  await expect(action).toBeDisabled();
  await expect(action).toHaveAttribute("aria-busy", "true");

  await expect(page.locator(".banner-action")).toHaveCount(0, { timeout: 10_000 });
  await expect(page.locator(".banner")).toContainText("HubSpot");
});

test("the attachment is a chip with its kind and size", async ({ page }) => {
  await openLease(page);

  const chip = page.locator(".attachment");
  await expect(chip).toHaveCount(1);
  await expect(chip).toContainText("Studio-lease-2026-v2.pdf");
  await expect(chip.locator(".ext")).toHaveText("PDF");
  await expect(chip.locator(".size")).toHaveText("412 KB");
});

test("the attachment chip hands the file to the OS, and says it is working until then", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  const chip = page.locator(".attachment");
  await expect(chip).toHaveAttribute("data-phase", "idle", { timeout: 10_000 });

  await chip.click();
  await expect(chip).toHaveAttribute("data-phase", "opening");
  await expect(chip).toBeDisabled();

  // Opened, and nothing to say about it: the file is in whatever owns it now.
  await expect(chip).toHaveAttribute("data-phase", "idle", { timeout: 10_000 });
  await expect(chip).toBeEnabled();
  expect(await toast(page)).toBeNull();
});

test("a file that will not open says so on the chip and in a toast", async ({ page }) => {
  await failCommands(page, ["attachment_open"]);
  await openLease(page);

  const chip = page.locator(".attachment");
  await chip.click();
  await expect(chip).toHaveAttribute("data-phase", "error");
  await expect(chip).toBeEnabled();
  await expect.poll(async () => (await toast(page))?.text ?? "").toContain("the mailbox is offline");
});

test("a note on the thread is a block on the note surface", async ({ page }) => {
  await openLease(page);

  const note = page.locator(".note");
  await expect(note).toContainText("Note to self");
  await expect(note).toContainText("Check the four percent cap");
});

test("More drops the rest of the thread's verbs, from the button and from .", async ({ page }) => {
  await openLease(page);

  await page.locator(".pane-bar").getByRole("button", { name: "More" }).click();
  const menu = page.getByRole("dialog", { name: "More" });
  await expect(menu).toBeVisible();
  // The verbs the bar does not carry, in the order the keyboard table lists them, each with its
  // key. Move to a label and Unsubscribe are not here because nothing owns them in the Inbox.
  await expect(menu.getByRole("menuitem")).toHaveText([
    /^Reply all/,
    /^Forward/,
    /^Mark (un)?seen/,
    /^Unstar/,
    /^Add a note/,
    /^Contact card/,
    /^Label/,
    /^Trash/,
    /^Mark as spam/,
  ]);
  await expect(menu.getByRole("menuitem", { name: /^Trash/ })).toContainText("#");

  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);

  await page.keyboard.press(".");
  await expect(menu).toBeVisible();
  // A verb's own key works from inside the menu and takes the menu down with it.
  await page.keyboard.press("#");
  await expect(menu).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toBe("Trashed");
  await expect(page.locator(".row", { hasText: "Lease renewal for the studio" })).toHaveCount(0);
});

test("a row in the More menu runs its verb", async ({ page }) => {
  await openLease(page);

  await page.keyboard.press(".");
  const menu = page.getByRole("dialog", { name: "More" });
  await menu.getByRole("menuitem", { name: /^Unstar/ }).click();
  await expect(menu).toHaveCount(0);
  // The menu reads the row, so opening it again is how the test sees the star has gone.
  await page.keyboard.press(".");
  await expect(menu.getByRole("menuitem", { name: /^Star/ })).toBeVisible();
});

test("Paper Trail groups by week and reads in the interface face", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.keyboard.press("3");
  await expect(page.locator(".list-title")).toHaveText("Paper Trail");
  await openRow(page, 1);

  await expect(page.locator(".thread-subject")).toHaveText("Bill payment pending");
  // The pane bar carries Move to Inbox in this place and nowhere else.
  await expect(page.locator(".pane-bar")).toContainText("Move to Inbox");
  await bodyReady(page);
  expect(
    await page.locator(".msg-frame").last().evaluate((el) => {
      const doc = (el as HTMLIFrameElement).contentDocument!;
      return doc.body.hasAttribute("data-plain");
    }),
  ).toBe(true);
});

test("a thread addressed to forty people has a head of two lines and a message on screen", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-crowd": "1" } });
  await listReady(page);
  await page.locator(".row", { hasText: "Q3 planning offsite" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await settle(page);

  // Four faces and one chip for the rest, rather than thirty-nine circles across the pane.
  await expect(page.locator(".thread-meta .avatar")).toHaveCount(5);
  await expect(page.locator(".thread-meta .avatar[data-more]")).toHaveText("+35");
  await expect(page.locator(".thread-meta .thread-people")).toHaveText(
    "Melanie Brennan, Yanni Kyriacos, Aditi Rao and 36 others",
  );
  // The names that did not fit are one hover away rather than gone.
  await expect(page.locator(".thread-meta .thread-people")).toHaveAttribute(
    "title",
    /Kofi Mensah/,
  );

  // The head is one line of people and at most two of subject, whatever the sender wrote.
  const meta = await box(page.locator(".thread-meta"));
  expect(meta.height).toBeLessThan(32);
  const subject = await box(page.locator(".thread-subject"));
  expect(subject.height).toBeLessThan(60);

  // Which is the point of all three: the message is above the fold rather than under it.
  const body = await box(page.locator(".msg-frame"));
  expect(body.top).toBeLessThan(280);
  expect((await bodyText(page)).join(" ")).toContain("The room is booked from ten");
});

test("a thread whose bodies have not arrived shows a skeleton, never the last one you read", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);

  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect
    .poll(async () => (await bodyText(page)).join(" "))
    .toContain("Attached the revised draft");

  await page.locator(".row", { hasText: "Dinner on Thursday?" }).click();

  // The pane is the thread that was clicked on the frame it was clicked, drawn from the row the
  // list was already holding, with the body still on its way.
  await expect(page.locator(".pane[data-opening] .thread-subject")).toHaveText(
    "Dinner on Thursday?",
  );
  await expect(page.locator(".msg-pending").first()).toBeVisible();
  expect((await bodyText(page)).join(" ")).not.toContain("Attached the revised draft");

  // And the view lands, and then the bodies land behind it, without the page having flashed.
  await expect(page.locator(".pane[data-opening]")).toHaveCount(0);
  await expect(page.locator(".thread-subject")).toHaveText("Dinner on Thursday?");
  await expect(page.locator(".msg-pending")).toHaveCount(0, { timeout: 10_000 });
  expect((await bodyText(page)).join(" ")).toContain("Priya said the place on Church Street");
});

test("a body the provider refuses becomes a line that offers to try again", async ({ page }) => {
  await openApp(page, {
    now: MIDDAY(),
    storage: { "marginmail-dev-pending": "1", "marginmail-dev-hydrate-fails": "1" },
  });
  await listReady(page);
  await page.locator(".row", { hasText: "Dinner on Thursday?" }).click();
  await expect(page.locator(".thread-subject")).toHaveText("Dinner on Thursday?");

  // The count came back as zero and nothing landed, which is not an error to the command and used
  // to leave the skeleton breathing for as long as the thread was open. No toast either: nothing
  // failed that a person could act on from a toast, and the slot itself carries the way back.
  const failed = page.locator('.msg-pending[data-state="error"]');
  await expect(failed).toBeVisible({ timeout: 10_000 });
  await expect(failed).toContainText("Could not fetch this message");
  expect(await toast(page)).toBeNull();

  // Asked for again with the provider answering this time: the line is a skeleton for a beat, and
  // then the body.
  await page.evaluate(() => localStorage.removeItem("marginmail-dev-hydrate-fails"));
  await failed.locator(".pill", { hasText: "Try again" }).click();
  await expect(page.locator('.msg-pending[data-state="error"]')).toHaveCount(0);
  await expect(page.locator(".msg-pending")).toHaveCount(0, { timeout: 10_000 });
  await expect
    .poll(async () => (await bodyText(page)).join(" "))
    .toContain("Priya said the place on Church Street");
});

test("a fetch the mailbox refuses outright says so, and the slot still offers to try again", async ({
  page,
}) => {
  await failCommands(page, ["thread_hydrate"]);
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await page.locator(".row", { hasText: "Dinner on Thursday?" }).click();

  await expect
    .poll(async () => (await toast(page))?.text ?? "", { timeout: 10_000 })
    .toContain("Could not fetch the rest of this thread");
  await expect(page.locator('.msg-pending[data-state="error"]')).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('.msg-pending[data-state="error"] .pill')).toHaveText("Try again");
});

test("a thread, for the eye", async ({ page }) => {
  mkdirSync(shots, { recursive: true });
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: "Piano on Wednesdays" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "thread.png") });
});
