// A mailbox that is not Google, reached with an address, then a name and a password.
//
// `openApp(page, { firstRun: true })` is an install with nothing connected, which is the only way
// to reach the connect screen at all. From there the fixture's directory of providers decides what
// each address does: one publishes its own settings, one is only in the public directory and wants
// an app password, one is worked out from the MX record, one presents a certificate nobody vouches
// for, one is Proton and points at a Bridge that is not running, and anything else is not published
// anywhere, which is the servers sheet.
//
// Nothing here reaches into the app. Every outcome is produced by typing a different address.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { listReady, openApp, settle } from "./app";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

/** The IMAP column of the sheet, and the SMTP one. They carry the same five field names. */
const incoming = (page: Page): Locator => page.locator(".imap-leg").first();
const outgoing = (page: Page): Locator => page.locator(".imap-leg").nth(1);

/** The first step: the address, submitted from the keyboard, and the sign-in panel it leads to. */
async function lookUp(page: Page, email: string): Promise<void> {
  await page.getByLabel("Email address").fill(email);
  await page.keyboard.press("Enter");
  await expect(page.getByLabel("Your name")).toBeVisible();
}

/** The second step: who you are and the password, wherever the flow is being drawn. */
async function signIn(page: Page, password = "correct horse"): Promise<void> {
  await page.getByLabel("Your name").fill("Priyanshu Jain");
  await page.getByLabel("Password").fill(password);
  await page.keyboard.press("Enter");
}

/**
 * The whole of the flow up to the servers' answer, filled the way a person fills it and submitted
 * from the keyboard, because Enter has to be the way out of forms this short.
 */
async function connectWith(page: Page, email: string, password = "correct horse"): Promise<void> {
  await lookUp(page, email);
  await signIn(page, password);
}

/**
 * The same flow in its other host: a sheet over Settings rather than the stage of a screen that is
 * not the app yet. Everything past the button is the one component, which is what the last two
 * tests are for.
 */
async function lookUpFromSettings(page: Page, email: string): Promise<void> {
  await page.keyboard.press("Meta+,");
  await expect(page.locator(".settings")).toBeVisible();
  await page.getByRole("button", { name: "Add account" }).click();
  await expect(page.getByRole("dialog", { name: "Add an account" })).toBeVisible();
  await lookUp(page, email);
  // The panel's own head carries what the stage draws as a heading, and there is only one of them.
  await expect(page.locator(".welcome-title")).toHaveCount(0);
}

test("the welcome screen asks for the address and nothing else", async ({ page }) => {
  await openApp(page, { firstRun: true });
  await expect(page.getByLabel("Email address")).toBeVisible();

  expect(await page.locator(".imap-address .field-label").allTextContents()).toEqual([
    "Email address",
  ]);
  // Nothing about a provider, and no second way in: the address is the way in.
  await expect(page.getByRole("button", { name: /Google/ })).toHaveCount(0);
  await expect(page.locator(".welcome-soon")).toHaveCount(0);
  await expect(page.getByLabel("Password")).toHaveCount(0);
});

test("the sign-in step is a name and a password, under where the servers came from", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await lookUp(page, "pj@fastmail.example");

  await expect(page.locator(".welcome-title")).toHaveText("Sign in to Fastmail");
  // Published by the provider is a different promise from guessed, and the sentence says which,
  // before the password is typed rather than after it was sent.
  await expect(page.locator(".welcome-line")).toContainText("Fastmail publishes its own settings");
  const facts = await page.locator(".imap-fact").allTextContents();
  expect(facts[0].replace(/\s+/g, " ")).toContain("imap.fastmail.example, port 993, TLS");
  expect(facts[1].replace(/\s+/g, " ")).toContain("smtp.fastmail.example, port 465, TLS");

  expect(await page.locator(".imap-form .field-label").allTextContents()).toEqual([
    "Your name",
    "Password",
  ]);
  // The password is a password, on a screen whose whole subject is where that password goes.
  await expect(page.locator(".imap-form input[type='password']")).toHaveCount(1);
  await expect(page.getByLabel("Your name")).toBeFocused();
  await expect(page.getByRole("button", { name: "Add account" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Change the servers" })).toBeVisible();

  await page.getByLabel("Your name").fill("Priyanshu Jain");
  mkdirSync(shots, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "connect-mail.png") });

  // Escape is the way back to the address, which is still there to be corrected.
  await page.keyboard.press("Escape");
  await expect(page.locator(".welcome-title")).toHaveText("Margin Mail");
  await expect(page.getByLabel("Email address")).toHaveValue("pj@fastmail.example");
});

