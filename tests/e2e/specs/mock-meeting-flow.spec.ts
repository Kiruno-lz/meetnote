import { test, expect } from "@playwright/test";

test("mock meeting end-to-end flow", async ({ page }) => {
  // Clear localStorage before first navigation to ensure clean state
  await page.addInitScript(() => {
    try { localStorage.clear(); } catch {}
  });

  // Step 1: Initial page load — verify brand
  await page.goto("/");
  await expect(page.locator(".nav-brand")).toHaveText("meetnote");

  // Step 2: Import mock audio — should navigate to transcription-progress
  await page.locator("#btn-import-mock").click();
  await expect(page.locator(".progress-bar-container")).toBeVisible();

  // Step 3: Wait for progress to complete and auto-navigate to recording-detail
  // 4 stages × 600ms = ~2400ms; wait up to 10s for the recording-detail to appear
  await expect(page.locator("#btn-speaker-editor")).toBeVisible({ timeout: 10000 });

  // Step 4: Verify recording-detail content
  await expect(page.locator(".transcript-segment").first()).toBeVisible();
  await expect(page.locator(".speaker-tag").first()).toBeVisible();

  // Step 5: Enter speaker editor
  await page.locator("#btn-speaker-editor").click();
  await expect(page.locator("#btn-confirm")).toBeVisible();

  // Step 6: Rename speaker-0 to 张三
  const renameInput = page.locator('.speaker-rename-input[data-id="speaker-0"]');
  await renameInput.fill("张三");
  await page.locator('.btn-rename[data-id="speaker-0"]').click();

  // After rename, the list rerenders — verify the tag shows new name
  await expect(page.locator(".speaker-tag").first()).toContainText("张三");

  // Step 7: Confirm corrections — should return to recording-detail
  await page.locator("#btn-confirm").click();
  await expect(page.locator("#btn-speaker-editor")).toBeVisible();
  await expect(page.locator("#btn-deverb")).toBeVisible();

  // Step 8: Enter deverbalization preview
  await page.locator("#btn-deverb").click();
  await expect(page.locator("#btn-save-markdown")).toBeVisible();

  // Step 9: Save to markdown library — navigates to markdown-library
  await page.locator("#btn-save-markdown").click();

  // Step 10: Verify markdown library shows the saved meeting
  await expect(page.locator(".meeting-library-item").first()).toBeVisible();
  await expect(
    page.locator(".meeting-library-item .meeting-title").first()
  ).toContainText("演示会议");
});
