// The first screen, and the states it moves through.
//
// `openApp(page, { firstRun: true })` is an install with nothing connected: the fixture answers
// every read as empty until a consent completes, at which point it fills in and narrates the first
// sync exactly as the backend does. So the whole flow, including the two states nobody can produce
// on demand against the real Google, is driven here by typing an address.
//
// The screen asks for one thing, the address, and works the rest out. A Google address goes to the
// browser, a Microsoft one is told why it cannot, and everything else is tests/imap.spec.ts. A
// cancelled consent and a withheld scope are dispatched as `auth` events, which is what the backend
// sends and what `src/dev/mockIpc.ts` sends. Nothing here reaches into the app.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { openApp, settle } from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

/** Quoted from docs/ui.md. This sentence is the promise the sign-in is asking to be trusted on. */
const PRIVACY =
  "Sign-in happens in your browser with Google. Margin never sees your password. The key Google " +
  "hands back is stored only on this device, and you can revoke it any time from your Google account.";

const REQUIRED = "https://www.googleapis.com/auth/gmail.modify";

/** An answer from the consent page, in the shape the backend emits. */
function auth(page: Page, event: Record<string, unknown>): Promise<void> {
  return page.evaluate((detail) => {
    window.dispatchEvent(new CustomEvent("auth", { detail }));
  }, {
    ok: false,
    error: null,
    accountId: null,
    email: null,
    cancelled: false,
    grantedScopes: [],
    missingRequired: [],
    ...event,
  });
}

const address = (page: Page) => page.getByLabel("Email address");

/** The whole of the first step: an address and Enter, which is how a person does it. */
async function go(page: Page, email: string): Promise<void> {
  await address(page).fill(email);
  await page.keyboard.press("Enter");
}

/** A Google address, which is the way in that goes to the browser. */
const connect = (page: Page) => go(page, "pj@gmail.com");

test("the welcome screen is the wordmark, one field, and what works", async ({ page }) => {
  await openApp(page, { firstRun: true });

  await expect(page.locator(".welcome-title")).toHaveText("Margin Mail");
  await expect(address(page)).toBeVisible();
  await expect(address(page)).toBeFocused();
  await expect(page.getByRole("button", { name: "Continue" })).toBeVisible();

  // No provider is named as a button. The one that used to be here was Google's, and a screen that
  // opens with a Google button and an "anything else" button under it has decided who it is for.
  await expect(page.getByRole("button", { name: /Google/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /other account/ })).toHaveCount(0);
  await expect(page.locator(".welcome-privacy")).toContainText("Any mailbox works here");

  // Nothing of the app is behind it: no header, no list, no account chip with nobody in it.
  await expect(page.locator(".account-chip")).toHaveCount(0);
  await expect(page.locator(".list-col")).toHaveCount(0);

  mkdirSync(shots, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "connect.png") });
});

test("something that is not an address is said so, and nothing is looked up", async ({ page }) => {
  await openApp(page, { firstRun: true });
  await go(page, "pj");

  await expect(page.locator(".welcome-trouble")).toContainText("An email address is what this needs");
  await expect(page.locator(".welcome-title")).toHaveText("Margin Mail");
  // Typing again clears the complaint, so it does not sit under a field that has since been fixed.
  await address(page).fill("pj@");
  await expect(page.locator(".welcome-trouble")).toHaveCount(0);
});

test("a Google address goes to the browser, and waiting offers both ways back to the link", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await connect(page);

  await expect(page.locator(".welcome-title")).toHaveText("Waiting for Google");
  // The browser that opened is not always the browser in front of you.
  await expect(page.getByRole("button", { name: "Open link again" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "Copy link" })).toBeEnabled();
  // And the promise the sign-in is asking to be trusted on is said on the screen that waits for it.
  await expect(page.locator(".welcome-privacy")).toHaveText(PRIVACY);
});

test("a work domain whose mail is delivered to Google goes the same way, after a lookup", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  // Nothing about the address says Google. The directory does, from the domain's MX record.
  await go(page, "pj@northgate.example");

  await expect(page.locator(".welcome-title")).toHaveText("Waiting for Google");
});

test("closing the consent page goes quietly back to the address, which is still there", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await connect(page);
  await expect(page.locator(".welcome-title")).toHaveText("Waiting for Google");

  await auth(page, { cancelled: true });

  await expect(page.locator(".welcome-title")).toHaveText("Margin Mail");
  await expect(address(page)).toHaveValue("pj@gmail.com");
  // Changing your mind is an answer. Nothing on the screen says anything went wrong.
  await expect(page.locator(".welcome-trouble")).toHaveCount(0);
  await expect(page.locator(".toast")).toHaveCount(0);
});

