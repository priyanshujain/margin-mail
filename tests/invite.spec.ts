// The invitation card, and the permission it needs.
//
// The fixture's work account was granted everything except Calendar, which is what makes the piano
// thread's invitation render read only with a Grant on it. Answering one therefore needs an account
// that was granted the scope, and the fixture has no invitation on the account that was, so this
// spec shims `accounts_list` the way `failCommands` shims a refusal: the module request is answered
// with a wrapper around the real fixture. Nothing in `src/` knows it happened.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { listReady, MIDDAY, openApp, settle, toast } from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

const CALENDAR = "https://www.googleapis.com/auth/calendar.events";

/** Every account, as it would read after somebody had granted the Calendar scope. */
async function withCalendarGranted(page: Page): Promise<void> {
  // Granted by running the grant, rather than by dressing up the account list. `account_grant` is
  // what the card's own Grant button calls, and the fixture now really does widen the account it
  // names, so this exercises the path instead of standing in for it.
  await page.route("**/src/dev/mockIpc.ts*", async (route) => {
    if (new URL(route.request().url()).searchParams.has("real")) return route.fallback();
    await route.fulfill({
      contentType: "text/javascript",
      body: [
        `import { mockCall as real } from "/src/dev/mockIpc.ts?real";`,
        `const SCOPE = ${JSON.stringify(CALENDAR)};`,
        `for (const account of await real("accounts_list")) {`,
        `  await real("account_grant", { accountId: account.id, extraScopes: [SCOPE] });`,
        `}`,
        `export const mockCall = real;`,
      ].join("\n"),
    });
  });
}

/** Makes `invite_respond` fail the way Rust fails when the scope was withheld. */
async function refuseForScope(page: Page): Promise<void> {
  await page.route("**/src/dev/mockIpc.ts*", async (route) => {
    if (new URL(route.request().url()).searchParams.has("real")) return route.fallback();
    await route.fulfill({
      contentType: "text/javascript",
      body: [
        `import { mockCall as real } from "/src/dev/mockIpc.ts?real";`,
        `const SCOPE = ${JSON.stringify(CALENDAR)};`,
        `export async function mockCall(command, args) {`,
        `  if (command === "invite_respond") throw new Error("missing scope " + SCOPE);`,
        `  const out = await real(command, args);`,
        `  if (command !== "accounts_list") return out;`,
        `  return out.map((a) =>`,
        `    a.grantedScopes.includes(SCOPE) ? a : { ...a, grantedScopes: [...a.grantedScopes, SCOPE] },`,
        `  );`,
        `}`,
      ].join("\n"),
    });
  });
}

/** Every command that went through the fixture, on `window.__calls`. */
async function recordCalls(page: Page): Promise<void> {
  await page.route("**/src/dev/mockIpc.ts*", async (route) => {
    if (new URL(route.request().url()).searchParams.has("real")) return route.fallback();
    await route.fulfill({
      contentType: "text/javascript",
      body: [
        `import { mockCall as real } from "/src/dev/mockIpc.ts?real";`,
        `window.__calls = [];`,
        `export function mockCall(command, args) {`,
        `  window.__calls.push({ command, args: args ?? {} });`,
        `  return real(command, args);`,
        `}`,
      ].join("\n"),
    });
  });
}

