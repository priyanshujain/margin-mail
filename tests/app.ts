// What every spec needs: a seeded load of the real app, and the handful of measurements this UI has
// to be judged on.
//
// Nothing here reaches into the app's internals. There is no test id anywhere in `src/`, so these
// helpers read what the browser actually laid out: rectangles, computed styles, rendered text. That
// is deliberate. A list row is two lines at 58 pixels or it is not, and a test that asked the app
// what it thought it had rendered would agree with the bug.
//
// `contrastOf` is ported from the calendar unchanged. It composites every translucent layer between
// the text and the page, which matters here because a selected row is a wash over a band over
// paper, and reading one background colour would measure the wrong thing.

import { expect, type Locator, type Page } from "@playwright/test";

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
  top: number;
  bottom: number;
  left: number;
  right: number;
}

export interface OpenOptions {
  theme?: "light" | "dark";
  /** Off means the list takes the window and a thread opens in place. */
  readingPane?: boolean;
  /** Extra localStorage entries, written before the app's first script runs. */
  storage?: Record<string, string>;
  /** Start with nothing connected, which is the only way to reach the connect screen. */
  firstRun?: boolean;
  /** The route to open. Defaults to the app itself. */
  hash?: string;
  now?: Date;
}

/**
 * A fixed instant on the day the suite is being run, at a given hour of the pinned zone.
 *
 * The fixture is anchored to the local day, so a thread that says "09:40" says it because of when
 * the suite ran. A test that reads a time has to say which hour it means, the same way it says
 * which theme it means. Asia/Kolkata is UTC+5:30 the whole year round, so the offset is written out.
 */
export function clockAt(hour: number, minute = 0): Date {
  const day = new Intl.DateTimeFormat("en-CA", {
    timeZone: "Asia/Kolkata",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(new Date());
  const pad = (n: number) => String(n).padStart(2, "0");
  return new Date(`${day}T${pad(hour)}:${pad(minute)}:00+05:30`);
}

/** The middle of the working day, which is when a mailbox looks like a mailbox. */
export const MIDDAY = () => clockAt(11, 30);

const SEEDED = "__test-seeded";

/**
 * Loads the app with a known store. The seed is written once per context rather than on every
 * navigation, so a test that reloads to check what survived is not silently reset underneath it.
 */
export async function openApp(page: Page, options: OpenOptions = {}): Promise<void> {
  const seed: Record<string, string> = {
    "marginmail-theme": options.theme ?? "light",
    "marginmail-pane": options.readingPane === false ? "0" : "1",
    ...(options.firstRun ? { "marginmail-dev-empty": "1" } : {}),
    ...(options.storage ?? {}),
  };

  await page.addInitScript(
    ({ values, flag }) => {
      try {
        if (localStorage.getItem(flag) === "1") return;
        localStorage.clear();
        for (const [key, value] of Object.entries(values)) localStorage.setItem(key, value);
        localStorage.setItem(flag, "1");
      } catch {
        /* a context without storage is not a context this app runs in */
      }
    },
    { values: seed, flag: SEEDED },
  );

  if (options.now) await page.clock.setFixedTime(options.now);
  await page.goto(options.hash ? `/${options.hash}` : "/");
}

/** The list has laid out and the fixture has arrived. */
export async function listReady(page: Page): Promise<void> {
  await expect(page.locator(".row").first()).toBeVisible();
  await settle(page);
}

/** Two frames, which is long enough for a layout pass and the render it causes. */
export function settle(page: Page): Promise<void> {
  return page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      }),
  );
}

export async function box(target: Locator): Promise<Box> {
  const rect = await target.boundingBox();
  if (!rect) throw new Error("element has no box");
  return {
    ...rect,
    top: rect.y,
    bottom: rect.y + rect.height,
    left: rect.x,
    right: rect.x + rect.width,
  };
}

export interface RowInfo {
  sender: string;
  subject: string;
  snippet: string;
  time: string;
  height: number;
  top: number;
  /** The dot in the gutter, which is the only thing that says a thread is new. */
  unseen: boolean;
  /** The message count, printed between sender and time only when there is more than one. */
  count: string;
  selected: boolean;
  /** The group head this row sits under, read off the DOM rather than asked of the app. */
  group: string;
  note: string | null;
}