test("a provider that wants an app password is told so before the password is typed", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await lookUp(page, "pj@fastmail.example");

  // The single most common way an IMAP setup fails, said under the field it fails in.
  await expect(page.locator(".imap-form .field-hint").nth(1)).toContainText("app password");
  await expect(page.locator(".imap-form .field-hint").nth(1)).toContainText("Privacy & Security");
});

test("discovery that finds nothing says so, and the servers come from the sign-in step", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await lookUp(page, "pj@harborlife.example");

  // Not a failure, and the panel does not read as one: it is the same panel with a different
  // first sentence, no server lines, and a button that opens them.
  await expect(page.locator(".welcome-title")).toHaveText("Sign in to harborlife.example");
  await expect(page.locator(".welcome-line")).toContainText(
    "Nobody publishes settings for harborlife.example",
  );
  await expect(page.locator(".imap-fact")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Enter the servers" })).toBeVisible();

  await signIn(page);
  await expect(page.getByRole("dialog", { name: "Servers" })).toBeVisible();
  await expect(page.locator(".banner")).toContainText(
    "Nobody publishes settings for harborlife.example",
  );

  await expect(incoming(page).getByLabel("Server")).toHaveValue("imap.harborlife.example");
  await expect(outgoing(page).getByLabel("Server")).toHaveValue("smtp.harborlife.example");
  // The address is carried across, and so is the password: nothing is typed twice.
  await expect(incoming(page).getByLabel("Username")).toHaveValue("pj@harborlife.example");
  await expect(incoming(page).getByLabel("Password")).toHaveValue("correct horse");
  // Which is exactly what the outgoing half does not insist on.
  await expect(outgoing(page).getByLabel("Username")).toHaveValue("");
  await expect(outgoing(page).getByLabel("Password")).toHaveValue("");

  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "connect-mail-servers.png") });

  // Escape goes back a step rather than throwing the whole flow away.
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Servers" })).toHaveCount(0);
  await expect(page.locator(".welcome-title")).toHaveText("Sign in to harborlife.example");
  await expect(page.getByLabel("Your name")).toHaveValue("Priyanshu Jain");
});

test("the servers can be typed without waiting for a lookup", async ({ page }) => {
  await openApp(page, { firstRun: true });
  await page.getByLabel("Email address").fill("pj@selfhosted.example");
  await page.getByRole("button", { name: "Enter the servers myself" }).click();

  // Straight to the sign-in step, with a sentence that does not pretend a lookup was made.
  await expect(page.locator(".welcome-title")).toHaveText("Sign in to selfhosted.example");
  await expect(page.locator(".welcome-line")).toContainText("The servers are yours to type");
  await expect(page.getByRole("button", { name: "Enter the servers" })).toBeVisible();

  await signIn(page);
  await expect(page.getByRole("dialog", { name: "Servers" })).toBeVisible();
  await expect(incoming(page).getByLabel("Server")).toHaveValue("imap.selfhosted.example");
  // Nothing was looked up, so nothing is said about what the lookup found.
  await expect(page.locator(".banner")).toHaveCount(0);
});

