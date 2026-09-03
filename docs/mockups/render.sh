#!/usr/bin/env bash
# Renders every mockup in src/ to a PNG beside this script, at 2x. The fonts come from the
# sibling margin repo's shared package, so nothing binary is vendored here, and Playwright comes
# from the calendar repo's node_modules, where chromium is already installed.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
fonts="$here/../../../../python/margin/shared/fonts"
runner="$here/../../../../python/margin-caledar"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cp "$here"/src/*.html "$here"/src/*.css "$work"/
mkdir -p "$work/fonts"
cp "$fonts"/HankenGrotesk-VF.ttf "$fonts"/HankenGrotesk-Italic-VF.ttf \
   "$fonts"/Literata-VF.ttf "$fonts"/Literata-Italic-VF.ttf "$work/fonts/"

only="${1:-}"

NODE_PATH="$runner/node_modules" node - "$work" "$here" "$only" <<'EOF'
const { chromium } = require("@playwright/test");
const fs = require("fs");
const path = require("path");
const [work, out, only] = process.argv.slice(2);

(async () => {
  const browser = await chromium.launch();
  const files = fs.readdirSync(work).filter((f) => f.endsWith(".html"));
  for (const file of files) {
    const name = file.replace(/\.html$/, "");
    if (only && name !== only) continue;
    const phone = name.startsWith("phone-");
    const context = await browser.newContext({
      viewport: phone ? { width: 390, height: 844 } : { width: 1440, height: 900 },
      deviceScaleFactor: 2,
    });
    const page = await context.newPage();
    await page.goto("file://" + path.join(work, file));
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(300);
    await page.screenshot({ path: path.join(out, name + ".png") });
    await context.close();
    console.log("rendered " + name + ".png");
  }
  await browser.close();
})();
EOF
