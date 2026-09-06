// More than one account at a time: the chip that switches between them, the merged list behind
// Ctrl+0, and the one thing the chip is allowed to say about sync.
//
// The fixture has two accounts on two hues, which is what makes the coloured edge in All accounts
// something a test can read rather than something a person has to be told is there.

import { expect, test, type Page } from "@playwright/test";
import { failCommands, listReady, MIDDAY, openApp, rows, settle } from "./app";

const chip = (page: Page) => page.locator(".account-chip");

/** Settings, on the Remove an account sheet, with the confirmation for one row's button up. */
async function askToRemove(page: Page, row: number) {
  await page.keyboard.press("Meta+,");
  await page.getByRole("button", { name: "Remove an account" }).click();
  const sheet = page.getByRole("dialog", { name: "Remove an account" });
  await sheet.locator(".remove-row").nth(row).getByRole("button", { name: "Remove" }).click();
  await expect(sheet.locator(".confirm")).toBeVisible();
  return sheet;
}

/** The account edge on every row: the 2px rule the list draws in All accounts and nowhere else. */
const edges = (page: Page) =>
  page.locator(".row-account").evaluateAll((marks) =>
    marks.map((mark) => mark.getAttribute("data-hue") ?? ""),
  );

/**
 * A sync status arriving from the backend. In the browser the fixture dispatches these as window
 * events under the Tauri names, so this is the backend's own surface rather than the app's.
 */
async function saySync(page: Page, accountId: string, phase: string): Promise<void> {
  await page.evaluate(
    ({ accountId, phase }) => {
      window.dispatchEvent(
        new CustomEvent("sync-progress", {
          detail: {
            accountId,
            phase,
            lastSyncMs: null,
            error: phase === "error" ? "the token was revoked" : null,
            pendingWrites: 0,
            // The word is the engine's: a refused token arrives already named, and the chip only
            // prints it. An error with no name is "Sync trouble", which is the rule useSync pins.
            message: phase === "error" ? "Signed out" : null,
            hydrated: 300,
            total: 1200,
            oldestMs: null,
          },
        }),
      );
    },
    { accountId, phase },
  );
  await settle(page);
}

test("the chip lists both accounts with their colours, and All accounts behind them", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await expect(chip(page)).toContainText("pj@73ai.org");
  // The chip wears the account's own colour rather than the one its address hashes to.
  await expect(chip(page).locator(".avatar")).toHaveAttribute("data-hue", "4");

  await chip(page).click();
  const options = page.locator(".account-option");
  await expect(options).toHaveCount(3);
  await expect(options.nth(0)).toContainText("pj@73ai.org");
  await expect(options.nth(0).locator(".avatar")).toHaveAttribute("data-hue", "4");
  await expect(options.nth(1)).toContainText("priyanshujain@gmail.com");
  await expect(options.nth(1).locator(".avatar")).toHaveAttribute("data-hue", "2");
  await expect(options.nth(2)).toContainText("All accounts");

  // Four ways into settings and the chip is one of them.
  await page.locator(".account-settings").click();
  await expect(page.locator(".settings")).toBeVisible();
  await expect(page.locator(".settings-title")).toHaveText("Accounts");
});

test("Ctrl and a number switch the account the list is showing", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const first = await rows(page);
  expect(first.length).toBeGreaterThan(0);

  await page.keyboard.press("Control+2");
  await expect(chip(page)).toContainText("priyanshujain@gmail.com");
  await expect(chip(page).locator(".avatar")).toHaveAttribute("data-hue", "2");
  await listReady(page);

  // A different mailbox, not the same one relabelled.
  const second = await rows(page);
  expect(second.map((row) => row.subject)).not.toEqual(first.map((row) => row.subject));
  // One account at a time carries no edge: the colour only answers a question All accounts asks.
  expect(await edges(page)).toEqual([]);

  await page.keyboard.press("Control+1");
  await expect(chip(page)).toContainText("pj@73ai.org");
});

