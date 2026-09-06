// The Screener, driven the way a person drives it: look at the cards, press a letter, and see
// which card left and what the toast says about it.
//
// A decision holds its card in place, buttons down, until the fixture answers, and only then does
// the card leave. That is deliberate and it is what the double-click test is about: a card that
// left at once put the next card under the pointer in time for the second click. Clear all is
// still optimistic, because nothing takes anything's place there.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { bodyText, listReady, MIDDAY, openApp, place, settle, toast } from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

/** A sender at a consumer domain, which the fixture has none of and a domain rule cannot cover. */
const CONSUMER = {
  accountId: "acct-1",
  sender: { name: "Nadia Okafor", address: "nadia.okafor@gmail.com" },
  threadKey: "<consumer-sunday@gmail.com>",
  subject: "Sunday",
  snippet: "Are you around on Sunday? I can bring the ladder.",
  dateMs: Date.now() - 3_600_000,
  suggestion: "inbox",
  reason: "Written by a person",
  waiting: 1,
};

interface ShimOptions {
  /** Keeps every command and its arguments on `window`, so a spec can say what was not sent. */
  record?: boolean;
  /** Extra cards `screener_list` answers with, for a sender the fixture does not have. */
  extra?: unknown[];
}

/**
 * Answers the fixture module's own request with a shim in front of it, the same trick
 * `failCommands` uses. Nothing in `src/` knows it happened: the app under test is the app that
 * ships, and the fixture behind this is the fixture it ships with.
 */
async function shimIpc(page: Page, options: ShimOptions): Promise<void> {
  await page.route("**/src/dev/mockIpc.ts*", async (route) => {
    if (new URL(route.request().url()).searchParams.has("real")) return route.fallback();
    await route.fulfill({
      contentType: "text/javascript",
      body: [
        `import { mockCall as real } from "/src/dev/mockIpc.ts?real";`,
        `const extra = ${JSON.stringify(options.extra ?? [])};`,
        `const record = ${options.record ? "true" : "false"};`,
        `if (record) window.__calls = [];`,
        `export function mockCall(command, args) {`,
        `  if (record) window.__calls.push({ command, args });`,
        `  const answer = real(command, args);`,
        `  if (command !== "screener_list" || extra.length === 0) return answer;`,
        `  const mine = extra.filter((c) => !args?.accountId || c.accountId === args.accountId);`,
        `  return answer.then((cards) => [...cards, ...mine]);`,
        `}`,
      ].join("\n"),
    });
  });
}

function calls(page: Page): Promise<{ command: string; args: Record<string, unknown> }[]> {
  return page.evaluate(() => (window as unknown as { __calls: never[] }).__calls ?? []);
}

interface CardInfo {
  sender: string;
  address: string;
  subject: string;
  snippet: string;
  why: string;
  /** The three buttons, with the letter each one prints. */
  actions: { label: string; key: string }[];
  focused: boolean;
}

/** Every card on screen, read the way a person reads it. */
function cards(page: Page): Promise<CardInfo[]> {
  return page.evaluate(() => {
    const text = (el: Element | null) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();
    return [...document.querySelectorAll<HTMLElement>(".screen-card")].map((el) => {
      const address = text(el.querySelector(".screen-name .addr"));
      return {
        sender: text(el.querySelector(".screen-name")).replace(address, "").trim(),
        address,
        subject: text(el.querySelector(".screen-subject")),
        snippet: text(el.querySelector(".screen-snippet")),
        why: text(el.querySelector(".pill")),
        actions: [...el.querySelectorAll<HTMLElement>(".screen-actions .button")].map((button) => {
          const key = text(button.querySelector(".key"));
          const all = text(button);
          return { label: (key && all.endsWith(key) ? all.slice(0, -key.length) : all).trim(), key };
        }),
        focused: el.hasAttribute("data-selected"),
      };
    });
  });
}

