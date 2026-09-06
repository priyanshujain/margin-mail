// The guide, which is a panel over the whole window: the mail stays where it was, nothing behind it
// can be pressed while it is up, and closing it puts back exactly what was underneath.
//
// Nothing here reads the app's own idea of what it drew. The rail is what the browser laid out, the
// sections are the group heads any list in this app draws, and the one keycap asserted is read out
// of the binding table rather than typed in, so a remapped key moves the assertion with it.

import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { MIDDAY, box, listReady, openApp, openRow, place, settle } from "./app";
import { keyFor } from "../src/keys/bindings";

const shots = join(fileURLToPath(new URL("..", import.meta.url)), "screenshots");

const SECTIONS = [
  "Getting started",
  "The Screener",
  "Reading",
  "Triage",
  "Writing",
  "Organising",
  "Accounts",
  "Questions",
];

/** Every picture the guide is allowed to show, which is the set another package captures. */
const PICTURES = [
  "compose",
  "contact-card",
  "feed",
  "inbox",
  "palette",
  "piles",
  "screener",
  "settings",
  "shortcuts",
  "snooze",
];

/**
 * The size a picture should be drawn at: half its own pixels, because every capture is at twice the
 * size. Read out of the PNG header rather than out of the app, so this asserts what the file says
 * and not what the app believes about it. Null when the captures are not on this checkout, since
 * nothing here may depend on a picture existing.
 */
function natural(name: string): { width: number; height: number } | null {
  const file = join(fileURLToPath(new URL("..", import.meta.url)), "public", "guide", `${name}.png`);
  if (!existsSync(file)) return null;
  const header = readFileSync(file);
  return { width: header.readUInt32BE(16) / 2, height: header.readUInt32BE(20) / 2 };
}

/** The open dialog's accessible name, which is how a panel says it is the one in front. */
const dialog = (page: Page) =>
  page.evaluate(() => document.querySelector('[role="dialog"]')?.getAttribute("aria-label") ?? null);

const tabs = (page: Page) =>
  page.locator(".guide-tab").evaluateAll((all) => all.map((tab) => tab.textContent ?? ""));

/**
 * The section labels in the rail. Scoped to the rail rather than read off the page, because the
 * guide is a panel over the app now and the list underneath it draws group heads of its own.
 */
const railGroups = (page: Page) =>
  page
    .locator(".guide-rail .group-head-label")
    .evaluateAll((all) => all.map((el) => (el.textContent ?? "").replace(/\s+/g, " ").trim()));

/** The one field, which is now the first thing on the panel rather than the head of the rail. */
const search = (page: Page) => page.locator(".guide-search input");

/**
 * Holds what a press hands to the browser instead of letting it hand it over. The ask button leaves
 * the app for a public website, and a suite that pressed it for real would open a tab on GitHub
 * every time it ran.
 */
async function heldOpen(page: Page): Promise<void> {
  await page.evaluate(() => {
    const held: string[] = [];
    (window as unknown as { asked: string[] }).asked = held;
    window.open = ((url?: string | URL) => {
      held.push(String(url));
      return null;
    }) as typeof window.open;
  });
}

const asked = (page: Page) =>
  page.evaluate(() => (window as unknown as { asked: string[] }).asked ?? []);

/** The palette is the way in: the guide has no key of its own and does not want one. */
async function openGuide(page: Page): Promise<void> {
  await page.keyboard.press("Meta+k");
  await page.locator(".palette-input").fill("guide");
  await page.keyboard.press("Enter");
  await expect(page.locator(".guide")).toBeVisible();
  await settle(page);
}

test("the palette opens the guide over the whole window", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  expect(await dialog(page)).toBeNull();
  await openGuide(page);
  expect(await dialog(page)).toBe("Guide");
  await expect(page.locator(".guide-title")).toHaveText("What is different here");

  // The app is still there and none of it can be reached: the title bar carries a stacking order of
  // its own, and a scrim that let its buttons through would be a modal in name only.
  await expect(page.locator(".list-col")).toBeVisible();
  const over = await page.evaluate(() => {
    const button = document.querySelector(".titlebar .segment button");
    if (!button) return "no button";
    const box = button.getBoundingClientRect();
    const top = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
    return top?.className ?? "nothing";
  });
  expect(over).toBe("overlay");

  mkdirSync(shots, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.screenshot({ path: join(shots, "guide.png") });
});

test("the rail lists the sections, and one article is on screen at a time", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openGuide(page);

  expect(await railGroups(page)).toEqual(SECTIONS);
  const all = await tabs(page);
  expect(all.length).toBeGreaterThan(SECTIONS.length);
  expect(all).toContain("How the Screener works");

  await page.locator(".guide-tab", { hasText: "How the Screener works" }).click();
  await expect(page.locator(".guide-title")).toHaveText("How the Screener works");
  await expect(page.locator(".guide-tab[data-active]")).toHaveText("How the Screener works");

  // One article, and only one. The rail is where the others are.
  await expect(page.locator(".guide-article")).toHaveCount(1);
  await expect(page.locator(".guide-title")).toHaveCount(1);
  await expect(page.locator(".guide-article")).toContainText("one card per sender");
  await expect(page.locator(".guide-article")).not.toContainText("What is different here");
});