/** Every row on screen, measured and read. */
export function rows(page: Page): Promise<RowInfo[]> {
  return page.evaluate(() => {
    const text = (el: Element | null) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();
    let group = "";
    const out: RowInfo[] = [];
    const walk = document.querySelectorAll<HTMLElement>(".group-head, .row");
    for (const el of walk) {
      if (el.classList.contains("group-head")) {
        group = text(el.querySelector(".group-head-label") ?? el);
        continue;
      }
      const rect = el.getBoundingClientRect();
      out.push({
        sender: text(el.querySelector(".row-sender")),
        subject: text(el.querySelector(".row-subject")),
        snippet: text(el.querySelector(".row-snippet")),
        time: text(el.querySelector(".row-time")),
        height: rect.height,
        top: rect.top,
        // The DTO calls it `unseen` and the row calls it new, because the group above it is
        // called New for you and the dot is the new-mail dot. Same fact, two vocabularies.
        unseen: el.hasAttribute("data-new"),
        count: text(el.querySelector(".row-count")),
        selected: el.hasAttribute("data-selected"),
        group,
        note: el.querySelector(".row-note") ? text(el.querySelector(".row-note")) : null,
      });
    }
    return out;
  });
}

/** The group heads in order, which is what "the Inbox has two groups" means. */
export function groups(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll(".group-head-label")].map((el) =>
      (el.textContent ?? "").replace(/\s+/g, " ").trim(),
    ),
  );
}

/** The value of a CSS custom property on the root, for the geometry the mockups pin down. */
export function token(page: Page, name: string): Promise<string> {
  return page.evaluate(
    (property) => getComputedStyle(document.documentElement).getPropertyValue(property).trim(),
    name,
  );
}

/** The open dialog's accessible name, or null. Every overlay in the app is one. */
export function openDialog(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const dialog = document.querySelector('[role="dialog"]');
    return dialog ? dialog.getAttribute("aria-label") : null;
  });
}

/**
 * The message bodies, which live in sandboxed iframes and so cannot be read with an ordinary
 * locator. Returns each frame's rendered text, which is enough to assert that a body arrived,
 * that quoted text is not in it, and that a blocked image left no broken picture behind.
 */
export async function bodyText(page: Page): Promise<string[]> {
  const frames = page.frames().filter((f) => f !== page.mainFrame());
  const out: string[] = [];
  for (const frame of frames) {
    out.push(await frame.evaluate(() => document.body.innerText.replace(/\s+/g, " ").trim()));
  }
  return out;
}

export interface Contrast {
  ratio: number;
  text: string;
  background: string;
}

/**
 * Contrast of a piece of text against whatever is actually behind it, compositing every translucent
 * layer between the element and the page.
 */
export function contrastOf(page: Page, selector: string): Promise<Contrast | null> {
  return page.evaluate((sel) => {
    const el = document.querySelector<HTMLElement>(sel);
    if (!el) return null;

    // Resolved through a canvas rather than by reading the string, so any colour space works.
    // getComputedStyle hands back whatever syntax the author wrote, and a modern colour function
    // falls through an rgb regex as transparent, which reports a readable row as 1:1.
    const probe = document.createElement("canvas");
    probe.width = 1;
    probe.height = 1;
    const ctx = probe.getContext("2d", { willReadFrequently: true })!;

    const parse = (value: string): [number, number, number, number] => {
      const match = value.match(/^rgba?\(([^)]+)\)$/);
      if (match) {
        const parts = match[1].split(/[,\s/]+/).filter(Boolean).map(Number);
        return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0, parts[3] ?? 1];
      }
      if (!value || value === "transparent" || value === "none") return [0, 0, 0, 0];
      ctx.clearRect(0, 0, 1, 1);
      ctx.fillStyle = "rgba(0, 0, 0, 0)";
      ctx.fillStyle = value;
      ctx.fillRect(0, 0, 1, 1);
      const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
      return [r, g, b, a / 255];
    };

    const over = (
      top: [number, number, number, number],
      bottom: [number, number, number],
    ): [number, number, number] => [
      top[0] * top[3] + bottom[0] * (1 - top[3]),
      top[1] * top[3] + bottom[1] * (1 - top[3]),
      top[2] * top[3] + bottom[2] * (1 - top[3]),
    ];

    const layers: [number, number, number, number][] = [];
    for (let node: HTMLElement | null = el; node; node = node.parentElement) {
      layers.push(parse(getComputedStyle(node).backgroundColor));
    }
    let background: [number, number, number] = [255, 255, 255];
    for (let i = layers.length - 1; i >= 0; i--) background = over(layers[i], background);

    const text = over(parse(getComputedStyle(el).color), background);

    const channel = (c: number) => {
      const s = c / 255;
      return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    };
    const luminance = (rgb: [number, number, number]) =>
      0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2]);

    const a = luminance(text);
    const b = luminance(background);
    const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
    const show = (rgb: [number, number, number]) => rgb.map((n) => Math.round(n)).join(",");
    return { ratio: Math.round(ratio * 100) / 100, text: show(text), background: show(background) };
  }, selector);
}

