// What the list costs while a mailbox is still coming in.
//
// The other suites read what is on screen. This one reads what the app did to put it there: how
// many times it asked SQLite for a page, how many rows it drew to show the same thing, and whether
// it said out loud that it was busy. All three were the difference between this list and one that
// stays smooth through a sync, and none of them is visible in a screenshot.
//
// Nothing here reaches into the app either. The two counters are module shims served in place of
// `src/dev/mockIpc.ts` and `src/ui/Avatar.tsx`, the same trick `failCommands` uses: the app under
// test is the app that ships, and what is counted is what it really called and really drew.

import { expect, test, type Page } from "@playwright/test";
import { listReady, MIDDAY, openApp, openRow, rows, settle, toast } from "./app";

declare global {
  interface Window {
    /** Every IPC command the app made, by name. Present only under `countCommands`. */
    __calls?: Record<string, number>;
    /** How many times a row has drawn its avatar, which is once per row render. */
    __rows?: number;
    /** Lets the next page through. Present only under `failNextPage`. */
    __allowPage?: boolean;
  }
}

/**
 * A fixture that says every list has another page and then refuses to produce it, until the test
 * says otherwise. The fixture's own pages fit on one screen, so this is the only way to reach the
 * foot of a list that is still paging.
 */
async function failNextPage(page: Page): Promise<void> {
  await page.route("**/src/dev/mockIpc.ts*", async (route) => {
    if (new URL(route.request().url()).searchParams.has("real")) return route.fallback();
    await route.fulfill({
      contentType: "text/javascript",
      body: [
        `import { mockCall as real } from "/src/dev/mockIpc.ts?real";`,
        `export function mockCall(command, args) {`,
        `  if (command !== "threads_list") return real(command, args);`,
        `  if (args.query.cursor === null) {`,
        `    return real(command, args).then((page) => ({ ...page, nextCursor: "more" }));`,
        `  }`,
        `  if (!window.__allowPage) return Promise.reject(new Error("the mailbox is offline"));`,
        `  return Promise.resolve({ threads: [], nextCursor: null, footer: null });`,
        `}`,
      ].join("\n"),
    });
  });
}

/**
 * Counts every command the app sends, and forwards each one to the fixture unchanged.
 *
 * `threads_list` is counted under its place as well as under its own name, because three parts of
 * this screen run that one command: the list, and each of the two piles at the foot of it. Only
 * the first of them is what a `store-changed` is supposed to cost.
 */
async function countCommands(page: Page): Promise<void> {
  await page.route("**/src/dev/mockIpc.ts*", async (route) => {
    if (new URL(route.request().url()).searchParams.has("real")) return route.fallback();
    await route.fulfill({
      contentType: "text/javascript",
      body: [
        `import { mockCall as real } from "/src/dev/mockIpc.ts?real";`,
        `window.__calls = {};`,
        `const count = (key) => { window.__calls[key] = (window.__calls[key] ?? 0) + 1; };`,
        `export function mockCall(command, args) {`,
        `  count(command);`,
        `  if (command === "threads_list") count(command + " " + args.query.place);`,
        `  return real(command, args);`,
        `}`,
      ].join("\n"),
    });
  });
}

const calls = (page: Page, command: string): Promise<number> =>
  page.evaluate((name) => window.__calls?.[name] ?? 0, command);

/**
 * Counts row renders, by counting the avatar every row draws.
 *
 * The avatar is inside the row and nothing else in the list has one, so it renders exactly when
 * its row does and never otherwise: a memoised row that decides it has nothing new to say never
 * reaches this. Counting here rather than in `Row` itself is what keeps the measurement honest,
 * because a counter wrapped around a memoised component would be counting the wrapper.
 *
 * The real component is called rather than mounted as a child, because a route that is fulfilled
 * by hand is served to the browser rather than to Vite and a bare `react` import would not
 * resolve. It takes props and returns markup, so calling it is what mounting it would have done.
 */
async function countRowRenders(page: Page): Promise<void> {
  await page.route("**/src/ui/Avatar.tsx*", async (route) => {
    if (new URL(route.request().url()).searchParams.has("real")) return route.fallback();
    await route.fulfill({
      contentType: "text/javascript",
      body: [
        `import { Avatar as real } from "/src/ui/Avatar.tsx?real";`,
        `export * from "/src/ui/Avatar.tsx?real";`,
        `window.__rows = 0;`,
        `export function Avatar(props) {`,
        `  window.__rows++;`,
        `  return real(props);`,
        `}`,
      ].join("\n"),
    });
  });
}

/** The backend's invalidation, as the browser half of `src/ipc.ts` receives it. */
async function storeChanged(page: Page, scope: string, times = 1): Promise<void> {
  await page.evaluate(
    ({ reason, count }) => {
      for (let i = 0; i < count; i++) {
        window.setTimeout(
          () => window.dispatchEvent(new CustomEvent("store-changed", { detail: reason })),
          i * 20,
        );
      }
    },
    { reason: scope, count: times },
  );
}

/** One account's status, as `sync-progress` carries it. */
async function syncProgress(page: Page, status: Record<string, unknown>): Promise<void> {
  await page.evaluate((detail) => {
    window.dispatchEvent(new CustomEvent("sync-progress", { detail }));
  }, status);
}

/** The list's own query, as `countCommands` records it. */
const INBOX_QUERY = "threads_list inbox";

/** Long enough for the coalescing window to close and the query it holds to come back. */
const AFTER_THE_BURST = 700;

