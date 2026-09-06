// The end-to-end suite drives the real UI in a browser, where `src/ipc.ts` routes every command to
// the dev fixture. One command starts everything: `pnpm test:ui` brings up Vite itself and reuses a
// dev server that is already running.
//
// The viewport is fixed at the size the mockups were rendered at, because half of what these tests
// assert is geometry: the 420px list column, the row height, whether the piles are on screen. A
// test that measured a window of unknown size would be asserting nothing.

import { defineConfig } from "@playwright/test";

const PORT = 1450;
const BASE_URL = `http://localhost:${PORT}`;

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  // No retries on purpose. A test that only passes on the second go is a test that is lying about
  // something, and this suite exists because things that looked fine were not.
  retries: 0,
  reporter: [["list"]],
  outputDir: "node_modules/.cache/playwright",
  timeout: 30_000,
  expect: { timeout: 5_000 },

  use: {
    baseURL: BASE_URL,
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
    // The fixture is anchored to the browser's local day, so pinning the zone keeps a run on one
    // machine comparable with a run on another. Nothing in the suite depends on the zone itself.
    timezoneId: "Asia/Kolkata",
    locale: "en-GB",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },

  // Deliberately not `devices["Desktop Chrome"]`: that device pins a Windows user agent, and the
  // keymap reads the platform off the user agent to decide whether the primary modifier is Command
  // or Control. A faked platform would test the wrong half of every shortcut.
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],

  webServer: {
    command: "pnpm dev",
    url: BASE_URL,
    reuseExistingServer: true,
    timeout: 60_000,
    stdout: "ignore",
    stderr: "pipe",
  },
});