const openPiano = async (page: Page) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.locator(".row", { hasText: "Piano on Wednesdays" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await expect(page.locator(".invite")).toBeVisible();
  await settle(page);
};

test("the invitation renders with its date, its time and its organiser", async ({ page }) => {
  await openPiano(page);

  const card = page.locator(".invite");
  await expect(card.locator(".invite-title")).toHaveText("Piano lesson: Cooper");
  // A calendar page: the month above the day, so the date is read rather than parsed.
  // Whatever the locale calls September in three or four letters, which is "Sept" in en-GB.
  await expect(card.locator(".invite-date .mon")).toHaveText(/^Sept?$/);
  await expect(card.locator(".invite-date .day")).toHaveText(/^\d{1,2}$/);
  await expect(card.locator(".invite-when")).toContainText("17:00 to 17:45");
  await expect(card.locator(".invite-when")).toContainText("Sunny Day Music, Bandra");
  await expect(card.locator(".invite-organizer")).toHaveText("Organised by Sunny Day Music");
  await expect(card.locator(".invite-open")).toHaveText("Open in Margin Calendar");

  // It is under the message that carried it, not floating at the end of the thread.
  const message = page.locator(".msg", { has: page.locator(".invite") });
  await expect(message).toHaveCount(1);
});

test("y on a focused card answers, and the three keys are printed on it", async ({ page }) => {
  await withCalendarGranted(page);
  await openPiano(page);

  const card = page.locator(".invite");
  const keys = await card.locator(".button .key").allTextContents();
  expect(keys).toEqual(["y", "m", "n"]);

  await card.click();
  await page.keyboard.press("y");

  await expect(card.locator(".invite-answered")).toContainText("Going");
  await expect.poll(async () => (await toast(page))?.text).toContain("Going · Piano lesson: Cooper");
});

test("m and n mean maybe and decline on the same card", async ({ page }) => {
  await withCalendarGranted(page);
  await openPiano(page);

  const card = page.locator(".invite");
  await card.click();
  await page.keyboard.press("m");
  await expect(card.locator(".invite-answered")).toContainText("Maybe");

  await page.keyboard.press("n");
  await expect(card.locator(".invite-answered")).toContainText("Not going");
});

test("the card's keys are the card's, and the list's meaning of y is not reached through it", async ({
  page,
}) => {
  await withCalendarGranted(page);
  await openPiano(page);

  await page.locator(".invite").click();
  await page.keyboard.press("y");
  // `y` in the list is a note, and a note sheet opening over an accepted invitation would be the
  // one thing docs/keyboard.md says must not happen.
  await expect(page.locator('[role="dialog"][aria-label="Note to self"]')).toHaveCount(0);
});

test("without the Calendar permission the card says so in one sentence and offers Grant", async ({
  page,
}) => {
  await openPiano(page);

  const card = page.locator(".invite");
  // Read only, and still a card: the date, the time and the organiser are the point of it.
  await expect(card.locator(".invite-title")).toHaveText("Piano lesson: Cooper");
  await expect(card.locator(".invite-when")).toContainText("17:00 to 17:45");

  const sentence = card.locator(".invite-scope");
  await expect(sentence).toHaveText(
    "Answering an invitation needs Google Calendar, which this account has not given Margin yet.",
  );
  await expect(sentence).toHaveText(/^[^.]+\.$/);

  await expect(card.locator(".button")).toHaveCount(1);
  await expect(card.locator(".button")).toHaveText("Grant");

  // The keys are not the card's while it cannot answer, so `y` is the list's note again.
  await card.click();
  await page.keyboard.press("y");
  await expect(page.locator('[role="dialog"][aria-label="Note to self"]')).toBeVisible();
});

test("a refusal that names the scope turns the card read only", async ({ page }) => {
  await refuseForScope(page);
  await openPiano(page);

  const card = page.locator(".invite");
  await expect(card.locator(".invite-scope")).toHaveCount(0);

  await card.click();
  await page.keyboard.press("y");

  // The scope list said it could and the call said it could not, and the call is the one that
  // knows: no toast about a failure, one sentence and the way to fix it.
  await expect(card.locator(".invite-scope")).toBeVisible();
  await expect(card.locator(".button")).toHaveText("Grant");
  await expect(card.locator(".invite-answered")).toHaveCount(0);
});

test("Grant asks for the Calendar scope on the account the thread is in", async ({ page }) => {
  await recordCalls(page);
  await openPiano(page);

  await page.locator(".invite .button", { hasText: "Grant" }).click();

  const grant = () =>
    page.evaluate(() =>
      (
        window as unknown as { __calls: { command: string; args: Record<string, unknown> }[] }
      ).__calls.find((call) => call.command === "account_grant"),
    );

  await expect.poll(grant).toBeTruthy();
  const call = await grant();
  expect(call?.args.accountId).toBe("acct-1");
  expect(call?.args.extraScopes).toEqual([CALENDAR]);
});

test("a thread with an invite card", async ({ page }) => {
  mkdirSync(shots, { recursive: true });
  await openPiano(page);
  await page.locator(".invite").click();
  await settle(page);
  await page.screenshot({ path: join(shots, "invite.png") });
});