test("a burst of store-changed costs the list one query rather than one per event", async ({
  page,
}) => {
  await countCommands(page);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const before = await calls(page, INBOX_QUERY);
  // A sync pass that touched anything says so, and while a mailbox is filling in every pass does.
  await storeChanged(page, "threads state", 6);
  await page.waitForTimeout(AFTER_THE_BURST);

  expect(await calls(page, INBOX_QUERY)).toBe(before + 1);
  // And the burst left the list where it was rather than emptying it for a frame.
  expect((await rows(page)).length).toBeGreaterThan(0);
});

test("a body landing behind an open thread re-reads that thread and leaves the list alone", async ({
  page,
}) => {
  await countCommands(page);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openRow(page, 0);

  const subject = await page.locator(".thread-subject").textContent();
  const lists = await calls(page, INBOX_QUERY);
  const views = await calls(page, "thread_view");

  // What the cache warmer emits, several times a second, for every body it brings in.
  await storeChanged(page, "thread", 3);
  await page.waitForTimeout(AFTER_THE_BURST);

  expect(await calls(page, INBOX_QUERY)).toBe(lists);
  expect(await calls(page, "thread_view")).toBe(views + 3);
  // Read again in place: the same thread, no flash, no scroll.
  expect(await page.locator(".thread-subject").textContent()).toBe(subject);
});

test("a row that has not changed does not draw again when the list reloads", async ({ page }) => {
  await countRowRenders(page);
  await countCommands(page);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const drawn = await page.evaluate(() => window.__rows ?? 0);
  expect(drawn).toBeGreaterThan(0);
  const before = await calls(page, INBOX_QUERY);

  // A reload that really did happen, with a page of summaries that are new objects saying exactly
  // what the old ones said, which is what every sync pass hands the list.
  await storeChanged(page, "threads");
  await expect.poll(() => calls(page, INBOX_QUERY)).toBe(before + 1);
  await settle(page);

  expect(await page.evaluate(() => window.__rows ?? 0)).toBe(drawn);

  // And the other half of the bargain: a row whose state did change draws again. `j` takes the
  // focus, which is a row that has to pick up the wash and the edge.
  await page.keyboard.press("j");
  await settle(page);
  expect(await page.evaluate(() => window.__rows ?? 0)).toBeGreaterThan(drawn);
});

test("the header says what sync is doing, and stops when it is idle", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  const line = page.locator(".sync-busy");
  await expect(line).toHaveCount(0);

  const caching = {
    accountId: "acct-1",
    phase: "caching",
    lastSyncMs: null,
    error: null,
    pendingWrites: 0,
    message: "Caching recent mail",
    hydrated: 1_204,
    total: 3_380,
    oldestMs: null,
  };
  await syncProgress(page, caching);
  await expect(line).toHaveText("Caching recent mail 1,204 of 3,380");

  // Quiet, and nothing to press: a status that could take a click is a status that can be in the
  // way of the mail behind it.
  expect(await line.evaluate((el) => el.closest("button, a") !== null)).toBe(false);
  // And the list is still the list while it says so.
  expect((await rows(page)).length).toBeGreaterThan(0);

  await syncProgress(page, { ...caching, phase: "idle", message: null, hydrated: 0, total: 0 });
  await expect(line).toHaveCount(0);
});

test("a page that did not come is one line at the foot with a way to ask again", async ({ page }) => {
  await failNextPage(page);
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  // The list is shorter than the window, so the foot is reached on arrival and the next page is
  // asked for at once. It is refused, and the foot says so; the rows above it are still the rows.
  const foot = page.locator('.list-foot[data-state="error"]');
  await expect(foot).toContainText("The rest of the list did not arrive.");
  await expect(foot.locator(".button")).toHaveText("Try again");
  expect((await rows(page)).length).toBeGreaterThan(0);
  // Said at the foot, where the way back is, and not as a toast.
  expect(await toast(page)).toBeNull();

  await page.evaluate(() => {
    window.__allowPage = true;
  });
  await foot.locator(".button").click();
  await expect(page.locator('.list-foot[data-state="error"]')).toHaveCount(0);
});

test("a long subject takes the width before the snippet gets any of it", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  // The row with the longest subject in the fixture, which is the one that used to be cut in half
  // beside a snippet that was cut in half too.
  const widths = await page.evaluate(() => {
    const out: { subject: string; subjectWidth: number; snippetWidth: number; full: boolean }[] = [];
    for (const row of document.querySelectorAll<HTMLElement>(".row")) {
      const subject = row.querySelector<HTMLElement>(".row-subject");
      const snippet = row.querySelector<HTMLElement>(".row-snippet");
      if (!subject || !snippet) continue;
      out.push({
        subject: (subject.textContent ?? "").trim(),
        subjectWidth: subject.getBoundingClientRect().width,
        snippetWidth: snippet.getBoundingClientRect().width,
        // Nothing of it is cut off.
        full: subject.scrollWidth <= subject.clientWidth,
      });
    }
    return out;
  });

  expect(widths.length).toBeGreaterThan(0);
  const longest = widths.reduce((a, b) => (b.subject.length > a.subject.length ? b : a));
  expect(longest.full).toBe(true);
  // The old rule gave the subject 62% of the line whatever it needed, so the longest subject in
  // the list was truncated with the snippet beside it holding room it could not use.
  expect(longest.subjectWidth).toBeGreaterThan(longest.snippetWidth);
});
