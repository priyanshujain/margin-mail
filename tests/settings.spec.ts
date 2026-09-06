// Settings, which is a place rather than a panel: it takes the stage, the header stays, and Escape
// gives the stage back.
//
// All twelve sections are built, and the rule this suite holds them to is the one in docs/plan.md:
// a section is real controls over real commands or it is not a section. So every one of them is
// opened and asked to show the control it is for, and the two that change something on disk before
// they change the screen are driven all the way through.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { failCommands, listReady, MIDDAY, openApp, settle } from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

const SECTIONS = [
  "Accounts",
  "Appearance",
  "Mail",
  "Privacy",
  "Screener",
  "Piles and snooze",
  "Writing",
  "Notifications",
  "Keyboard",
  "Backup",
  "Data",
  "About",
];

async function openSettings(page: Page, storage?: Record<string, string>): Promise<void> {
  await openApp(page, { now: MIDDAY(), storage });
  await listReady(page);
  await page.keyboard.press("Meta+,");
  await expect(page.locator(".settings")).toBeVisible();
}

const rail = (page: Page) =>
  page.locator(".settings-tab").evaluateAll((tabs) => tabs.map((tab) => tab.textContent ?? ""));

test("Cmd+, opens settings on Accounts, with every section in the rail", async ({ page }) => {
  await openSettings(page);

  expect(await rail(page)).toEqual(SECTIONS);
  await expect(page.locator(".settings-tab[data-active]")).toHaveText("Accounts");
  await expect(page.locator(".settings-title")).toHaveText("Accounts");

  // The stage, not an overlay: the header is still there and nothing is modal.
  await expect(page.locator(".account-chip")).toBeVisible();
  await expect(page.locator('[role="dialog"]')).toHaveCount(0);
  await expect(page.locator(".list-col")).toHaveCount(0);

  mkdirSync(shots, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "settings.png") });

  // Escape gives the stage back to the mail.
  await page.keyboard.press("Escape");
  await expect(page.locator(".list-col")).toBeVisible();
});

test("j and k walk the rail", async ({ page }) => {
  await openSettings(page);

  await page.keyboard.press("j");
  await expect(page.locator(".settings-tab[data-active]")).toHaveText("Appearance");
  await page.keyboard.press("j");
  await expect(page.locator(".settings-tab[data-active]")).toHaveText("Mail");
  await expect(page.locator(".settings-title")).toHaveText("Mail");

  await page.keyboard.press("k");
  await expect(page.locator(".settings-tab[data-active]")).toHaveText("Appearance");
});

/**
 * The control each section exists for. About is the one that is a statement rather than a form, so
 * what it owes the reader is the version string a bug report carries.
 */
const CONTROLS: [string, string][] = [
  ["Accounts", ".acct .swatch"],
  ["Appearance", '[role="tablist"][aria-label="Theme"] [role="tab"]'],
  ["Mail", '[role="tablist"][aria-label^="Storage window"] [role="tab"]'],
  ["Privacy", '[role="tablist"][aria-label="Remote images"] [role="tab"]'],
  ["Screener", '[role="switch"]'],
  ["Piles and snooze", 'select[aria-label="Tomorrow morning"]'],
  ["Writing", '[role="tablist"][aria-label="Undo delay"] [role="tab"]'],
  ["Notifications", '[role="switch"]'],
  ["Keyboard", 'button:has-text("Open the file")'],
  ["Backup", '[role="tablist"][aria-label="Backup store"] [role="tab"]'],
  ["Data", 'button:has-text("Export")'],
  ["About", ".settings-version"],
];