test("the search narrows the rail, by a word in a body as well as in a title", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openGuide(page);

  const whole = (await tabs(page)).length;

  // A word that is only ever in one article's prose, and in nobody's title.
  await search(page).fill("itinerary");
  await expect(page.locator(".guide-tab")).toHaveCount(1);
  expect(await tabs(page)).toEqual(["Reply later and Set aside"]);
  expect(await railGroups(page)).toEqual(["Triage"]);

  // A word nothing says: no rail left to walk.
  await search(page).fill("kryptonite");
  await expect(page.locator(".guide-tab")).toHaveCount(0);
  // The article that was up stays up: narrowing the rail is not closing the page.
  await expect(page.locator(".guide-title")).toHaveText("What is different here");

  await search(page).fill("");
  await expect(page.locator(".guide-tab")).toHaveCount(whole);
});

test("the search is the first thing on the panel and already has the keyboard", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openGuide(page);

  // Above both columns and the width of the panel, rather than a field in the corner of the rail.
  const field = await box(page.locator(".guide-search"));
  const rail = await box(page.locator(".guide-rail"));
  const article = await box(page.locator(".guide-panel"));
  const guide = await box(page.locator(".guide"));
  expect(field.bottom).toBeLessThanOrEqual(rail.top);
  expect(field.bottom).toBeLessThanOrEqual(article.top);
  expect(Math.round(field.left)).toBe(Math.round(guide.left));
  expect(Math.round(field.width)).toBe(Math.round(guide.width));
  expect(field.width).toBeGreaterThan(rail.width);

  // Opened and typed into in one motion: nothing is pressed between the two.
  await expect(search(page)).toBeFocused();
  await page.keyboard.type("itinerary");
  await expect(search(page)).toHaveValue("itinerary");
  expect(await tabs(page)).toEqual(["Reply later and Set aside"]);

  // And the rail still walks with the keys once the field has given them up.
  await search(page).fill("");
  await page.locator(".guide-tab").first().click();
  const first = await page.locator(".guide-title").textContent();
  await page.keyboard.press("j");
  await expect(page.locator(".guide-title")).not.toHaveText(first!);
});

test("a search nothing answers offers to ask, with what was typed in the title", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openGuide(page);

  const nothing = page.locator(".guide-nothing");
  await expect(nothing).toHaveCount(0);

  await search(page).fill("kryptonite badger");
  await expect(page.locator(".guide-tab")).toHaveCount(0);
  await expect(nothing.locator(".empty-state")).toHaveText("Nothing here answers that");

  // What the press does is said before it is pressed: whose site, in what, and that it is public.
  const said = await nothing.locator(".guide-ask").innerText();
  expect(said).toContain("GitHub");
  expect(said).toContain("browser");
  expect(said).toContain("public");

  await heldOpen(page);
  await nothing.getByRole("button", { name: "Ask on GitHub" }).click();

  const sent = await asked(page);
  expect(sent).toHaveLength(1);
  const url = new URL(sent[0]);
  expect(`${url.origin}${url.pathname}`).toBe(
    "https://github.com/priyanshujain/margin-mail/issues/new",
  );
  expect(url.searchParams.get("labels")).toBe("question");
  expect(url.searchParams.get("title")).toBe("kryptonite badger");
});

test("a search that is answered does not offer to ask", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openGuide(page);

  await search(page).fill("itinerary");
  await expect(page.locator(".guide-tab")).toHaveCount(1);
  await expect(page.locator(".guide-nothing")).toHaveCount(0);

  await search(page).fill("");
  await expect(page.locator(".guide-nothing")).toHaveCount(0);
});

test("the close control and Escape both put back what was underneath", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);

  await page.keyboard.press("3");
  await expect(page.locator(".list-title")).toHaveText("Paper Trail");
  await openRow(page, 0);
  const subject = await page.locator(".thread-subject").textContent();

  await openGuide(page);
  await page.getByRole("button", { name: "Close" }).click();
  expect(await dialog(page)).toBeNull();
  // Not merely the place: the thread that was open is still open, at the scroll it was at.
  await expect(page.locator(".list-title")).toHaveText("Paper Trail");
  expect(await place(page)).toBe("paper-trail");
  await expect(page.locator(".thread-subject")).toHaveText(subject!);

  await openGuide(page);
  await page.keyboard.press("Escape");
  expect(await dialog(page)).toBeNull();
  expect(await place(page)).toBe("paper-trail");
  await expect(page.locator(".thread-subject")).toHaveText(subject!);
});

