import { defineConfig, devices } from "@playwright/test";

/**
 * Browser smoke tests for the Docker edition.
 *
 * The suite mocks `/api/v1/*` per test, so it needs no database or backend:
 * `pnpm test:e2e` starts the Vite dev server and exercises the real React
 * bundle. Set `E2E_BASE_URL` to point at an already-running stack (for example
 * `http://localhost:8080`) when you want the same flows against the real API.
 * Set `E2E_SERVER=preview` to exercise the production build instead of the
 * dev server (CI does this after `pnpm build`).
 */
const baseURL = process.env.E2E_BASE_URL ?? "http://127.0.0.1:4173";
const preview = process.env.E2E_SERVER === "preview";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL,
    locale: "ja-JP",
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: process.env.E2E_BASE_URL
    ? undefined
    : {
        // Bind and connect on the same address: `localhost` can resolve to
        // ::1 while Vite listens on 127.0.0.1 (or vice versa), which made the
        // readiness probe time out on some runners.
        command: `pnpm exec vite ${preview ? "preview " : ""}--host 127.0.0.1 --port 4173 --strictPort`,
        url: baseURL,
        reuseExistingServer: !process.env.CI,
        timeout: 120_000,
      },
});