// One switch is the system's permission and the app's preference together, and the rest of the
// section exists only while it is on.
test("notifications are one switch, and the rest only once it is on", async ({ page }) => {
  await openSettings(page, { "marginmail-dev-notify": "denied" });
  await page.locator(".settings-tab", { hasText: "Notifications" }).click();
  const allow = page.getByRole("switch", { name: "Allow notifications" });
  const inbox = page.getByRole("switch", { name: "Notify about the Inbox" });

  // Refused by the system: off and locked, one line says so, one button fixes it, nothing else.
  await expect(allow).toBeDisabled();
  await expect(allow).toHaveAttribute("aria-checked", "false");
  await expect(page.locator('[data-permission="denied"]')).toContainText(
    "turned off for Margin Mail in System Settings",
  );
  await expect(page.getByRole("button", { name: "Open System Settings" })).toBeVisible();
  await expect(inbox).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Send a test notification" })).toHaveCount(0);
  await page.screenshot({ path: join(shots, "settings-notifications-denied.png") });

  // Never asked: off, and turning it on asks. A yes brings the rest with it, the Inbox included.
  await page.evaluate(() => localStorage.setItem("marginmail-dev-notify", "prompt"));
  await page.locator(".settings-tab", { hasText: "Writing" }).click();
  await page.locator(".settings-tab", { hasText: "Notifications" }).click();
  await expect(allow).toBeEnabled();
  await expect(allow).toHaveAttribute("aria-checked", "false");
  await expect(inbox).toHaveCount(0);
  await allow.click();
  await expect(allow).toHaveAttribute("aria-checked", "true");
  await expect(inbox).toHaveAttribute("aria-checked", "true");
  await expect(page.getByRole("switch", { name: "Notify about the Feed" })).toHaveAttribute(
    "aria-checked",
    "false",
  );
  await expect(page.getByRole("button", { name: "Send a test notification" })).toBeEnabled();
  await page.screenshot({ path: join(shots, "settings-notifications.png") });

  // Off hides the rest again.
  await allow.click();
  await expect(allow).toHaveAttribute("aria-checked", "false");
  await expect(inbox).toHaveCount(0);
});

test("every section in the rail opens on real controls", async ({ page }) => {
  await openSettings(page);

  for (const [name, control] of CONTROLS) {
    await page.locator(".settings-tab", { hasText: name }).click();
    await expect(page.locator(".settings-title")).toHaveText(name);
    await expect(page.locator(`.settings-inner ${control}`).first()).toBeVisible();
  }

  // Nothing that cannot work: the recovery phrase is not offered a second time, and there is no
  // field for an OAuth client of your own, because no command takes one.
  await page.locator(".settings-tab", { hasText: "Accounts" }).click();
  await expect(page.getByRole("button", { name: /OAuth/i })).toHaveCount(0);
});

test("Accounts shows both accounts, their permissions, and a Grant for the missing one", async ({
  page,
}) => {
  await openSettings(page);

  await expect(page.locator(".acct")).toHaveCount(2);
  await expect(page.locator(".acct-addr").first()).toHaveText("pj@73ai.org");
  await expect(page.locator(".acct-addr").nth(1)).toHaveText("priyanshujain@gmail.com");

  // Plain English, never the name of the scope.
  const first = page.locator(".acct").first();
  await expect(first.locator(".scope .what").first()).toHaveText("Read and change your mail");
  await expect(first.locator(".scope")).toHaveCount(6);
  await expect(first.locator(".scope .state")).toHaveCount(5);

  // One permission is missing on the first account, so exactly one line carries a Grant and says
  // what it costs.
  const grant = first.getByRole("button", { name: "Grant" });
  await expect(grant).toHaveCount(1);
  await expect(first.locator(".scope[data-missing] .what")).toHaveText(
    "Answer calendar invitations",
  );
  await expect(first.locator(".scope-why")).toContainText("Calendar is not connected");

  // The second account granted everything, so it does not list six lines to say so.
  await expect(page.locator(".acct").nth(1).locator(".scope")).toHaveCount(0);
  await expect(page.locator(".acct").nth(1)).toContainText("All granted");

  // The colour is pickable, and the avatar wears the one that was picked.
  await first.locator('.swatch[data-hue="7"]').click();
  await expect(first.locator(".swatch[data-on]")).toHaveAttribute("data-hue", "7");
  await expect(first.locator(".avatar")).toHaveAttribute("data-hue", "7");

  // Removing revokes at Google as well, and the question says what that costs before it runs. The
  // only choice left is whether the local data goes too, and the box for that starts ticked.
  await page.getByRole("button", { name: "Remove an account" }).click();
  await expect(page.getByRole("dialog", { name: "Remove an account" })).toBeVisible();
  await expect(page.locator(".remove-row").first().getByRole("button", { name: "Revoke" })).toHaveCount(0);
  await page.locator(".remove-row").first().getByRole("button", { name: "Remove" }).click();
  await expect(page.locator(".confirm-body")).toContainText("Margin Calendar");
  await expect(page.locator(".confirm-body")).toContainText("every machine");
  await expect(page.locator(".confirm-option input")).toBeChecked();
});