test("an article prints the key its verb answers to, whatever the keymap says it is", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openGuide(page);

  await page.locator(".guide-tab", { hasText: "Archive, trash and spam" }).click();
  await expect(page.locator(".guide-title")).toHaveText("Archive, trash and spam");

  // Read out of the binding table rather than typed here, and an unmodified verb on purpose: what
  // a modifier prints is glyphs the running platform chooses, and what a bare key prints is itself.
  const combo = keyFor("archive");
  expect(combo).not.toBeNull();
  expect(combo).not.toContain("+");
  // Every cap on the page, rather than whichever one happens to be first: what is asserted is that
  // the key the table answers with is the key the article prints, not where in the article it is.
  const caps = await page
    .locator(".guide-article .key")
    .evaluateAll((all) => all.map((one) => one.textContent ?? ""));
  expect(caps).toContain(combo!);

  // And the cap rides beside the name of the verb, so the sentence still reads on a phone, where
  // every keycap in the app hides itself.
  await expect(page.locator(".guide-p").first()).toContainText(
    "takes a thread out of the Inbox",
  );
});

test("the pictures are the ones that were captured, with an alt and a caption each", async ({
  page,
}) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openGuide(page);

  const found: string[] = [];
  const rail = page.locator(".guide-tab");
  for (let at = 0; at < (await rail.count()); at++) {
    await rail.nth(at).click();
    // The photographs only. A drawn figure is a figure too, and it has no file behind it to be
    // captured at twice the size or drawn at half of it.
    const shown = await page.locator(".guide-figure:not([data-drawn])").evaluateAll((figures) =>
      figures.map((figure) => {
        const img = figure.querySelector("img");
        return {
          src: img?.getAttribute("src") ?? "",
          alt: img?.getAttribute("alt") ?? "",
          caption: (figure.querySelector("figcaption")?.textContent ?? "").trim(),
          drawn: Math.round(img?.getBoundingClientRect().width ?? 0),
          room: Math.round(figure.getBoundingClientRect().width),
        };
      }),
    );
    for (const picture of shown) {
      expect(picture.alt.length).toBeGreaterThan(0);
      expect(picture.caption.length).toBeGreaterThan(0);
      found.push(picture.src);

      // A picture is drawn at the size it was taken at, and only the column narrows it. Anything
      // smaller than that is a screenshot of an app whose type has been shrunk out of legibility,
      // which is the whole reason it is captured at twice the size in the first place.
      const size = natural(picture.src.replace(/^\/guide\/|\.png$/g, ""));
      if (size) expect(picture.drawn, picture.src).toBe(Math.min(size.width, picture.room));
    }
  }

  // The src is what is asserted and never that a picture loaded: the files are captured by another
  // package and a suite that waited for them would be a suite that fails on a clean checkout.
  expect(found.sort()).toEqual(PICTURES.map((name) => `/guide/${name}.png`));
});

// The rule the whole guide is written to: a cap rides beside the name of the verb it belongs to,
// and the sentence has to survive the cap being taken away, because on a phone it is taken away.
// Read on the phone itself rather than by reading the source, since what is being asserted is what
// the paragraph says once the browser has finished with it.
test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("every sentence reads with the keycaps taken out of it", async ({ page }) => {
    await openApp(page, { now: MIDDAY() });
    await listReady(page);
    await openGuide(page);
    await expect(page.locator(".guide-article .key").first()).toBeHidden();

    const rail = page.locator(".guide-tab");
    const broken: string[] = [];
    for (let at = 0; at < (await rail.count()); at++) {
      await rail.nth(at).click();
      broken.push(
        ...(await page.locator(".guide-p, .guide-steps li").evaluateAll((all) =>
          all
            .map((el) => (el as HTMLElement).innerText)
            // A space in front of a full stop, or two of them in a row, is where a cap used to be.
            .filter((said) => /\s[.,;:?]/.test(said) || /\s\s/.test(said)),
        )),
      );
    }
    expect(broken).toEqual([]);
  });
});

test("a link in the guide goes where it says, and the guide closes behind it", async ({ page }) => {
  await openApp(page, { now: MIDDAY() });
  await listReady(page);
  await openGuide(page);

  // A cross-link stays in the guide and opens the article it names.
  await page.locator(".guide-tab", { hasText: "Where your mail goes" }).click();
  await page.locator(".guide-link", { hasText: "How the Screener works" }).click();
  await expect(page.locator(".guide-title")).toHaveText("How the Screener works");
  await expect(page.locator(".guide-tab[data-active]")).toHaveText("How the Screener works");

  // A link to a place in the app closes the guide behind it, because that is what it offered to do.
  await page.locator(".guide-tab", { hasText: "Clear the whole queue" }).click();
  await page.locator(".guide-link", { hasText: "Open the Screener" }).click();
  expect(await dialog(page)).toBeNull();
  expect(await place(page)).toBe("screener");
});