test("Ctrl+0 is All accounts, and every row wears its account's colour", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("Control+0");
  await expect(chip(page)).toContainText("All accounts");
  await listReady(page);

  const merged = await rows(page);
  const found = await edges(page);
  // One edge per row, and both accounts are in the one list.
  expect(found).toHaveLength(merged.length);
  expect(new Set(found)).toEqual(new Set(["4", "2"]));

  // Two pixels, which is what docs/ui.md pins the edge at.
  const width = await page
    .locator(".row-account")
    .first()
    .evaluate((mark) => mark.getBoundingClientRect().width);
  expect(width).toBeCloseTo(2, 1);
});

test("the chip says so when an account is offline, signed out or paused", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  // Nothing is said while sync is working, which is the whole rule.
  await expect(page.locator(".sync-note")).toHaveCount(0);
  await saySync(page, "acct-1", "syncing");
  await expect(page.locator(".sync-note")).toHaveCount(0);

  await saySync(page, "acct-1", "error");
  await expect(page.locator(".sync-note")).toHaveText("Signed out");

  await saySync(page, "acct-1", "paused");
  await expect(page.locator(".sync-note")).toHaveText("Paused");

  await saySync(page, "acct-1", "offline");
  await expect(page.locator(".sync-note")).toHaveText("Offline");

  // The other account is fine, so its chip says nothing.
  await saySync(page, "acct-1", "idle");
  await saySync(page, "acct-2", "error");
  await expect(page.locator(".sync-note")).toHaveCount(0);
  await page.keyboard.press("Control+2");
  await expect(page.locator(".sync-note")).toHaveText("Signed out");

  // And All accounts speaks for whichever of them is broken.
  await page.keyboard.press("Control+0");
  await expect(page.locator(".sync-note")).toHaveText("Signed out");
});

test("removing an account holds the confirmation and says what it is doing", async ({ page }) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);
  const sheet = await askToRemove(page, 1);

  // One button per row: there is no Revoke to weigh against Remove any more. The question says
  // what the button costs at Google, and the box that deletes the local data starts ticked.
  await expect(sheet.locator(".remove-row")).toHaveCount(0);
  await expect(sheet.locator(".confirm-body")).toContainText("every machine");
  await expect(sheet.getByRole("checkbox")).toBeChecked();

  await sheet.getByRole("button", { name: "Remove" }).click();

  // Cancel cannot take back a delete that is already running, and neither can Escape.
  const removing = sheet.getByRole("button", { name: "Removing" });
  await expect(removing).toBeVisible();
  await expect(removing).toBeDisabled();
  await expect(sheet).toHaveAttribute("data-busy", "");
  await expect(sheet.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await expect(sheet.getByRole("checkbox")).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(sheet).toBeVisible();

  await expect(sheet).toHaveCount(0);
  await expect(page.locator(".acct")).toHaveCount(1);
  await expect(page.locator(".toast-text")).toContainText("was removed and Margin's access at Google revoked");
  await expect(page.locator(".toast-text")).not.toContainText("stay on this computer");
});

test("unticking the box keeps the mail on this computer and still revokes", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  const sheet = await askToRemove(page, 0);
  await sheet.getByRole("checkbox").uncheck();
  await sheet.getByRole("button", { name: "Remove" }).click();
  await expect(sheet).toHaveCount(0);
  await expect(page.locator(".toast-text")).toContainText("Margin's access at Google revoked");
  await expect(page.locator(".toast-text")).toContainText("stay on this computer");
  await expect(page.locator(".acct")).toHaveCount(1);
});

test("a removal the backend refuses is a failure, not a removed account", async ({ page }) => {
  await failCommands(page, ["account_remove"]);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  const sheet = await askToRemove(page, 0);
  await sheet.getByRole("button", { name: "Remove" }).click();

  await expect(page.locator(".toast-text")).toContainText("Could not remove that account");
  // Still on the question, with both accounts where they were.
  await expect(sheet.locator(".confirm")).toBeVisible();
  await expect(sheet.getByRole("button", { name: "Remove" })).toBeEnabled();
  await expect(page.locator(".acct")).toHaveCount(2);
});