test("withholding the one required permission says which, and offers to try again", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await connect(page);
  await expect(page.locator(".welcome-title")).toHaveText("Waiting for Google");

  await auth(page, { missingRequired: [REQUIRED] });

  await expect(page.locator(".welcome-title")).toHaveText("One permission is missing");
  // Named in plain English rather than as a scope URL.
  await expect(page.locator(".welcome-line")).toContainText("Read and change your mail");
  await expect(page.locator(".welcome-line")).not.toContainText("googleapis.com");
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
});

test("an Outlook address is told why before anything is typed, and can try another", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await go(page, "pj@hotmail.com");

  await expect(page.locator(".welcome-title")).toHaveText("Outlook is not here yet");
  await expect(page.locator(".welcome-line")).toContainText("app registration with Microsoft");
  // No password was asked for, because none would have worked.
  await expect(page.getByLabel("Password")).toHaveCount(0);

  await page.getByRole("button", { name: "Try another address" }).click();
  await expect(page.locator(".welcome-title")).toHaveText("Margin Mail");
  await expect(address(page)).toHaveValue("pj@hotmail.com");
});

test("a mailbox with no IMAP behind it says so, by name", async ({ page }) => {
  await openApp(page, { firstRun: true });
  await go(page, "pj@hey.com");

  await expect(page.locator(".welcome-title")).toHaveText("HEY has no way in");
  await expect(page.locator(".welcome-line")).toContainText("no IMAP, no POP");
  // Escape is the other way back, and it is the same way back.
  await page.keyboard.press("Escape");
  await expect(page.locator(".welcome-title")).toHaveText("Margin Mail");
});

test("the first sync is a bar and a count, and becomes the Inbox when the mail lands", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await connect(page);

  // Consent is back and the account is written. Before anything is fetched, the one question:
  // how far back this device holds. A month is the default and one press starts the sync.
  await expect(page.locator(".welcome-title")).toHaveText("How far back?");
  await expect(page.locator(".window-choice-option[data-on]")).toHaveText("30 days");
  await expect(page.locator(".welcome-bar")).toHaveCount(0);
  await page.getByRole("button", { name: "Start" }).click();

  await expect(page.locator(".welcome-title")).toHaveText("Bringing in the last month");
  await expect(page.locator(".welcome-bar")).toBeVisible();
  await expect(page.locator(".welcome-count")).toContainText("of 4,812 messages");

  // The bar is a real fraction of a real denominator rather than a spinner, so it is somewhere
  // between nothing and everything while the mail is arriving.
  await expect
    .poll(async () => {
      const filled = await page
        .locator(".welcome-bar-fill")
        .evaluate((el) => new DOMMatrixReadOnly(getComputedStyle(el).transform).a);
      return filled > 0 && filled < 1;
    })
    .toBe(true);

  // It hands over when the first page of threads arrives, not when the sync finishes.
  await expect(page.locator(".row").first()).toBeVisible();
  await expect(page.locator(".welcome")).toHaveCount(0);

  // And the first-run panel arrives over that Inbox, once, with what the pass over senders did.
  await expect(page.getByRole("dialog", { name: "You are set up" })).toBeVisible();
  await expect(page.locator(".onboard-lead")).toContainText("senders");
  await expect(page.locator(".choice-option[data-on]")).toHaveText("a week");

  await page.getByRole("button", { name: "Done" }).click();
  await expect(page.getByRole("dialog", { name: "You are set up" })).toHaveCount(0);

  // Once per account, and the fact is this device's.
  const remembered = await page.evaluate(() => localStorage.getItem("marginmail-onboarded"));
  expect(remembered).toContain("acct-1");
});

test("a first sync that fails says why instead of counting for ever", async ({ page }) => {
  // The fixture emits the working status and the failure inside one timer, so React renders them
  // as one update. A screen that waited to watch the phase change from working to stopped never
  // saw the working half, and sat on "Listing your mail" with the reason two layers down in a
  // status nothing rendered. That is the whole bug, and this is the shape of it.
  await openApp(page, { firstRun: true, storage: { "marginmail-dev-sync-fails": "1" } });
  await connect(page);
  await page.getByRole("button", { name: "Start" }).click();

  await expect(page.locator(".welcome-title")).toHaveText("Your mail did not arrive");
  await expect(page.locator(".welcome-bar")).toHaveCount(0);

  // Google's own sentence, verbatim, because it names the thing to go and fix.
  await expect(page.locator(".welcome-trouble")).toContainText("Gmail API has not been used");

  // And two ways on, neither of which is closing the app.
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
  await page.getByRole("button", { name: "Go in anyway" }).click();
  await expect(page.locator(".welcome")).toHaveCount(0);
});
