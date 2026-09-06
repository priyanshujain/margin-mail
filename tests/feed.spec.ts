// The Feed, which is the one place where the list and the message are the same thing.
//
// Everything here is read off the page rather than asked of the app: a card is a head, a title and
// a body in a frame, and the hairline is either between two cards or it is not.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import {
  bodyText,
  failCommands,
  listReady,
  MIDDAY,
  openApp,
  place,
  settle,
  toast,
  token,
} from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

interface CardInfo {
  sender: string;
  address: string;
  time: string;
  title: string;
  /** Whether the body is showing in full rather than clipped at the fade. */
  open: boolean;
  /** Whether the body has arrived at all. */
  loaded: boolean;
  focused: boolean;
  faded: boolean;
  foot: { label: string; key: string }[];
}

function cards(page: Page): Promise<CardInfo[]> {
  return page.evaluate(() => {
    const text = (el: Element | null) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();
    return [...document.querySelectorAll<HTMLElement>(".feed-card")].map((el) => {
      const address = text(el.querySelector(".msg-name .addr"));
      const body = el.querySelector(".feed-body");
      return {
        sender: text(el.querySelector(".msg-name")).replace(address, "").trim(),
        address,
        time: text(el.querySelector(".msg-time")),
        title: text(el.querySelector(".feed-title")),
        open: body?.hasAttribute("data-open") ?? false,
        loaded: body !== null,
        focused: el.hasAttribute("data-selected"),
        faded: el.querySelector(".feed-fade") !== null,
        foot: [...el.querySelectorAll<HTMLElement>(".feed-foot .button")].map((button) => {
          const key = text(button.querySelector(".key"));
          const all = text(button);
          return { label: (key && all.endsWith(key) ? all.slice(0, -key.length) : all).trim(), key };
        }),
      };
    });
  });
}

/** The hairline's place in the column: how many cards sit above it, or -1 when it is not there. */
function leftOffAt(page: Page): Promise<number> {
  return page.evaluate(() => {
    const marker = document.querySelector(".left-off");
    if (!marker) return -1;
    const above = [...document.querySelectorAll<HTMLElement>(".feed-card")].filter(
      (card) => card.compareDocumentPosition(marker) & Node.DOCUMENT_POSITION_FOLLOWING,
    );
    return above.length;
  });
}

/** Which card is the newest one on screen, which is what the hairline will mark next time. */
function topmostCard(page: Page): Promise<number> {
  return page.evaluate(() => {
    const edge = document.querySelector<HTMLElement>(".feed")!.getBoundingClientRect().top;
    return [...document.querySelectorAll<HTMLElement>(".feed-card")].findIndex(
      (card) => card.getBoundingClientRect().bottom > edge + 1,
    );
  });
}

async function openFeed(page: Page): Promise<void> {
  await listReady(page);
  await page.keyboard.press("2");
  await expect(page.locator(".feed-card").first()).toBeVisible();
  // The bodies arrive one call behind the cards.
  await expect(page.locator(".feed-body").first()).toBeVisible();
  await settle(page);
}

test("the Feed is a column of open cards on its own measure, and 2 reaches it", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await openFeed(page);

  expect(await place(page)).toBe("feed");
  expect(await token(page, "--feed-w")).toBe("720px");
  const column = await page.locator(".feed-inner").boundingBox();
  expect(column?.width).toBe(720);

  const all = await cards(page);
  expect(all.map((c) => c.sender)).toEqual([
    "The Browser",
    "Field Notes Dispatch",
    "Ledger Lines",
    "Bengaluru Systems Meetup",
    "Craftsman Notes",
  ]);

  const browser = all[0];
  expect(browser.address).toBe("hello@thebrowser.example");
  expect(browser.time).toBe("Today 06:00");
  expect(browser.title).toBe("Five things worth reading this weekend");
  // The card is already open: the body is here, not a row that opens one.
  expect(browser.loaded).toBe(true);
  expect(await bodyText(page)).toContainEqual(expect.stringContaining("Good morning."));

  // Read more and Save clip on the left, Unsubscribe and Move on the right, each printing its key.
  expect(browser.foot).toEqual([
    { label: "Read more", key: "↩" },
    { label: "Save clip", key: "⌘⇧C" },
    { label: "Unsubscribe", key: "⌘U" },
    { label: "Move", key: "v" },
  ]);

  // No read state, no counts, no New for you. Time is the only order.
  await expect(page.locator(".feed .group-head")).toHaveCount(0);
  await expect(page.locator(".feed [data-new]")).toHaveCount(0);
});

test("a card longer than the clip fades out, and Enter opens and closes it", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await openFeed(page);

  const first = () => cards(page).then((all) => all[0]);
  // The Browser's four paragraphs are longer than one clip; the rest are not.
  await expect.poll(async () => (await first()).faded).toBe(true);
  expect(await cards(page).then((all) => all[1].faded)).toBe(false);

  const clipped = await page.locator(".feed-body").first().boundingBox();
  expect(clipped?.height).toBeLessThan(260);

  await page.keyboard.press("j");
  expect((await first()).focused).toBe(true);

  await page.keyboard.press("Enter");
  await expect.poll(async () => (await first()).open).toBe(true);
  expect((await first()).foot[0].label).toBe("See less");
  expect((await first()).faded).toBe(false);
  const opened = await page.locator(".feed-body").first().boundingBox();
  expect(opened!.height).toBeGreaterThan(clipped!.height);

  await page.keyboard.press("Enter");
  await expect.poll(async () => (await first()).open).toBe(false);
  expect((await first()).foot[0].label).toBe("Read more");
});