test("changing the security corrects the port, on both halves", async ({ page }) => {
  await openApp(page, { firstRun: true });
  await connectWith(page, "pj@harborlife.example");
  await expect(page.getByRole("dialog", { name: "Servers" })).toBeVisible();

  await expect(incoming(page).getByLabel("Port")).toHaveValue("993");
  await incoming(page).getByRole("tab", { name: "STARTTLS" }).click();
  await expect(incoming(page).getByLabel("Port")).toHaveValue("143");
  await incoming(page).getByRole("tab", { name: "TLS", exact: true }).click();
  await expect(incoming(page).getByLabel("Port")).toHaveValue("993");

  // The two halves agree on the shape and disagree on the numbers.
  await expect(outgoing(page).getByLabel("Port")).toHaveValue("465");
  await outgoing(page).getByRole("tab", { name: "STARTTLS" }).click();
  await expect(outgoing(page).getByLabel("Port")).toHaveValue("587");
  await outgoing(page).getByRole("tab", { name: "None" }).click();
  await expect(outgoing(page).getByLabel("Port")).toHaveValue("25");
  // And an unencrypted connection says what that means rather than letting it pass.
  await expect(outgoing(page).locator(".field-hint[data-tone='error']")).toContainText(
    "Nothing on this connection is encrypted",
  );
});

test("a refused password says so on the panel it was typed on, and what to do about it", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await connectWith(page, "pj@oakridge-school.example");

  // A mistyped password is corrected where it was typed, not in a two-column sheet of servers
  // that were never the problem.
  await expect(page.locator(".welcome-trouble")).toContainText(
    "Signing in to imap.oakridge-school.example did not work",
  );
  await expect(page.getByRole("dialog", { name: "Servers" })).toHaveCount(0);
  // The server's own words, because they name the thing to go and fix.
  await expect(page.locator(".imap-said")).toContainText("Application-specific password required");
  await expect(page.locator(".imap-advice")).toHaveText(
    "This account wants an app password rather than the one you sign in with.",
  );
  // And the way to change the servers is still there for whoever needs it.
  await expect(page.getByRole("button", { name: "Change the servers" })).toBeEnabled();
});

test("a wrong password on the outgoing half says it was the outgoing half", async ({ page }) => {
  await openApp(page, { firstRun: true });
  await connectWith(page, "pj@harborlife.example");
  await expect(page.getByRole("dialog", { name: "Servers" })).toBeVisible();

  await incoming(page).getByLabel("Server").fill("mail.harborlife.example");
  await outgoing(page).getByLabel("Server").fill("mail.harborlife.example");
  await outgoing(page).getByLabel("Password").fill("wrong");
  await page.getByRole("button", { name: "Test and add" }).click();

  await expect(page.locator(".banner")).toContainText(
    "Your mail came through. Sending, through mail.harborlife.example, did not.",
  );
  await expect(page.locator(".imap-said")).toContainText("Invalid credentials");
});

test("a certificate nobody vouches for is a decision, with the fingerprint to check it by", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await lookUp(page, "pj@sunnydaymusic.example");

  // The rung of the ladder that answered for this one was a guess, and the panel says so before
  // anybody types a password into it.
  await expect(page.locator(".welcome-line")).toContainText(
    "Nobody publishes settings for sunnydaymusic.example",
  );
  await expect(page.locator(".welcome-line")).toContainText("a guess");
  await signIn(page);

  await expect(page.getByRole("dialog", { name: "The server's certificate" })).toBeVisible();
  await expect(page.locator(".imap-lead")).toContainText(
    "mail.sunnydaymusic.example presented a certificate",
  );
  await expect(page.locator(".imap-lead")).toContainText("it signed its own certificate");

  // Scoped to the sheet: the sign-in panel under it still carries its two server lines.
  const sheet = page.getByRole("dialog", { name: "The server's certificate" });
  const facts = await sheet.locator(".imap-fact dt").allTextContents();
  expect(facts).toEqual(["Fingerprint", "Subject", "Issuer", "Expires"]);
  await expect(sheet.locator(".imap-fact dd").first()).toContainText("9F:2C:41:8E");
  // Never asked about the loopback, so a person seeing this is being asked about the network.
  await expect(page.locator(".imap-note")).toContainText("out on the network");

  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "connect-mail-certificate.png") });

  // Going back is the other answer, and it goes back to the panel whose test raised the question.
  await page.getByRole("button", { name: "Go back" }).click();
  await expect(page.getByRole("dialog", { name: "Servers" })).toHaveCount(0);
  await expect(page.locator(".welcome-title")).toHaveText("Sign in to sunnydaymusic.example");
});

