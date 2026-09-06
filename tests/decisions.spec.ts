// The things HEY owns, kept beside the mail: a note, a name of your own, two threads made one, and
// the two libraries that read the mailbox sideways.
//
// None of these is a provider change and none of them shows anywhere in Gmail. What they have in
// common is that they are yours, so every assertion here is about the decision being visible where
// it was made and still there when you come back to it.

import { expect, test, type Page } from "@playwright/test";
import { listReady, MIDDAY, openApp, rows, toast } from "./app";

const openPlace = async (page: Page, label: string) => {
  await page.keyboard.press("Meta+k");
  await page.locator(".palette-row", { hasText: label }).first().click();
};

test("y writes a note, and it is in the pane and under the row", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.locator(".row", { hasText: "Dinner on Thursday?" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();

  await page.keyboard.press("y");
  await expect(page.locator('[role="dialog"]')).toHaveAttribute("aria-label", "Note to self");
  await page.locator(".field-textarea").fill("Book the table before Priya asks again");
  await page.locator(".panel-foot .button", { hasText: "Save" }).click();

  const note = page.locator(".thread-inner .note");
  await expect(note).toContainText("Note to self");
  await expect(note).toContainText("Book the table before Priya asks again");

  // And one line under the row, on the note surface, which is the only thing a list says about it.
  await expect
    .poll(async () => (await rows(page)).find((row) => row.sender === "Maya Raghunathan")?.note)
    .toBe("Book the table before Priya asks again");

  // It came from the backend rather than from the component that wrote it, which is what going
  // away and coming back proves. The dev fixture lives in the page and cannot outlive a reload of
  // it, so this is as far as the browser build can be asked.
  await page.keyboard.press("3");
  await expect(page.locator(".list-title")).toHaveText("Paper Trail");
  await page.keyboard.press("1");
  await page.locator(".row", { hasText: "Dinner on Thursday?" }).click();
  await expect(page.locator(".thread-inner .note")).toContainText(
    "Book the table before Priya asks again",
  );
});

test("a rename shows in the list and in the pane, with what it was", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.locator(".row", { hasText: "Lease renewal for the studio" }).click();
  await expect(page.locator(".thread-subject")).toBeVisible();

  // The subject is the control: the thing you want to change is the thing you press.
  await page.locator(".thread-name").click();
  await expect(page.locator('[role="dialog"]')).toHaveAttribute("aria-label", "Rename this thread");
  await page.locator(".field-input").fill("Studio lease, clause 7");
  await page.locator(".panel-foot .button", { hasText: "Rename" }).click();

  await expect(page.locator(".thread-name")).toHaveText("Studio lease, clause 7");
  await expect(page.locator(".thread-subject .renamed")).toHaveText(
    'renamed · was "Lease renewal for the studio"',
  );
  await expect
    .poll(async () => (await rows(page)).find((row) => row.sender === "Arun Kulkarni")?.subject)
    .toBe("Studio lease, clause 7");

  // And the real subject is one press away again, because a name of your own is not a deletion.
  await page.locator(".thread-name").click();
  await page.locator(".panel-foot .button", { hasText: "Use the real subject" }).click();
  await expect(page.locator(".thread-name")).toHaveText("Lease renewal for the studio");
  await expect(page.locator(".thread-subject .renamed")).toHaveCount(0);
});

test("g merges a selection into one row with a banner offering Unmerge", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  const before = (await rows(page)).length;

  await page.keyboard.press("j");
  await page.keyboard.press("j");
  await page.keyboard.press("x");
  await page.keyboard.press("Shift+J");
  await page.keyboard.press("g");

  // Two rows became one, and the one that is left is the first of them.
  await expect(page.locator(".row", { hasText: "Arun Kulkarni" })).toHaveCount(0);
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(1);
  await expect.poll(async () => (await rows(page)).length).toBe(before - 1);
  await expect.poll(async () => (await toast(page))?.text).toBe("2 threads merged");
  // The rows it acted on are gone, so the selection that pointed at them is gone with them.
  await expect(page.locator(".action-bar")).toHaveCount(0);

  await page.locator(".row", { hasText: "Maya Raghunathan" }).click();
  const banner = page.locator(".thread-banner .banner", { hasText: "Merged from" });
  await expect(banner).toContainText("2 threads");
  await expect(banner).toContainText("Lease renewal for the studio");
  await expect(banner.locator(".banner-action")).toHaveText("Unmerge");

  await banner.locator(".banner-action").click();
  await expect(page.locator(".thread-banner .banner", { hasText: "Merged from" })).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toBe("Unmerged");
});

test("Clips lists what was saved and goes back to the thread", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await openPlace(page, "Clips");
  await expect(page.locator(".list-title")).toHaveText("Clips");

  const clips = page.locator(".clip");
  await expect(clips).toHaveCount(2);
  await expect(clips.nth(0).locator(".clip-text")).toContainText(
    "Graphite, cedar, the Napoleonic wars",
  );
  await expect(clips.nth(1).locator(".clip-meta")).toContainText("Russell Young");
  await expect(clips.nth(1).locator(".clip-meta")).toContainText("Pumpkin bread recipe");

  // Each one links back, which is the only thing a clip is for.
  await clips.nth(1).locator(".clip-open").click();
  await expect(page.locator(".thread-subject")).toHaveText("Pumpkin bread recipe");
  await expect(page.locator(".list-title")).toHaveText("Inbox");
});

test("All files is a grid that filters by type and by sender", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await openPlace(page, "All files");
  await expect(page.locator(".list-title")).toHaveText("All files");

  const cards = page.locator(".file-card");
  await expect(cards.first()).toBeVisible();
  const all = await cards.count();
  expect(all).toBeGreaterThan(2);

  await page.locator(".files-filters .button", { hasText: "Images" }).click();
  await expect(cards).toHaveCount(2);
  for (const name of await page.locator(".file-name").allTextContents()) {
    expect(name).toMatch(/\.png$/);
  }

  await page.locator(".files-filters .button", { hasText: "PDFs" }).click();
  await expect.poll(async () => await cards.count()).toBeLessThan(all);
  for (const mark of await page.locator(".file-mark").allTextContents()) {
    expect(mark).toBe("PDF");
  }

  // And by sender, which narrows what is already on screen rather than starting again.
  const senders = () =>
    page.evaluate(() =>
      [...document.querySelectorAll(".file-card")].map((card) =>
        ((card.querySelectorAll(".file-meta")[0]?.textContent ?? "").split(" · ")[0] ?? "").trim(),
      ),
    );
  const [who] = await senders();
  expect(new Set(await senders()).size).toBeGreaterThan(1);
  await page.locator(".files-picker select").selectOption({ label: who });
  await expect.poll(async () => [...new Set(await senders())]).toEqual([who]);

  await page.locator(".files-filters .button", { hasText: "Everything" }).click();
  await page.locator(".files-picker select").selectOption("");
  await expect.poll(async () => await cards.count()).toBe(all);

  // Opening a card opens the thread it is in. Nothing was fetched to get here.
  await cards.first().click();
  await expect(page.locator(".thread-subject")).toBeVisible();
});