test("j and k move between cards and bring the focused one on screen", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await openFeed(page);

  const focused = async () => (await cards(page)).find((c) => c.focused)?.sender;
  expect(await focused()).toBeUndefined();

  await page.keyboard.press("j");
  await expect.poll(focused).toBe("The Browser");
  await page.keyboard.press("j");
  await page.keyboard.press("j");
  await expect.poll(focused).toBe("Ledger Lines");

  // The card the keyboard is on is a card you can see.
  const visible = await page.evaluate(() => {
    const card = document.querySelector<HTMLElement>(".feed-card[data-selected]")!;
    const column = document.querySelector<HTMLElement>(".feed")!.getBoundingClientRect();
    const box = card.getBoundingClientRect();
    return box.bottom > column.top && box.top < column.bottom;
  });
  expect(visible).toBe(true);

  await page.keyboard.press("k");
  await expect.poll(focused).toBe("Field Notes Dispatch");
});

test("Unsubscribe acts on the card it is on, and Cmd+U on the card the keyboard is on", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await openFeed(page);

  // The button on the second card, which the keyboard has never been on.
  await page
    .locator(".feed-card")
    .nth(1)
    .locator(".feed-foot .button", { hasText: "Unsubscribe" })
    .click();
  await expect
    .poll(async () => (await toast(page))?.text)
    .toBe("Unsubscribed from dispatch@fieldnotes.example");
  expect((await toast(page))?.action).toContain("Undo");

  // The click took the focus to that card, so one step up is The Browser.
  await page.keyboard.press("k");
  await page.keyboard.press("Meta+u");
  await expect
    .poll(async () => (await toast(page))?.text)
    .toBe("Unsubscribed from hello@thebrowser.example");
});

test("the banner says images come through Margin and counts what was stripped today", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await openFeed(page);

  const banner = page.locator(".feed .banner");
  await expect(banner).toContainText("Images are loaded through Margin, never from the sender.");
  // One card carries a tracker and it arrived today, so that is what the count is.
  await expect(banner).toContainText("Trackers stripped from 1 item today.");
  await expect(banner.locator("b")).toHaveText("1 item");
});

test("the hairline marks where the last visit ended", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await openFeed(page);

  // Nothing has arrived since a visit that never happened.
  expect(await leftOffAt(page)).toBe(-1);

  // Read down the column and then go somewhere else, which is what leaving a place is.
  await page.evaluate(() => {
    const column = document.querySelector<HTMLElement>(".feed")!;
    column.scrollTop = column.scrollHeight;
  });
  await settle(page);
  const stopped = await topmostCard(page);
  expect(stopped).toBeGreaterThan(0);

  await page.keyboard.press("1");
  await listReady(page);

  await page.keyboard.press("2");
  await expect(page.locator(".feed-card").first()).toBeVisible();
  await expect(page.locator(".left-off")).toHaveText("You left off here");
  expect(await leftOffAt(page)).toBe(stopped);
});

test("a card whose body is not cached breathes until it arrives, and is never an empty frame", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await page.keyboard.press("2");
  await expect(page.locator(".feed-card").first()).toBeVisible();

  // The mirror has the card and not its body, so the card is its head, its title and the pane's
  // skeleton where the body will go. No frame, because an empty frame reads as an empty message.
  const first = page.locator(".feed-card").first();
  await expect(first.locator(".msg-pending")).toBeVisible();
  await expect(first.locator(".feed-body")).toHaveCount(0);

  // The fetch goes out behind the view and the view is read again when it lands, which is the half
  // that was missing: a card that only ever asked once was empty for the whole session.
  await expect(first.locator(".feed-body")).toBeVisible({ timeout: 10_000 });
  await expect(first.locator(".msg-pending")).toHaveCount(0);
  await expect
    .poll(async () => (await bodyText(page)).join(" "), { timeout: 10_000 })
    .toContain("Good morning.");
});

test("a body that does not come is one line on the card with a way to ask again", async ({
  page,
}) => {
  await failCommands(page, ["thread_hydrate"]);
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  await page.keyboard.press("2");

  const first = page.locator(".feed-card").first();
  const failed = first.locator('[data-state="error"]');
  await expect(failed).toBeVisible({ timeout: 10_000 });
  await expect(failed).toContainText("Could not fetch this message");
  await expect(first.locator(".feed-body")).toHaveCount(0);
  // Said on the card, where the way back is, and not as a toast: a Feed of two hundred cards that
  // cannot fetch would be two hundred toasts.
  await expect(page.locator(".toast")).toHaveCount(0);

  // Asking again is asking again: the skeleton comes back while the fetch is out.
  await failed.getByText("Try again").click();
  await expect(first.locator('[data-state="error"]')).toHaveCount(0);
  await expect(first.locator(".msg-pending")).toBeVisible();
  await expect(first.locator('[data-state="error"]')).toBeVisible({ timeout: 10_000 });
});

// One picture per test rather than a loop, because `openApp` seeds the store once per context and
// a second call on the same page would take the dark picture in the light palette.
for (const theme of ["light", "dark"] as const) {
  test(`the Feed in ${theme}`, async ({ page }) => {
    mkdirSync(shots, { recursive: true });
    await openApp(page, { theme, now: MIDDAY() });
    await openFeed(page);
    await page.evaluate(() => document.fonts.ready);
    await settle(page);
    await page.screenshot({ path: join(shots, `feed-${theme}.png`) });
  });
}
