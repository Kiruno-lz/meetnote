import { test, expect, type Page } from "@playwright/test";

async function importMockMeeting(page: Page): Promise<void> {
  await page.addInitScript(() => {
    try {
      localStorage.clear();
    } catch {}
  });
  await page.goto("/");
  await page.locator("#btn-import-mock").click();
  await expect(page.locator("#btn-speaker-editor")).toBeVisible({ timeout: 10000 });
  await page.locator(".nav-btn").filter({ hasText: "会议项目" }).click();
  await expect(page.locator(".meeting-item").first()).toBeVisible({ timeout: 3000 });
}

test.describe("meeting management", () => {
  test("hover reveals edit and delete buttons", async ({ page }) => {
    await importMockMeeting(page);
    const card = page.locator(".meeting-item").first();
    // Hover over the card
    await card.hover();
    // Actions container transitions to opacity:1 on hover — check CSS rather than visibility
    // (Playwright toBeVisible does not check opacity; use toHaveCSS for the hover state)
    await expect(card.locator(".meeting-item-actions")).toHaveCSS("opacity", "1", { timeout: 2000 });
    // Buttons themselves are inside the now-visible actions container
    await expect(card.locator(".btn-edit")).toBeVisible({ timeout: 2000 });
    await expect(card.locator(".btn-delete")).toBeVisible({ timeout: 2000 });
  });

  test("edit title updates the meeting", async ({ page }) => {
    await importMockMeeting(page);
    const card = page.locator(".meeting-item").first();
    await card.hover();
    await card.locator(".btn-edit").click();

    // Inline editor should appear
    const input = card.locator(".edit-title-input");
    await expect(input).toBeVisible();
    await input.fill("新会议标题");
    await card.locator(".btn-confirm-edit").click();

    // Title should be updated
    await expect(page.locator(".meeting-item .meeting-title").first()).toContainText("新会议标题");
  });

  test("delete meeting removes it from list", async ({ page }) => {
    await importMockMeeting(page);
    const initialCount = await page.locator(".meeting-item").count();

    const card = page.locator(".meeting-item").first();
    await card.hover();

    // Register dialog handler before the click that triggers confirm()
    page.once("dialog", (dialog) => dialog.accept());
    await card.locator(".btn-delete").click();

    // Count should decrease by one
    await expect(page.locator(".meeting-item")).toHaveCount(initialCount - 1, { timeout: 3000 });
  });
});
