// The verbs, driven the way a person drives them: focus a row, press the key, and look at what the
// list and the toast now say.
//
// Nothing here waits for a call to finish before it looks. Every one of these actions is optimistic
// and the row is expected to have moved before the fixture answers, which is the whole point of
// doing it that way, so an assertion that passed only after a round trip would be missing the bug.

import { expect, test, type Page } from "@playwright/test";
import { failCommands, listReady, MIDDAY, openApp, paletteRows, rows, toast } from "./app";

/** Focuses the row a spec means to act on, by walking to it rather than by clicking it open. */
async function focusRow(page: Page, sender: string): Promise<void> {
  for (let i = 0; i < 20; i++) {
    const at = (await rows(page)).find((row) => row.selected);
    if (at?.sender === sender) return;
    await page.keyboard.press("j");
  }
  throw new Error(`never reached ${sender}`);
}

/**
 * A fixture whose label list fails the first time it is read, which is the read the list makes on
 * boot, and takes a while the next time, which is the read the picker makes when it opens. The
 * same trick as `failCommands`: the module is answered with a shim in front of the real one.
 */
async function slowLabels(page: Page): Promise<void> {
  await page.route("**/src/dev/mockIpc.ts*", async (route) => {
    if (new URL(route.request().url()).searchParams.has("real")) return route.fallback();
    await route.fulfill({
      contentType: "text/javascript",
      body: [
        `import { mockCall as real } from "/src/dev/mockIpc.ts?real";`,
        `let reads = 0;`,
        `export function mockCall(command, args) {`,
        `  if (command !== "labels_list") return real(command, args);`,
        `  if (reads++ === 0) return Promise.reject(new Error("the mailbox is offline"));`,
        `  return new Promise((resolve) => setTimeout(resolve, 700)).then(() => real(command, args));`,
        `}`,
      ].join("\n"),
    });
  });
}

test("e archives the focused thread, says so, and z brings it back", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const before = (await rows(page)).length;
  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("e");

  // The row is gone before the fixture has answered: the call goes behind the change, not in
  // front of it.
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toBe("Archived");
  expect((await toast(page))?.action).toContain("Undo");

  await page.keyboard.press("z");
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(1);
  await expect.poll(async () => (await rows(page)).length).toBe(before);
});

test("the toast's Undo goes down on the press and says what it undid", async ({ page }) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("e");
  await expect.poll(async () => (await toast(page))?.text).toBe("Archived");

  await page.locator(".toast-action").click();
  // Down before the undo has landed, so there is no second press to be refused.
  await expect(page.locator(".toast")).toHaveCount(0);
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(1, {
    timeout: 5_000,
  });
  // And then said, the way `z` says it, with nothing left on it to press.
  await expect.poll(async () => (await toast(page))?.text).toBe("Undone: Archived");
  expect((await toast(page))?.action).toBeNull();
});

test("# trashes and ! marks spam, both with a way back", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("#");
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toBe("Trashed");

  await page.keyboard.press("z");
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(1);

  await focusRow(page, "Arun Kulkarni");
  await page.keyboard.press("!");
  await expect(page.locator(".row", { hasText: "Arun Kulkarni" })).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toBe("Marked as spam");

  await page.keyboard.press("z");
  await expect(page.locator(".row", { hasText: "Arun Kulkarni" })).toHaveCount(1);
});

/** The three places under Other have no key, so the palette is how a test reaches them too. */
async function goToPlace(page: Page, title: string): Promise<void> {
  await page.keyboard.press("Meta+k");
  await page.locator(".palette-row", { hasText: title }).first().click();
  await expect(page.locator(".list-title")).toHaveText(title);
}

test("# in Trash puts a thread back where it was, and ! in Spam takes the mark off", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const parcel = page.locator(".row", { hasText: "Your parcel could not be delivered" });
  await goToPlace(page, "Trash");
  await expect(parcel).toHaveCount(1);

  await focusRow(page, "Parcel Notice");
  await page.keyboard.press("#");
  await expect(parcel).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toBe("Restored");

  // Back where it was, which is the Paper Trail its sender is routed to and not the Inbox.
  await page.keyboard.press("1");
  await expect(page.locator(".list-title")).toHaveText("Inbox");
  await expect(parcel).toHaveCount(0);
  await page.keyboard.press("3");
  await expect(page.locator(".list-title")).toHaveText("Paper Trail");
  await expect(parcel).toHaveCount(1);

  const phish = page.locator(".row", { hasText: "Account verification required" });
  await goToPlace(page, "Spam");
  await expect(phish).toHaveCount(1);

  await focusRow(page, "Aćcount Security");
  await page.keyboard.press("!");
  await expect(phish).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toBe("Marked as not spam");
});

test("u toggles the dot and Shift+S toggles the star", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const seen = () => rows(page).then((all) => all.find((row) => row.sender === "Lena Brandt")?.unseen);

  await focusRow(page, "Lena Brandt");
  expect(await seen()).toBe(false);
  await page.keyboard.press("u");
  await expect.poll(seen).toBe(true);
  await page.keyboard.press("u");
  await expect.poll(seen).toBe(false);

  // Marking one seen or unseen shows itself in the row, so it does not also announce itself.
  expect(await toast(page)).toBeNull();

  const star = page.locator(".row", { hasText: "Lena Brandt" }).locator(".row-mark");
  await expect(star).toHaveCount(0);
  await page.keyboard.press("Shift+S");
  await expect(star).toHaveCount(1);
  await page.keyboard.press("Shift+S");
  await expect(star).toHaveCount(0);
});