test("trusting a certificate is remembered, and the connection carries on", async ({ page }) => {
  await openApp(page, { firstRun: true });
  await connectWith(page, "pj@sunnydaymusic.example");

  // One question per socket, because that is what the trust store holds: a fingerprint accepted on
  // a host and a port. This server answers on both ports, so it is asked about twice, and the
  // second panel names the outgoing one.
  await page.getByRole("button", { name: "Trust this certificate" }).click();
  await expect(page.locator(".imap-note")).toContainText("port 465");
  await page.getByRole("button", { name: "Trust this certificate" }).click();

  // Both accepted, the servers answered, and there is nothing left to confirm.
  await expect(page.locator(".welcome")).toHaveCount(0);
  await expect(page.locator(".row").first()).toBeVisible();
});

test("a bridge that is not running says so, rather than printing a socket error", async ({
  page,
}) => {
  await openApp(page, { firstRun: true });
  await lookUp(page, "pj@proton.me");

  // Proton's own published settings, which point at Bridge on this machine, and the hint that says
  // which password Bridge wants before it is asked for.
  await expect(page.locator(".welcome-title")).toHaveText("Sign in to Proton Mail Bridge");
  await expect(page.locator(".imap-fact").first()).toContainText("127.0.0.1, port 1143");
  await expect(page.locator(".imap-form .field-hint").nth(1)).toContainText("Mailbox details");

  await signIn(page);
  await expect(page.locator(".imap-advice")).toContainText("Proton Bridge is not running");
  // The socket error is still there for whoever wants it. It is just not the headline.
  await expect(page.locator(".imap-said")).toContainText("Connection refused");
});

test("a bridge that is running connects like anything else", async ({ page }) => {
  await openApp(page, { firstRun: true, storage: { "marginmail-dev-bridge": "1" } });
  await lookUp(page, "pj@proton.me");

  // A port that was typed rather than defaulted survives the security changing under it.
  await page.getByRole("button", { name: "Change the servers" }).click();
  await incoming(page).getByRole("tab", { name: "TLS", exact: true }).click();
  await expect(incoming(page).getByLabel("Port")).toHaveValue("1143");
  await page.keyboard.press("Escape");

  await signIn(page);
  await expect(page.locator(".welcome")).toHaveCount(0);
  await expect(page.locator(".row").first()).toBeVisible();
});

test("the account arrives and the welcome screen is over", async ({ page }) => {
  await openApp(page, { firstRun: true });
  await connectWith(page, "pj@fastmail.example");

  await expect(page.locator(".welcome")).toHaveCount(0);
  await expect(page.locator(".row").first()).toBeVisible();
});

test("servers typed by hand connect the same account the same way", async ({ page }) => {
  await openApp(page, { firstRun: true });
  await connectWith(page, "pj@harborlife.example");
  await expect(page.getByRole("dialog", { name: "Servers" })).toBeVisible();

  await incoming(page).getByLabel("Server").fill("mail.harborlife.example");
  await outgoing(page).getByLabel("Server").fill("mail.harborlife.example");
  // Enter, from inside the sheet, because the whole flow has to be finishable without a mouse.
  await outgoing(page).getByLabel("Server").press("Enter");

  await expect(page.locator(".welcome")).toHaveCount(0);
  await expect(page.locator(".row").first()).toBeVisible();
});

