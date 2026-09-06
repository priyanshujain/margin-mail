// Snooze: the picker, where the thread goes, and how it comes back.
//
// Every moment here is computed in the browser from the four times in settings, so the clock is
// pinned and the zone is the config's. A test that let the machine's own hour decide what
// "Tomorrow" meant would pass in one timezone and not in another.

import { expect, test, type Page } from "@playwright/test";
import { failCommands, listReady, MIDDAY, openApp, rows, toast } from "./app";

/** Focuses the row a spec means to act on, by walking to it rather than by clicking it open. */
async function focusRow(page: Page, sender: string): Promise<void> {
  for (let i = 0; i < 20; i++) {
    const at = (await rows(page)).find((row) => row.selected);
    if (at?.sender === sender) return;
    await page.keyboard.press("j");
  }
  throw new Error(`never reached ${sender}`);
}

/** A day in the pinned zone, as `<input type="date">` wants it. */
function day(offset: number): string {
  const today = new Intl.DateTimeFormat("en-CA", {
    timeZone: "Asia/Kolkata",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(new Date());
  const at = new Date(`${today}T00:00:00Z`);
  at.setUTCDate(at.getUTCDate() + offset);
  return at.toISOString().slice(0, 10);
}

/** Every row of the open picker: what it says, and the key that takes it. */
function choices(page: Page): Promise<{ label: string; keycap: string; when: string }[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>(".snooze-option")].map((el) => ({
      label: (el.querySelector(".snooze-label")?.textContent ?? "").trim(),
      keycap: (el.querySelector(".key")?.textContent ?? "").trim(),
      when: (el.querySelector(".snooze-when")?.textContent ?? "").trim(),
    })),
  );
}

test("b opens the six choices with their keys and the moment each of them means", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("b");
  await expect(page.locator(".snooze-picker")).toBeVisible();

  // The times are the settings file's, not this screen's: three hours, 08:00, Saturday 09:00,
  // Monday 08:00. Pinned at 11:30, later today is 14:30.
  expect(await choices(page)).toEqual([
    { label: "Later today", keycap: "1", when: "Today 14:30" },
    { label: "Tomorrow", keycap: "2", when: "Tomorrow 08:00" },
    { label: "This weekend", keycap: "3", when: expect.stringContaining("09:00") },
    { label: "Next week", keycap: "4", when: expect.stringContaining("08:00") },
    { label: "Pick a date and time", keycap: "5", when: "" },
    { label: "If no reply by", keycap: "6", when: "" },
  ]);

  // The picker is in front, so the number keys mean these six rather than the places.
  await page.keyboard.press("Escape");
  await expect(page.locator(".snooze-picker")).toHaveCount(0);
  await expect(page.locator(".list-title")).toHaveText("Inbox");
});

test("b comes up before the four times are here, and says it is reading them", async ({ page }) => {
  await openApp(page, { now: MIDDAY(), storage: { "marginmail-dev-pending": "1" } });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("b");

  // Nothing reads settings before this key does, so the popover comes up into the wait rather
  // than after it. A `b` that showed nothing until then was a `b` that got pressed twice.
  const picker = page.locator(".snooze-picker");
  await expect(picker).toBeVisible();
  await expect(picker).toHaveAttribute("data-state", "loading");
  await expect(picker.locator(".snooze-wait")).toHaveText("Reading your snooze times");
  await expect(page.locator(".snooze-option")).toHaveCount(0);

  await expect(page.locator(".snooze-option")).toHaveCount(6, { timeout: 5_000 });
  await expect(picker).not.toHaveAttribute("data-state", /./);
  expect((await choices(page))[0]).toEqual({ label: "Later today", keycap: "1", when: "Today 14:30" });
});

test("b says why when the settings could not be read", async ({ page }) => {
  await failCommands(page, ["settings_get"]);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("b");

  const picker = page.locator(".snooze-picker");
  await expect(picker).toHaveAttribute("data-state", "error");
  await expect(picker.locator(".snooze-wait")).toContainText("Could not read your snooze times");
  await expect(picker.locator(".snooze-wait")).toContainText("the mailbox is offline");
  await expect(page.locator(".snooze-option")).toHaveCount(0);

  // Escape still puts it away, and nothing was chosen, so nothing moved.
  await page.keyboard.press("Escape");
  await expect(picker).toHaveCount(0);
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(1);
});

test("the toast's Undo puts the thread back and says so", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("b");
  await expect(page.locator(".snooze-picker")).toBeVisible();
  await page.keyboard.press("1");
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toBe("Snoozed");

  await page.locator(".toast-action").click();
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(1);
  await expect.poll(async () => (await toast(page))?.text).toBe("Undone: Snoozed");
  expect((await toast(page))?.action).toBeNull();
});

test("choosing one takes the thread out of the list, and 7 shows it with its return time", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("b");
  await expect(page.locator(".snooze-picker")).toBeVisible();
  await page.keyboard.press("2");

  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(0);
  await expect.poll(async () => (await toast(page))?.text).toBe("Snoozed");
  expect((await toast(page))?.action).toContain("Undo");

  await page.keyboard.press("7");
  await expect(page.locator(".list-title")).toHaveText("Snoozed");
  // In this place the time slot is about the return rather than about when the mail arrived, and
  // the moment is the one the picker offered.
  await expect
    .poll(async () => (await rows(page)).find((row) => row.sender === "Maya Raghunathan")?.time)
    .toBe("Tomorrow 08:00");
  for (const row of await rows(page)) expect(row.time).toMatch(/\d\d:\d\d$/);
});

test("a thread that is late says so, and comes back into Back above New for you", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await focusRow(page, "Maya Raghunathan");
  await page.keyboard.press("b");
  await expect(page.locator(".snooze-picker")).toBeVisible();
  await page.keyboard.press("5");
  await page.locator(".snooze-field input").fill(`${day(-1)}T09:00`);
  await page.locator(".snooze-form-foot .button", { hasText: "Snooze" }).click();
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(0);

  // Due and not yet returned: the row says it is late rather than printing a time that has gone.
  await page.keyboard.press("7");
  await expect
    .poll(async () => (await rows(page)).find((row) => row.sender === "Maya Raghunathan")?.time)
    .toBe("Due yesterday");

  // Nothing runs in the background, so the return happens when somebody looks. The window coming
  // back to the foreground is one of the three moments that count.
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(page.locator(".row", { hasText: "Maya Raghunathan" })).toHaveCount(0);

  await page.keyboard.press("1");
  await expect
    .poll(async () => (await rows(page)).find((row) => row.sender === "Maya Raghunathan")?.group)
    .toBe("Back");
  const groups = (await rows(page)).map((row) => row.group);
  expect(groups.indexOf("Back")).toBeLessThan(groups.indexOf("New for you"));
});

test("the picker acts on a selection, and Escape leaves it alone", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("j");
  await page.keyboard.press("x");
  await page.keyboard.press("Shift+J");
  await page.keyboard.press("b");
  await expect(page.locator(".snooze-picker")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.locator(".snooze-picker")).toHaveCount(0);
  // Nothing was chosen, so nothing moved and nothing was said.
  expect(await toast(page)).toBeNull();

  await page.keyboard.press("b");
  await expect(page.locator(".snooze-picker")).toBeVisible();
  await page.keyboard.press("1");
  await expect.poll(async () => (await toast(page))?.text).toBe("2 threads snoozed");
  await page.keyboard.press("7");
  await expect.poll(async () => (await rows(page)).length).toBe(3);
});