export interface MessageInfo {
  name: string;
  address: string;
  time: string;
  /** Older messages are one line: a preview instead of a body. */
  collapsed: boolean;
  preview: string;
}

/** The messages in the reading pane, read the way a person reads them: heads, not state. */
export function paneMessages(page: Page): Promise<MessageInfo[]> {
  return page.evaluate(() => {
    const text = (el: Element | null) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();
    return [...document.querySelectorAll<HTMLElement>(".msg")].map((el) => ({
      name: text(el.querySelector(".msg-name")).replace(text(el.querySelector(".addr")), "").trim(),
      address: text(el.querySelector(".msg-name .addr")),
      time: text(el.querySelector(".msg-time")),
      collapsed: el.hasAttribute("data-collapsed"),
      preview: text(el.querySelector(".msg-preview")),
    }));
  });
}

export interface PaletteRow {
  group: string;
  label: string;
  keys: string[];
}

/** Every row of the open palette, with the group it sits under and the caps it prints. */
export function paletteRows(page: Page): Promise<PaletteRow[]> {
  return page.evaluate(() => {
    const text = (el: Element | null) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();
    const out: PaletteRow[] = [];
    for (const section of document.querySelectorAll(".palette-list > li")) {
      const group = text(section.querySelector(".palette-group"));
      for (const row of section.querySelectorAll(".palette-row")) {
        out.push({
          group,
          label: text(row.querySelector(".palette-label")),
          keys: [...row.querySelectorAll(".palette-keys .key")].map((k) => text(k)),
        });
      }
    }
    return out;
  });
}

/** Opens the row at `index` by clicking it, and waits for the thread to arrive in the pane. */
export async function openRow(page: Page, index: number): Promise<void> {
  await page.locator(".row").nth(index).click();
  await expect(page.locator(".thread-subject")).toBeVisible();
  await settle(page);
}

/**
 * Makes the fixture refuse a command, which is the only way to see what a failed call looks like:
 * the dev backend is in the page rather than on the wire, so there is no request to intercept.
 *
 * The module request itself is answered instead, with a shim that forwards to the real fixture and
 * throws for the commands named. Nothing in `src/` knows this happened, which is the point: the
 * app under test is the app that ships.
 */
export async function failCommands(page: Page, commands: string[]): Promise<void> {
  await page.route("**/src/dev/mockIpc.ts*", async (route) => {
    // The shim's own import of the real thing carries a query, and answering that one too would
    // be a module that imports itself.
    if (new URL(route.request().url()).searchParams.has("real")) return route.fallback();
    await route.fulfill({
      contentType: "text/javascript",
      body: [
        `import { mockCall as real } from "/src/dev/mockIpc.ts?real";`,
        `const refused = ${JSON.stringify(commands)};`,
        `export function mockCall(command, args) {`,
        `  if (refused.includes(command)) return Promise.reject(new Error("the mailbox is offline"));`,
        `  return real(command, args);`,
        `}`,
      ].join("\n"),
    });
  });
}

/** The toast at the foot of the window: what it says, and whether it offers a way back. */
export async function toast(page: Page): Promise<{ text: string; action: string | null } | null> {
  return page.evaluate(() => {
    const el = document.querySelector(".toast");
    if (!el) return null;
    const text = (el.querySelector(".toast-text")?.textContent ?? "").replace(/\s+/g, " ").trim();
    const action = el.querySelector(".toast-action");
    return { text, action: action ? (action.textContent ?? "").trim() : null };
  });
}

/** Every button on the selection's action bar, with the key it prints. */
export function actionBar(page: Page): Promise<{ label: string; key: string }[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>(".action-verbs .button")].map((el) => {
      const key = (el.querySelector(".key")?.textContent ?? "").trim();
      const all = (el.textContent ?? "").replace(/\s+/g, " ").trim();
      return { label: (key && all.endsWith(key) ? all.slice(0, -key.length) : all).trim(), key };
    }),
  );
}

/** The rows the selection holds, read off the checkboxes rather than asked of the store. */
export function checkedRows(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>(".row[data-checked]")].map((el) =>
      (el.querySelector(".row-sender")?.textContent ?? "").trim(),
    ),
  );
}

/** How far down the list has been scrolled, which a place has to get back when search gives it up. */
export function listScroll(page: Page): Promise<number> {
  return page.evaluate(() => document.querySelector(".list")?.scrollTop ?? 0);
}

/** Which place is up, read off the root rather than asked of the app. */
export function place(page: Page): Promise<string | null> {
  return page.evaluate(() => document.querySelector(".app")?.getAttribute("data-place") ?? null);
}
