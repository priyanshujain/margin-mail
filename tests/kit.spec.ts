// The design system's own suite. Two things are worth asserting about a stylesheet: that the
// geometry the layout is built from is really what it claims, and that nothing has quietly written
// a colour instead of reaching for a token. Everything else about how it looks is a screenshot and
// a pair of eyes, which is why this file writes two of them.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";

const root = fileURLToPath(new URL("..", import.meta.url));
const shots = join(root, "screenshots");

/** A hex literal, and not an id selector: #root is three characters of which none is a hex digit. */
const HEX = /#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})\b/;

/** Back to the top with the layout finished, which is the state both pictures are taken in. */
async function settle(page: Page) {
  await page.evaluate(
    () =>
      new Promise<void>((done) => {
        window.scrollTo(0, 0);
        requestAnimationFrame(() => requestAnimationFrame(() => done()));
      }),
  );
}

function stylesheets(dir: string): string[] {
  return readdirSync(join(root, dir))
    .filter((f) => f.endsWith(".css"))
    .map((f) => join(dir, f));
}

function sources(dir: string): string[] {
  return readdirSync(join(root, dir))
    .filter((f) => f.endsWith(".tsx"))
    .map((f) => join(dir, f));
}

test("the kit renders in both palettes", async ({ page }) => {
  await page.goto("/#/kit");
  await expect(page.locator(".kit")).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  // The faces arrive after the first layout and everything below the fold moves when they do. A
  // popover measured against the old layout would be hanging off nothing in the picture.
  await settle(page);

  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.screenshot({ path: join(shots, "kit-light.png"), fullPage: true });

  await page.getByRole("tab", { name: "Dark" }).click();
  await settle(page);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.screenshot({ path: join(shots, "kit-dark.png"), fullPage: true });
});

test("the primitives with a fixed geometry have it", async ({ page }) => {
  await page.goto("/#/kit");
  await expect(page.locator(".kit")).toBeVisible();
  await page.evaluate(() => document.fonts.ready);

  // The list column. Nothing on this page is 420px wide, but every screen is built from the token
  // and a change to it would go unnoticed until a mockup stopped matching.
  const listWidth = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--list-w").trim(),
  );
  expect(listWidth).toBe("420px");

  const row = page.locator(".row").first();
  expect((await row.boundingBox())?.height).toBe(46);

  const avatar = row.locator(".avatar");
  const box = await avatar.boundingBox();
  expect(box?.width).toBe(30);
  expect(box?.height).toBe(30);

  // A row carrying a note grows for it rather than clipping it, so it is the one row that is not
  // 46px and the assertion above has to be reading a row without one.
  const noted = page.locator(".row[data-noted]").first();
  expect((await noted.boundingBox())?.height).toBeGreaterThan(46);
});

test("no stylesheet in the design system writes a colour", () => {
  const files = [...stylesheets("src/ui"), ...stylesheets("src/screens")];
  expect(files.length).toBeGreaterThan(10);

  const offenders: string[] = [];
  for (const file of files) {
    readFileSync(join(root, file), "utf8")
      .split("\n")
      .forEach((line, i) => {
        if (HEX.test(line)) offenders.push(`${file}:${i + 1}  ${line.trim()}`);
      });
  }
  expect(offenders, "a colour belongs in src/styles/mail.css as a token").toEqual([]);
});

test("every text field in the app tells the webview not to fill it in", () => {
  // WebKit treats an `<input type="text">` with no `autocomplete` as a form field, keeps what was
  // typed into one, and offers it back later as a pill with a cross on it. Under the palette that
  // reads as a suggestion the app is making, which is a promise the palette does not keep: every
  // row in it is a command that exists, and the browser's memory of last Tuesday is not one.
  //
  // Source rather than the rendered page, because the point is that the next input somebody adds
  // is covered too, and a field nobody wrote a spec for is exactly the one that will leak.
  const files = [...sources("src/ui"), ...sources("src/screens")];
  expect(files.length).toBeGreaterThan(10);

  const offenders: string[] = [];
  for (const file of files) {
    const text = readFileSync(join(root, file), "utf8");
    for (const tag of text.match(/<input\b[\s\S]*?\n\s*\/>/g) ?? []) {
      if (/NO_AUTOFILL|autoComplete=/.test(tag)) continue;
      offenders.push(`${file}  ${tag.split("\n")[0].trim()}`);
    }
  }
  expect(offenders, "spread NO_AUTOFILL from src/ui onto it").toEqual([]);
});

test("nothing in the app claims a keychain it does not use", () => {
  // src-tauri/src/google/secrets.rs is explicit that the OS credential store was removed on every
  // platform, and that what replaced it is an encrypted file in the app data directory whose
  // desktop protection amounts to "anyone who can read the home directory can read the token".
  // Three places in the interface said "keychain" anyway. In an app whose whole pitch is that your
  // mail and your decisions stay on your own machine, overstating where a password sits is the one
  // kind of copy that is worth a test.
  const files = [...sources("src/ui"), ...sources("src/screens")];
  const offenders = files.filter((file) =>
    /keychain/i.test(readFileSync(join(root, file), "utf8")),
  );
  expect(offenders, "say what secrets.rs actually does: encrypted on this device").toEqual([]);
});