/** Opens the Screener from the Inbox the way the keyboard does, and waits for the cards. */
async function openScreener(page: Page): Promise<void> {
  await listReady(page);
  await page.keyboard.press("6");
  await expect(page.locator(".screen-card").first()).toBeVisible();
  await settle(page);
}

test("a card per sender, with its reason and where it suggests they go", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await openScreener(page);

  expect(await place(page)).toBe("screener");
  await expect(page.locator(".screen-intro h1")).toHaveText("Screener");
  await expect(page.locator(".screen-intro p")).toContainText(
    "Three people wrote to you for the first time",
  );

  const all = await cards(page);
  expect(all.map((c) => c.sender)).toEqual([
    "Todd Markham",
    "The Browser",
    "Evite on behalf of Robyn Madison",
  ]);

  const todd = all[0];
  expect(todd.address).toBe("todd@harborlife.example");
  expect(todd.subject).toBe("Re: Life insurance quote");
  expect(todd.snippet).toContain("I hope all is well with you");
  expect(todd.why).toBe("Written by a person · suggested Inbox");
  // The suggestion is on the button as well as in the pill: `y` says where it is going.
  expect(todd.actions).toEqual([
    { label: "Yes, to Inbox", key: "y" },
    { label: "Elsewhere", key: "v" },
    { label: "No", key: "n" },
  ]);

  expect(all[1].why).toBe("Has an unsubscribe header · suggested Feed");
  expect(all[1].actions[0].label).toBe("Yes, to Feed");
  expect(all[2].why).toBe("Sent by a service for a person · suggested Paper Trail");
  expect(all[2].actions[0].label).toBe("Yes, to Paper Trail");

  // The keyboard is on the top card, and it is the only one wearing the ring.
  expect(all.map((c) => c.focused)).toEqual([true, false, false]);

  // One faint sentence about what a No costs and how to take it back.
  await expect(page.locator(".screen-note")).toContainText("Screened out");
  await expect(page.locator(".screen-note")).toContainText("contact card");
});

test("y accepts the suggestion, the next card takes the focus, and the toast puts it back", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await openScreener(page);

  await page.keyboard.press("y");

  // Gone once the fixture has answered, which in the fixture is the same frame.
  await expect(page.locator(".screen-card")).toHaveCount(2);
  const after = await cards(page);
  expect(after[0].sender).toBe("The Browser");
  // Dealing cards: the hand stays where it was rather than going back to the top of the pile.
  expect(after[0].focused).toBe(true);

  await expect.poll(async () => (await toast(page))?.text).toBe("Screened in to Inbox");
  expect((await toast(page))?.action).toContain("Undo");

  await page.locator(".toast-action").click();
  await expect(page.locator(".screen-card")).toHaveCount(3);
  expect((await cards(page))[0].sender).toBe("Todd Markham");
  // And it says what it undid, the way `z` does, with nothing left to press twice.
  await expect.poll(async () => (await toast(page))?.text).toBe("Undone: Screened in to Inbox");
  expect((await toast(page))?.action).toBeNull();
});

test("a double-click on Yes decides one sender, because the card waits for the answer", async ({
  page,
}) => {
  await shimIpc(page, { record: true });
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await openScreener(page);

  const first = page.locator(".screen-card").first();
  await first.locator(".screen-actions .button").first().dblclick();

  // The card is still there, marked, with all three buttons down, so the second click landed on
  // a button that could not take it rather than on the next sender's Yes.
  await expect(first).toHaveAttribute("data-state", "deciding");
  await expect(page.locator(".screen-card")).toHaveCount(3);
  for (const button of await first.locator(".screen-actions .button").all()) {
    await expect(button).toBeDisabled();
  }

  await expect(page.locator(".screen-card")).toHaveCount(2, { timeout: 5_000 });
  expect((await cards(page))[0].sender).toBe("The Browser");
  await expect.poll(async () => (await toast(page))?.text).toBe("Screened in to Inbox");

  const decided = (await calls(page)).filter((c) => c.command === "screener_decide");
  expect(decided).toHaveLength(1);
  expect(decided[0].args).toMatchObject({ address: "todd@harborlife.example" });
});