test("a call that does not go through puts the row back and says so", async ({ page }) => {
  await failCommands(page, ["flags_set"]);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const before = (await rows(page)).length;
  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("e");

  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(1);
  await expect.poll(async () => (await toast(page))?.text).toContain("did not go through");
  expect((await rows(page)).length).toBe(before);
  // The row goes back where it was rather than to the end of the list.
  expect((await rows(page))[1].sender).toBe("Maya Raghunathan");
});

test("Mark all as seen is a link on the heading and a row in the palette", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await expect.poll(async () => (await rows(page)).some((row) => row.unseen)).toBe(true);
  await page.locator(".group-head-action", { hasText: "Mark all as seen" }).click();
  await expect.poll(async () => (await rows(page)).some((row) => row.unseen)).toBe(false);
  await expect.poll(async () => (await toast(page))?.text).toBe("Marked everything as seen");

  await page.keyboard.press("z");
  await expect.poll(async () => (await rows(page)).some((row) => row.unseen)).toBe(true);

  await page.keyboard.press("Meta+k");
  const palette = await paletteRows(page);
  expect(palette.find((row) => row.label === "Mark all as seen")?.group).toBe("Actions");
});

test("the palette lists the provider's labels, and Shift+L puts one on a selection", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("Meta+k");
  const palette = await paletteRows(page);
  const labels = palette.filter((row) => row.group === "Labels").map((row) => row.label);
  expect(labels).toEqual(["The flat", "School", "Travel"]);
  // The provider's system labels are not among them. Every one is a place this app already has
  // under a name of its own, and a palette that answered "spam" with a Labels row and an Other row
  // is one that makes you choose between two spellings of the same place.
  expect(labels).not.toContain("SPAM");
  expect(labels).not.toContain("IMPORTANT");
  // They are the provider's places, and a place has no key of its own.
  expect(palette.find((row) => row.label === "Travel")?.keys).toEqual([]);
  await page.keyboard.press("Escape");

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("x");
  await page.keyboard.press("Shift+L");
  await expect(page.locator('[role="dialog"]')).toHaveAttribute("aria-label", "Label");

  await page.locator(".label-option", { hasText: "Travel" }).click();
  await expect.poll(async () => (await toast(page))?.text).toBe("Labelled Travel");

  // A label never shows on a row: the only way to see it is to go to the place it is.
  await expect(page.locator(".row-note", { hasText: "Travel" })).toHaveCount(0);
  await page.keyboard.press("Meta+k");
  await page.locator(".palette-row", { hasText: "Travel" }).click();
  await expect(page.locator(".list-title")).toHaveText("Travel");
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(1);
});

test("the label picker waits for the labels rather than calling an empty list none", async ({
  page,
}) => {
  await slowLabels(page);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("x");
  await page.keyboard.press("Shift+L");
  await expect(page.locator('[role="dialog"]')).toHaveAttribute("aria-label", "Label");

  // The boot read failed and the picker's own read is out, so the list is empty and that is not
  // the same thing as there being no labels.
  await expect(page.locator('.label-none[data-state="loading"]')).toHaveText("Reading your labels");
  await expect(page.locator(".label-none", { hasText: "No labels" })).toHaveCount(0);

  // Three, which is what this account has of its own. The provider's system labels are not
  // offered: putting `SPAM` on a thread is not a label you would ever have meant to pick.
  await expect(page.locator(".label-option")).toHaveCount(3, { timeout: 5_000 });
  await expect(page.locator(".label-none")).toHaveCount(0);
});

test("the label picker says when the labels could not be read, and offers to try again", async ({
  page,
}) => {
  await failCommands(page, ["labels_list"]);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("x");
  await page.keyboard.press("Shift+L");

  const failed = page.locator('.label-none[data-state="error"]');
  await expect(failed).toContainText("Could not read your labels");
  await expect(failed.locator(".button")).toHaveText("Try again");
  await expect(page.locator(".label-none", { hasText: "No labels" })).toHaveCount(0);
});

test("v moves to a label in a label list, and means nothing anywhere else", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  // In the Inbox `v` is the routing verb, which belongs to the next milestone: nothing owns it, so
  // it does nothing and shows nothing.
  await page.keyboard.press("j");
  await page.keyboard.press("v");
  await expect(page.locator('[role="dialog"]')).toHaveCount(0);

  await page.keyboard.press("Meta+k");
  await page.locator(".palette-row", { hasText: "The flat" }).click();
  await expect(page.locator(".list-title")).toHaveText("The flat");
  await expect(page.locator(".row", { hasText: "Arun Kulkarni" })).toHaveCount(1);

  await page.keyboard.press("j");
  await page.keyboard.press("v");
  await expect(page.locator('[role="dialog"]')).toHaveAttribute("aria-label", "Move to a label");
  await page.locator(".label-option", { hasText: "School" }).click();
  await expect.poll(async () => (await toast(page))?.text).toBe("Moved to School");
});
