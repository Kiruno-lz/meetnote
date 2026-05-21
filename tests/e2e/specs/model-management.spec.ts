import { test, expect } from "@playwright/test";

test.describe("model management", () => {
  test("navigate to model management shows Whisper model list", async ({ page }) => {
    await page.addInitScript(() => {
      try {
        localStorage.clear();
      } catch {}
    });
    await page.goto("/");

    // Click model management nav button
    await page.locator(".nav-btn").filter({ hasText: "模型管理" }).click();

    // Should show the section heading
    await expect(page.locator("h2").filter({ hasText: "模型管理" })).toBeVisible();

    // Should show at least one model card (mock data in non-Tauri mode)
    await expect(page.locator(".card").first()).toBeVisible({ timeout: 3000 });

    // The app element itself should not be empty
    await expect(page.locator("#app")).not.toBeEmpty();
  });
});