test("Add account asks for the address and works the rest out", async ({ page }) => {
  await openSettings(page);

  await page.getByRole("button", { name: "Add account" }).click();
  const sheet = page.getByRole("dialog", { name: "Add an account" });
  await expect(sheet).toBeVisible();

  // Nothing has been started yet and the browser has not been opened: the button that opens this
  // used to be Google's, so a Proton or a Fastmail address was sent to a Google consent page. Now
  // it is one field, and no provider is named as a button.
  await expect(page.locator(".settings-waiting")).toHaveCount(0);
  await expect(sheet.getByLabel("Email address")).toBeFocused();
  await expect(sheet.getByRole("button", { name: "Continue" })).toBeVisible();
  await expect(sheet.getByRole("button", { name: /Google/ })).toHaveCount(0);

  mkdirSync(shots, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "settings-add-account.png") });

  // A Google address is still one press past the address and no typing, and it still hands the
  // flow to the browser. The sheet gives the window up: the strip above the list is what waits.
  await sheet.getByLabel("Email address").fill("pj@gmail.com");
  await page.keyboard.press("Enter");
  await expect(sheet).toHaveCount(0);
  await expect(page.locator(".settings-waiting")).toContainText("Waiting for Google in your browser");
});

test("Add account holds a panel over the window until the mail is in, then opens that Inbox", async ({
  page,
}) => {
  await openSettings(page);
  await page.getByRole("button", { name: "Add account" }).click();
  await page.getByLabel("Email address").fill("work@gmail.com");
  await page.keyboard.press("Enter");

  // Consent comes back and the account is written. What used to happen next was nothing: the new
  // account sat under Settings on an empty Inbox with one line in the header. Now the wait for
  // its mail takes the window, in the engine's words, with the count.
  const panel = page.getByRole("dialog", { name: "Bringing in work@gmail.com" });
  await expect(panel).toBeVisible();
  // First the one question, because the first sync reads the answer: a year, this time.
  await expect(panel.locator(".window-choice")).toBeVisible();
  await expect(panel.locator(".arrive-bar")).toHaveCount(0);
  await panel.getByRole("radio", { name: "A year" }).click();
  await panel.getByRole("button", { name: "Start" }).click();
  await expect(panel.locator(".window-choice")).toHaveCount(0);
  await expect(panel.locator(".arrive-bar")).toBeVisible();
  await expect(panel).toContainText("Bringing in the last year");
  await expect(panel.locator(".arrive-count")).toContainText("of 4,812 messages");
  // Nothing closes it: not the close control, not Escape, not the scrim.
  await expect(panel.getByRole("button", { name: "Close" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(panel).toBeVisible();

  // The pass ends, the panel goes, and what is behind it is the new account's Inbox.
  await expect(panel).toHaveCount(0);
  await expect(page.locator(".settings")).toHaveCount(0);
  await expect(page.locator(".account-chip")).toContainText("work@gmail.com");
  await expect(page.locator(".app")).toHaveAttribute("data-place", "inbox");
  // And the first-run panel over it says what the pass over senders did.
  await expect(page.getByRole("dialog", { name: "You are set up" })).toBeVisible();
});

test("a first pass that stops leaves the panel with the reason and two ways on", async ({ page }) => {
  await openSettings(page, { "marginmail-dev-sync-fails": "1" });
  await page.getByRole("button", { name: "Add account" }).click();
  await page.getByLabel("Email address").fill("work@gmail.com");
  await page.keyboard.press("Enter");

  const panel = page.getByRole("dialog", { name: "Bringing in work@gmail.com" });
  await expect(panel).toBeVisible();
  await panel.getByRole("button", { name: "Start" }).click();
  await expect(panel).toContainText("refused the first request");
  await expect(panel.locator(".arrive-trouble")).toContainText("Gmail API has not been used");
  await expect(panel.getByRole("button", { name: "Try again" })).toBeVisible();

  // Going in anyway is an Inbox that says what it has, on the account that was added.
  await panel.getByRole("button", { name: "Go in anyway" }).click();
  await expect(panel).toHaveCount(0);
  await expect(page.locator(".account-chip")).toContainText("work@gmail.com");
  await expect(page.locator(".settings")).toHaveCount(0);
});

test("Add account tells an Outlook address why, in the sheet, with the way back", async ({
  page,
}) => {
  await openSettings(page);
  await page.getByRole("button", { name: "Add account" }).click();
  await page.getByLabel("Email address").fill("pj@outlook.com");
  await page.keyboard.press("Enter");

  // Outlook is the third mailbox the person this was built for has, and it does not work. A
  // sentence saying so, when the address is typed, is better than a button that fails.
  const sheet = page.getByRole("dialog", { name: "Outlook is not here yet" });
  await expect(sheet).toBeVisible();
  await expect(sheet).toContainText("app registration with Microsoft");
  await expect(page.locator(".settings-waiting")).toHaveCount(0);

  // The panel's own back control retraces the step, to the address, which is still there.
  await sheet.getByRole("button", { name: "Back to the address" }).click();
  await expect(page.getByRole("dialog", { name: "Add an account" })).toBeVisible();
  await expect(page.getByLabel("Email address")).toHaveValue("pj@outlook.com");
});

test("an IMAP account is its servers, and a Google account is still its permissions", async ({
  page,
}) => {
  await openSettings(page, { "marginmail-dev-imap": "1" });

  await expect(page.locator(".acct")).toHaveCount(3);
  const imap = page.locator(".acct").nth(2);
  await expect(imap.locator(".acct-addr")).toHaveText("pj@fastmail.example");

  // Nothing was ever granted, so there is no line offering to grant it. Every one of those buttons
  // used to open a Google consent page for an account that has no Google behind it.
  await expect(imap.locator(".scope")).toHaveCount(0);
  await expect(imap.getByRole("button", { name: "Grant" })).toHaveCount(0);

  // What it has instead: the two servers it is actually using, who logs in, and where the password
  // is. These come from imap_servers, so they are what the next connection will use.
  const facts = imap.locator(".acct-servers .set-row");
  await expect(facts).toHaveCount(4);
  await expect(facts.nth(0)).toContainText("imap.fastmail.example, port 993, TLS");
  await expect(facts.nth(1)).toContainText("smtp.fastmail.example, port 465, TLS");
  await expect(facts.nth(2)).toContainText("pj@fastmail.example");
  await expect(facts.nth(3)).toContainText("Sealed on this device");

  // Aliases are the provider's answer and there is no provider here to give one, so the line says
  // that rather than reporting a count of nothing.
  await expect(imap).toContainText("only ever sends as its own address");

  await imap.scrollIntoViewIfNeeded();
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "settings-imap-account.png") });

  // And the Google card is untouched: six lines, five granted, one Grant, and no servers.
  const google = page.locator(".acct").first();
  await expect(google.locator(".scope")).toHaveCount(6);
  await expect(google.locator(".scope .state")).toHaveCount(5);
  await expect(google.getByRole("button", { name: "Grant" })).toHaveCount(1);
  await expect(google.locator(".acct-servers")).toHaveCount(0);

  // There is a Google account here, so the footnote about the shared client is about something.
  await expect(page.getByText("share one Google client")).toBeVisible();
});

