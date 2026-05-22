import { test, expect } from "@playwright/test";
import * as path from "path";

test.describe("audio file import", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      try {
        localStorage.clear();
      } catch {}
    });
    await page.goto("/");
  });

  test("valid WAV file import triggers progress flow", async ({ page }) => {
    const wavPath = path.join(__dirname, "../fixtures/test-audio.wav");

    // Trigger file input via setInputFiles (bypasses native dialog)
    const fileInput = page.locator("#audio-file-input");
    await fileInput.setInputFiles(wavPath);

    // Should navigate to transcription-progress
    await expect(page.locator(".progress-bar-container")).toBeVisible({ timeout: 5000 });
    // After progress completes, should navigate to recording-detail
    await expect(page.locator("#btn-speaker-editor")).toBeVisible({ timeout: 10000 });
  });

  test("invalid file extension shows error message", async ({ page }) => {
    // Simulate a .txt file — extension is not supported
    await page.locator("#audio-file-input").setInputFiles({
      name: "document.txt",
      mimeType: "text/plain",
      buffer: Buffer.from("hello world"),
    });

    // Should show error message in #import-error
    const errorDiv = page.locator("#import-error");
    await expect(errorDiv).toContainText(/.+/, { timeout: 3000 });
  });
});
