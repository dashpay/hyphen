import { defineConfig, devices } from "playwright/test";

export default defineConfig({
  testDir: "./tests",
  timeout: 60_000,
  fullyParallel: true,
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: "pnpm gen && svelte-kit sync && pnpm exec vite dev --host 127.0.0.1 --port 4173 --strictPort",
    port: 4173,
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