test("the shared Google client footnote is gone when no account is Google", async ({ page }) => {
  await openSettings(page, { "marginmail-dev-imap": "only" });

  await expect(page.locator(".acct")).toHaveCount(1);
  await expect(page.locator(".acct .acct-servers")).toBeVisible();

  // Revoking a grant nobody made is not a consequence anybody here has, and a paragraph explaining
  // it reads as though the app has quietly signed you in to something.
  await expect(page.getByText("share one Google client")).toHaveCount(0);
  await expect(page.locator(".scope")).toHaveCount(0);
});

test("Appearance changes the theme and the face, and both survive a reload", async ({ page }) => {
  await openSettings(page);
  await page.locator(".settings-tab", { hasText: "Appearance" }).click();

  await page.getByRole("tab", { name: "Dark" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

  await page.getByRole("tab", { name: "Light" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");

  // The text face writes --font-heading, which is a token change and touches nothing else.
  const picker = page.getByLabel("Text font");
  await picker.selectOption({ label: "EB Garamond" });
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.style.getPropertyValue("--font-heading")),
    )
    .toContain("EB Garamond");

  // And it is written where the boot script in index.html reads it before the first paint.
  const stored = await page.evaluate(() => localStorage.getItem("marginmail-fonts"));
  expect(stored).toContain("EB Garamond");

  await page.reload();
  await expect(page.locator(".list-col")).toBeVisible();
  const afterReload = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--font-heading"),
  );
  expect(afterReload).toContain("EB Garamond");
});

test("the storage window asks before it shrinks, and does not before it widens", async ({
  page,
}) => {
  await openSettings(page);
  await page.locator(".settings-tab", { hasText: "Mail" }).click();

  const work = page.getByRole("tablist", { name: "Storage window for pj@73ai.org" });
  const personal = page.getByRole("tablist", {
    name: "Storage window for priyanshujain@gmail.com",
  });

  // What the account holds now, and how far back it reaches.
  await expect(page.locator(".set-card").first()).toContainText("4,812 messages");
  await expect(page.locator(".set-card").first()).toContainText("back to");

  // Widening only ever adds, so it goes straight through.
  await work.getByRole("tab", { name: "90 days" }).click();
  await expect(page.locator('[role="dialog"]')).toHaveCount(0);
  await expect(work.getByRole("tab", { name: "90 days" })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  // Rust queues the backfill off the same write, and while it runs the section shows the same
  // thin bar the first sync draws.
  await page.evaluate(() => {
    window.dispatchEvent(
      new CustomEvent("sync-progress", {
        detail: {
          accountId: "acct-1",
          phase: "backfilling",
          lastSyncMs: null,
          error: null,
          pendingWrites: 0,
          message: "Bringing in the older mail",
          hydrated: 300,
          total: 1200,
          oldestMs: null,
        },
      }),
    );
  });
  const bar = page.locator('.set-filling [role="progressbar"]');
  await expect(bar).toHaveAttribute("aria-valuenow", "25");
  await expect(page.locator(".set-filling")).toContainText("Bringing in the older mail");

  // Narrowing evicts, so it says what it would take before it takes it.
  await personal.getByRole("tab", { name: "30 days" }).click();
  const asking = page.getByRole("dialog", { name: "Storage window" });
  await expect(asking).toBeVisible();
  await expect(asking.locator(".confirm-title")).toHaveText(
    "Keep only the last 30 days of priyanshujain@gmail.com on this device?",
  );
  await expect(asking.locator(".confirm-body")).toContainText("older than 30 days");
  await expect(asking.locator(".confirm-body")).toContainText("Nothing is removed from Gmail");
  await expect(asking.locator(".confirm-body")).toContainText("kept whatever their age");

  // Cancelling leaves the window where it was.
  await asking.getByRole("button", { name: "Cancel" }).click();
  await expect(personal.getByRole("tab", { name: "90 days" })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  await personal.getByRole("tab", { name: "30 days" }).click();
  await page.getByRole("button", { name: "Narrow the window" }).click();
  await expect(page.locator('[role="dialog"]')).toHaveCount(0);
  await expect(personal.getByRole("tab", { name: "30 days" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});

test("the interface font is the second slot, and it round trips like the first", async ({
  page,
}) => {
  await openSettings(page);
  await page.locator(".settings-tab", { hasText: "Appearance" }).click();

  await page.getByLabel("Interface font").selectOption({ label: "Fraunces" });
  await expect
    .poll(() => page.evaluate(() => document.documentElement.style.getPropertyValue("--font-ui")))
    .toContain("Fraunces");

  // Away and back: what comes back is what the command answered with, not what the click guessed.
  await page.locator(".settings-tab", { hasText: "Data" }).click();
  await page.locator(".settings-tab", { hasText: "Appearance" }).click();
  await expect(page.getByLabel("Interface font")).toHaveValue(/fraunces/i);
  await expect(page.getByLabel("Text font")).toHaveValue(/literata/i);
});

test("the undo delay round trips through the command", async ({ page }) => {
  await openSettings(page);
  await page.locator(".settings-tab", { hasText: "Writing" }).click();

  const delay = page.getByRole("tablist", { name: "Undo delay" });
  await expect(delay.getByRole("tab", { name: "10s" })).toHaveAttribute("aria-selected", "true");

  await delay.getByRole("tab", { name: "20s" }).click();
  await page.locator(".settings-tab", { hasText: "Screener" }).click();
  await page.locator(".settings-tab", { hasText: "Writing" }).click();
  await expect(delay.getByRole("tab", { name: "20s" })).toHaveAttribute("aria-selected", "true");
});

test("Backup says the recovery phrase cannot be shown twice, and why", async ({ page }) => {
  await openSettings(page);
  await page.locator(".settings-tab", { hasText: "Backup" }).click();

  await expect(page.locator(".settings-inner")).toContainText("Backing up to Google Drive");
  await expect(page.locator(".settings-inner")).toContainText("cannot be shown again");
  // The mechanism rather than a rule: the sentence has to say why, not apologise.
  await expect(page.locator(".settings-inner")).toContainText("the phrase itself was dropped");

  // And no button that could not work if it were pressed.
  await expect(page.getByRole("button", { name: /phrase/i })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Restore" })).toBeVisible();
});

test("Data says what clearing the mirror does and does not throw away", async ({ page }) => {
  await openSettings(page);
  await page.locator(".settings-tab", { hasText: "Data" }).click();

  await expect(page.locator(".settings-inner")).toContainText("Storage used");

  await page.getByRole("button", { name: "Clear the mirror for pj@73ai.org" }).click();
  const asking = page.getByRole("dialog", { name: "Clear the mirror" });
  await expect(asking.locator(".confirm-title")).toContainText("pj@73ai.org");
  await expect(asking.locator(".confirm-body")).toContainText("Nothing is removed from Gmail");
  await expect(asking.locator(".confirm-body")).toContainText("Every decision you have made is kept");

  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.keyboard.press("Escape");
  await expect(page.locator('[role="dialog"]')).toHaveCount(0);
});

test("the Mail section, for the eye", async ({ page }) => {
  await openSettings(page);
  await page.locator(".settings-tab", { hasText: "Mail" }).click();
  await expect(page.locator(".set-card")).toHaveCount(2);

  mkdirSync(shots, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "settings-mail.png") });
});

/**
 * Makes the fixture answer as a device that has never had a recovery phrase, which is the only
 * state the phrase can be shown in and the one the seeded fixture is past. The module request is
 * answered with a shim over the real one, the same way `failCommands` does it.
 */
async function withoutAPhrase(page: Page): Promise<void> {
  await page.route("**/src/dev/mockIpc.ts*", async (route) => {
    if (new URL(route.request().url()).searchParams.has("real")) return route.fallback();
    await route.fulfill({
      contentType: "text/javascript",
      body: [
        `import { mockCall as real } from "/src/dev/mockIpc.ts?real";`,
        `export async function mockCall(command, args) {`,
        `  const out = await real(command, args);`,
        `  if (command === "settings_get" || command === "settings_set")`,
        `    return { ...out, backup: { ...out.backup, hasPhrase: false } };`,
        `  if (command === "backup_configure") return { ...out, hasPhrase: false };`,
        `  return out;`,
        `}`,
      ].join("\n"),
    });
  });
}

test("turning backup on shows the phrase, once, with what it is for", async ({ page }) => {
  await withoutAPhrase(page);
  await openSettings(page);
  await page.locator(".settings-tab", { hasText: "Backup" }).click();

  // Not offered before there is a backup to have one for.
  await expect(page.locator(".phrase")).toHaveCount(0);

  await page.getByRole("tab", { name: "Google Drive" }).click();
  await expect(page.locator(".phrase")).toHaveText(/(\w+ ){11}\w+/);
  await expect(page.locator(".settings-inner")).toContainText("only time they will be on a screen");

  mkdirSync(shots, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "settings-backup.png") });
});

test("the name and the signature are editable, and the signature is one field in two places", async ({
  page,
}) => {
  await openSettings(page);
  const first = page.locator(".acct").first();

  const name = first.getByLabel("Name for pj@73ai.org");
  await name.fill("Work");
  await name.blur();
  await expect(name).toHaveValue("Work");
  // The account registry owns the name, so the avatar it draws everywhere follows it.
  await expect(first.locator(".avatar")).toHaveText("W");

  const signature = first.getByLabel("Signature for pj@73ai.org");
  await signature.fill("Priyanshu, 73ai");
  await signature.blur();

  // docs/settings.md puts the same field in Writing, because that is where somebody writing a
  // signature looks for it.
  await page.locator(".settings-tab", { hasText: "Writing" }).click();
  await expect(page.getByLabel("Signature for pj@73ai.org")).toHaveValue("Priyanshu, 73ai");
});

// The three exports and the import each hold the database lock for as long as it takes to walk
// the mirror. Under the pending flag the fixture takes most of a second over them, which is long
// enough to see that the button that was pressed says so and cannot be pressed again.
test("an export says it is writing and cannot be pressed twice", async ({ page }) => {
  await openSettings(page, { "marginmail-dev-pending": "1" });
  await page.locator(".settings-tab", { hasText: "Data" }).click();

  const section = page.locator(".set-section");
  await expect(section).toHaveAttribute("data-phase", "idle");

  const mbox = page.getByRole("button", { name: "Export pj@73ai.org as mbox" });
  const other = page.getByRole("button", { name: "Export priyanshujain@gmail.com as mbox" });
  const state = page.locator(".set-stack", { hasText: "App state" }).getByRole("button");
  const exporting = state.first();
  const importing = state.nth(1);

  await mbox.click();
  await expect(section).toHaveAttribute("data-phase", "exporting");
  await expect(mbox).toHaveText("Writing");
  await expect(mbox).toBeDisabled();
  // The same lock, so the other exports wait for it rather than queueing a second file.
  await expect(other).toBeDisabled();
  await expect(exporting).toBeDisabled();

  await expect(section).toHaveAttribute("data-phase", "idle");
  await expect(mbox).toHaveText("pj@73ai.org");
  await expect(mbox).toBeEnabled();
  await expect(page.locator(".toast-text")).toContainText("Written to");

  await exporting.click();
  await expect(exporting).toHaveText("Writing");
  await expect(exporting).toBeDisabled();
  await expect(mbox).toBeDisabled();
  await expect(exporting).toHaveText("Export");

  const path = page.getByLabel("App state file to import");
  await path.fill("/Users/you/Downloads/margin-mail-state.json");
  await importing.click();
  await expect(section).toHaveAttribute("data-phase", "importing");
  await expect(importing).toHaveText("Importing");
  await expect(importing).toBeDisabled();
  await expect(section).toHaveAttribute("data-phase", "idle");
  await expect(importing).toHaveText("Import");
  await expect(path).toHaveValue("");
  await expect(page.locator(".toast-text")).toContainText("was imported");
});

test("the backup store moves as it is pressed, and each wait names its own button", async ({
  page,
}) => {
  await openSettings(page, { "marginmail-dev-pending": "1" });
  await page.locator(".settings-tab", { hasText: "Backup" }).click();

  const section = page.locator(".set-section");
  const store = page.getByRole("tablist", { name: "Backup store" });
  const drive = store.getByRole("tab", { name: "Google Drive" });
  const off = store.getByRole("tab", { name: "Off" });
  await expect(drive).toHaveAttribute("aria-selected", "true");

  // The segment shows the new pick before the command answers, and nothing else can be picked
  // until it does. It used to sit on the old value for the whole of the key derivation.
  await off.click();
  await expect(off).toHaveAttribute("aria-selected", "true");
  await expect(store).toHaveAttribute("data-disabled", "");
  await expect(drive).toBeDisabled();
  await expect(section).toHaveAttribute("data-phase", "configuring");

  await expect(section).toHaveAttribute("data-phase", "idle");
  await expect(store).not.toHaveAttribute("data-disabled", "");
  await expect(off).toHaveAttribute("aria-selected", "true");

  // Two buttons, two waits, and only the one that was pressed goes grey.
  const backUp = page.getByRole("button", { name: /^Back(ing)? up/ });
  const restore = page.getByRole("button", { name: /^Restor(e|ing)$/ });
  await page
    .getByLabel("Recovery phrase")
    .fill("candle harbour ribbon meadow anchor lantern pepper thicket marble orchard signal walnut");
  await expect(restore).toBeEnabled();

  await backUp.click();
  await expect(section).toHaveAttribute("data-phase", "backing-up");
  await expect(backUp).toHaveText("Backing up");
  await expect(backUp).toBeDisabled();
  await expect(restore).toBeEnabled();
  await expect(section).toHaveAttribute("data-phase", "idle");
  await expect(backUp).toHaveText("Back up now");
  await expect(page.locator(".toast-text")).toContainText("Backed up");

  await restore.click();
  await expect(section).toHaveAttribute("data-phase", "restoring");
  await expect(restore).toHaveText("Restoring");
  await expect(restore).toBeDisabled();
  await expect(backUp).toBeEnabled();
  await expect(section).toHaveAttribute("data-phase", "idle");
  await expect(page.getByLabel("Recovery phrase")).toHaveValue("");
  await expect(page.locator(".toast-text")).toContainText("Restored from your backup");
});

test("clearing the mirror holds the confirmation until the command answers", async ({ page }) => {
  await openSettings(page, { "marginmail-dev-pending": "1" });
  await page.locator(".settings-tab", { hasText: "Data" }).click();

  await page.getByRole("button", { name: "Clear the mirror for pj@73ai.org" }).click();
  const asking = page.getByRole("dialog", { name: "Clear the mirror" });
  await asking.getByRole("button", { name: "Clear it" }).click();

  // The button says what it is doing, and nothing on the sheet closes it while it does: not
  // Cancel, not the close control, not Escape, not the scrim.
  const clearing = asking.getByRole("button", { name: "Clearing" });
  await expect(clearing).toBeVisible();
  await expect(clearing).toBeDisabled();
  await expect(asking).toHaveAttribute("data-busy", "");
  await expect(asking.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await expect(asking.getByRole("button", { name: "Close" })).toBeDisabled();
  await page.keyboard.press("Escape");
  // Down the left edge, where the scrim is: the title bar sits over the overlay along the top.
  await page.locator(".overlay").click({ position: { x: 20, y: 450 } });
  await expect(asking).toBeVisible();

  await expect(asking).toHaveCount(0);
  await expect(page.locator(".toast-text")).toContainText("is being fetched again");
});

test("a name that cannot be written goes back to what it was", async ({ page }) => {
  await failCommands(page, ["account_set_name"]);
  await openSettings(page);
  const first = page.locator(".acct").first();

  const name = first.getByLabel("Name for pj@73ai.org");
  const was = await name.inputValue();
  await name.fill("Work");
  await name.blur();

  await expect(page.locator(".toast-text")).toContainText("Could not change that name");
  await expect(name).toHaveValue(was);
  await expect(first.locator(".avatar")).not.toHaveText("W");
});

test("a box in the title bar leaves Settings rather than opening behind it", async ({ page }) => {
  // Clicking Inbox used to change the place underneath and leave Settings covering it, with no way
  // back except a key. The place commands always closed Settings; the title bar did not use them.
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await page.keyboard.press("Meta+,");
  await expect(page.locator(".settings")).toBeVisible();

  // Nothing is selected while Settings is up: saying Inbox is selected under a screen that is not
  // the Inbox is the same lie the trap was built on.
  await expect(page.locator('.titlebar [role="tab"][aria-selected="true"]')).toHaveCount(0);

  await page.locator('.titlebar [role="tab"]').filter({ hasText: "Feed" }).click();
  await expect(page.locator(".settings")).toHaveCount(0);
  await expect(page.locator(".app")).toHaveAttribute("data-place", "feed");
});