test("Settings runs the same flow, and the account it connects is in the list", async ({ page }) => {
  await openApp(page);
  await listReady(page);
  await lookUpFromSettings(page, "pj@fastmail.example");

  // The same panel as the welcome screen's, with the same sentence about where the settings came
  // from, in a sheet whose head is doing the heading's job.
  await expect(page.getByRole("dialog", { name: "Sign in to Fastmail" })).toBeVisible();
  await expect(page.locator(".welcome-line")).toContainText("Fastmail publishes its own settings");

  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "settings-connect-mail.png") });

  await signIn(page);

  // The account is written and its first sync takes the window, the way a Google account's does
  // from Settings: the sign-in sheet is gone, the arriving panel is up, and when the pass ends
  // what is behind it is the new account's Inbox with the first-run panel over it.
  await expect(page.getByRole("dialog", { name: "Sign in to Fastmail" })).toHaveCount(0);
  const panel = page.getByRole("dialog", { name: "Bringing in pj@fastmail.example" });
  await expect(panel).toBeVisible();
  await panel.getByRole("button", { name: "Start" }).click();
  await expect(panel).toHaveCount(0);
  await expect(page.locator(".settings")).toHaveCount(0);
  await expect(page.locator(".account-chip")).toContainText("pj@fastmail.example");
  const setUp = page.getByRole("dialog", { name: "You are set up" });
  await expect(setUp).toBeVisible();
  await setUp.getByRole("button", { name: "Done" }).click();

  // The tour follows that panel for every account added, a password account included. Escape is
  // how somebody on their third mailbox refuses it, and the keys belong to the window again after.
  await expect(page.getByRole("dialog", { name: "Getting started" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".overlay")).toHaveCount(0);

  // And the account is on the list without anything being reloaded.
  await page.keyboard.press("Meta+,");
  await expect(page.locator(".acct")).toHaveCount(3);
  await expect(page.locator(".acct").nth(2).locator(".acct-addr")).toHaveText("pj@fastmail.example");
  // And its card is the IMAP card: the servers it was just added with, and nothing to grant.
  await expect(page.locator(".acct").nth(2).locator(".acct-servers")).toContainText(
    "imap.fastmail.example, port 993, TLS",
  );
  await expect(page.locator(".acct").nth(2).getByRole("button", { name: "Grant" })).toHaveCount(0);
});

test("a refusal in Settings is said in the sheet, and the servers open over it", async ({
  page,
}) => {
  await openApp(page);
  await listReady(page);
  await lookUpFromSettings(page, "pj@oakridge-school.example");
  await signIn(page);

  // A flow no wire protocol has ever run for real fails in Settings the way it fails on the welcome
  // screen: out loud, with what the server said, in the panel the password was typed into.
  const sheet = page.getByRole("dialog", { name: "Sign in to Oakridge School Mail" });
  await expect(sheet).toBeVisible();
  await expect(sheet.locator(".welcome-trouble")).toContainText(
    "Signing in to imap.oakridge-school.example did not work",
  );
  await expect(sheet.locator(".imap-advice")).toHaveText(
    "This account wants an app password rather than the one you sign in with.",
  );

  // The servers take the window rather than a place inside the section, so the panel that opened
  // them gives it up: one dialog, not a sheet stacked on a sheet.
  await page.getByRole("button", { name: "Change the servers" }).click();
  await expect(page.getByRole("dialog", { name: "Servers" })).toBeVisible();
  await expect(page.locator('[role="dialog"]')).toHaveCount(1);

  // Escape goes back to the flow that opened it, still inside Settings, with the account list
  // behind it untouched.
  await page.keyboard.press("Escape");
  await expect(sheet).toBeVisible();
  await expect(page.locator(".acct")).toHaveCount(2);
});
