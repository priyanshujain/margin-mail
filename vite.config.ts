import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// Ports are offset from margin's 1420/1421, the calendar's 1430/1431 and the editor's 1440/1441, so
// the four apps can run side by side. The Playwright config reuses whatever answers on the port, so
// a port shared with a sibling is a suite that silently drives the wrong app.
export default defineConfig({
  plugins: [react()],

  clearScreen: false,
  server: {
    port: 1450,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1451,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
});