/** A held sender whose thread the fixture does hold, so an expanded card has a body to fetch. */
const CACHED = {
  accountId: "acct-1",
  sender: { name: "Meridian Properties", address: "leasing@meridianproperties.in" },
  threadKey: "<lease-2026-001@meridianproperties.in>",
  subject: "Lease renewal for the studio",
  snippet: "Attached the revised draft. The only change is clause 7, which now",
  dateMs: Date.now() - 7_200_000,
  suggestion: "paper-trail",
  reason: "Sent by a service for a person",
  waiting: 1,
};

test("an expanded card whose body is not cached breathes until it arrives", async ({ page }) => {
  await shimIpc(page, { extra: [CACHED] });
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await openScreener(page);

  await page.locator(".screen-card", { hasText: "Meridian Properties" }).click();
  await page.keyboard.press("Enter");

  // The skeleton in the message's slot, over the Reply button, and no frame until there is a body
  // to put in it.
  const slot = page.locator(".screen-message");
  await expect(slot.locator(".msg-pending")).toBeVisible();
  await expect(slot.locator(".msg-frame")).toHaveCount(0);
  await expect(slot.locator(".button")).toHaveText(/Reply/);

  await expect(slot.locator(".msg-frame")).toBeVisible({ timeout: 10_000 });
  await expect(slot.locator(".msg-pending")).toHaveCount(0);
  await expect
    .poll(async () => (await bodyText(page)).join(" "), { timeout: 10_000 })
    .toContain("Attached the revised draft");
});

test("j and k walk the cards", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await openScreener(page);

  const focused = async () => (await cards(page)).find((c) => c.focused)?.sender;
  expect(await focused()).toBe("Todd Markham");

  await page.keyboard.press("j");
  await expect.poll(focused).toBe("The Browser");
  await page.keyboard.press("j");
  await expect.poll(focused).toBe("Evite on behalf of Robyn Madison");
  // The end of the pile is the end of the pile, not the top of it again.
  await page.keyboard.press("j");
  await expect.poll(focused).toBe("Evite on behalf of Robyn Madison");
  await page.keyboard.press("k");
  await expect.poll(focused).toBe("The Browser");
});

test("Enter opens the whole message on the card, and Enter closes it", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await openScreener(page);

  await page.keyboard.press("Enter");
  await expect(page.locator(".screen-message")).toHaveCount(1);
  // Replying screens the sender in, so the card says so beside the button.
  await expect(page.locator(".screen-message .button")).toHaveText(/Reply/);
  await expect(page.locator(".screen-message-note")).toContainText("screens this sender in");
  // The body itself is a `thread_view`, which the dev fixture cannot answer for this sender: its
  // Screener cards name thread keys that are in no place and so are in no fixture list. So what
  // the slot shows is the line for a body that did not come, with the way to ask again, rather
  // than an empty frame over the button.
  const failed = page.locator(".screen-message [data-state=\"error\"]");
  await expect(failed).toContainText("Could not fetch this message");
  await expect(failed.getByText("Try again")).toBeVisible();

  await page.keyboard.press("Enter");
  await expect(page.locator(".screen-message")).toHaveCount(0);
});

