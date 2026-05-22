import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./specs",
  use: {
    baseURL: "http://localhost:1420",
    headless: true,
  },
  webServer: {
    command: "cd ../../frontend && bun run dev",
    url: "http://localhost:1420",
    reuseExistingServer: true,
    timeout: 30000,
  },
  timeout: 30000,
  retries: 0,
});