test("v opens the picker, and a consumer address is not offered a domain rule", async ({ page }) => {
  await shimIpc(page, { extra: [CONSUMER] });
  await openApp(page, { now: MIDDAY() });
  await openScreener(page);

  await expect(page.locator(".screen-card")).toHaveCount(4);

  // A company can carry a rule for everyone at it.
  await page.keyboard.press("v");
  await expect(page.locator('[role="dialog"]')).toHaveAttribute(
    "aria-label",
    "Where does their mail go?",
  );
  await expect(page.locator(".screen-option")).toHaveText(["Inbox", "Feed", "Paper Trail"]);
  await expect(page.locator(".toggle-label")).toHaveText("Everyone at harborlife.example");
  await page.keyboard.press("Escape");

  // Nobody at gmail.com is one sender, and `state::write::set_rule` refuses the rule, so the
  // toggle is not offered rather than offered and taken back.
  await page.locator(".screen-card", { hasText: "Nadia Okafor" }).click();
  await page.keyboard.press("v");
  await expect(page.locator(".screen-option")).toHaveCount(3);
  await expect(page.locator(".toggle-label")).toHaveCount(0);

  await page.locator(".screen-option", { hasText: "Paper Trail" }).click();
  await expect.poll(async () => (await toast(page))?.text).toBe("Screened in to Paper Trail");
});

test("n screens the sender out and sends nothing", async ({ page }) => {
  await shimIpc(page, { record: true });
  await openApp(page, { now: MIDDAY() });
  await openScreener(page);

  await page.keyboard.press("n");
  await expect(page.locator(".screen-card")).toHaveCount(2);

  // Plain words about what happened, and nothing that congratulates anybody.
  await expect.poll(async () => (await toast(page))?.text).toBe("Screened out");

  const sent = await calls(page);
  const decided = sent.filter((c) => c.command === "screener_decide");
  expect(decided).toHaveLength(1);
  expect(decided[0].args).toMatchObject({
    address: "todd@harborlife.example",
    destination: "screened-out",
    wholeDomain: false,
  });
  // Nothing is sent. Not a draft, not a message, not an unsubscribe on the way out.
  expect(
    sent.filter((c) => /^(send|draft_save|unsubscribe)/.test(c.command)).map((c) => c.command),
  ).toEqual([]);
});

test("Clear all asks before it screens everybody out", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await openScreener(page);

  await page.locator(".screen-tools .button", { hasText: "Clear all" }).click();
  await expect(page.locator(".confirm-title")).toHaveText("Screen out three senders?");
  // Asking is asking: nobody has left the pile yet.
  await expect(page.locator(".screen-card")).toHaveCount(3);

  await page.locator(".confirm-actions .button", { hasText: "Cancel" }).click();
  await expect(page.locator(".screen-card")).toHaveCount(3);

  await page.locator(".screen-tools .button", { hasText: "Clear all" }).click();
  await page.locator(".confirm-actions .button", { hasText: "Screen them out" }).click();
  await expect(page.locator(".screen-card")).toHaveCount(0);
  await expect(page.locator(".empty-state")).toHaveText("No one is waiting");
  await expect.poll(async () => (await toast(page))?.text).toBe("3 senders screened out");
});

test("the Inbox pill counts senders, prints its key, and goes when nobody is waiting", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const pill = page.locator(".list-head .pill");
  await expect(pill).toHaveText(/Screen 3 new senders/);
  // The only count anywhere in this app, and it counts senders rather than messages.
  await expect(pill.locator(".key")).toHaveText("6");

  await pill.click();
  expect(await place(page)).toBe("screener");

  await page.locator(".screen-tools .button", { hasText: "Clear all" }).click();
  await page.locator(".confirm-actions .button", { hasText: "Screen them out" }).click();
  await expect(page.locator(".screen-card")).toHaveCount(0);

  await page.keyboard.press("1");
  await listReady(page);
  await expect(page.locator(".list-head .pill")).toHaveCount(0);
});

// One picture per test rather than a loop, because `openApp` seeds the store once per context and
// a second call on the same page would take the dark picture in the light palette.
for (const theme of ["light", "dark"] as const) {
  test(`the Screener in ${theme}`, async ({ page }) => {
    mkdirSync(shots, { recursive: true });
    await openApp(page, { theme, now: MIDDAY() });
    await openScreener(page);
    await page.evaluate(() => document.fonts.ready);
    await settle(page);
    await page.screenshot({ path: join(shots, `screener-${theme}.png`) });
  });
}
