# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: mock-meeting-flow.spec.ts >> mock meeting end-to-end flow
- Location: specs/mock-meeting-flow.spec.ts:3:5

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: locator('.meeting-library-item').first()
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for locator('.meeting-library-item').first()

```

```yaml
- main:
  - navigation:
    - text: meetnote
    - button "会议项目"
    - button "Markdown 库"
    - button "模型管理"
    - button "设置": ⚙️
  - heading "Markdown 库" [level=2]
  - text: 演示会议 2026/6/2 19:05:16 2024/01/01
  - button "重命名":
    - img
  - button "删除":
    - img
  - button "返回首页"
```

# Test source

```ts
  1  | import { test, expect } from "@playwright/test";
  2  | 
  3  | test("mock meeting end-to-end flow", async ({ page }) => {
  4  |   // Clear localStorage before first navigation to ensure clean state
  5  |   await page.addInitScript(() => {
  6  |     try { localStorage.clear(); } catch {}
  7  |   });
  8  | 
  9  |   // Step 1: Initial page load — verify brand
  10 |   await page.goto("/");
  11 |   await expect(page.locator(".nav-brand")).toHaveText("meetnote");
  12 | 
  13 |   // Step 2: Import mock audio — should navigate to transcription-progress
  14 |   await page.locator("#btn-import-mock").click();
  15 |   await expect(page.locator(".progress-bar-container")).toBeVisible();
  16 | 
  17 |   // Step 3: Wait for progress to complete and auto-navigate to recording-detail
  18 |   // 4 stages × 600ms = ~2400ms; wait up to 10s for the recording-detail to appear
  19 |   await expect(page.locator("#btn-speaker-editor")).toBeVisible({ timeout: 10000 });
  20 | 
  21 |   // Step 4: Verify recording-detail content
  22 |   await expect(page.locator(".transcript-segment").first()).toBeVisible();
  23 |   await expect(page.locator(".speaker-tag").first()).toBeVisible();
  24 | 
  25 |   // Step 5: Enter speaker editor
  26 |   await page.locator("#btn-speaker-editor").click();
  27 |   await expect(page.locator("#btn-confirm")).toBeVisible();
  28 | 
  29 |   // Step 6: Rename speaker-0 to 张三
  30 |   const renameInput = page.locator('.speaker-rename-input[data-id="speaker-0"]');
  31 |   await renameInput.fill("张三");
  32 |   await page.locator('.btn-rename[data-id="speaker-0"]').click();
  33 | 
  34 |   // After rename, the list rerenders — verify the tag shows new name
  35 |   await expect(page.locator(".speaker-tag").first()).toContainText("张三");
  36 | 
  37 |   // Step 7: Confirm corrections — should return to recording-detail
  38 |   await page.locator("#btn-confirm").click();
  39 |   await expect(page.locator("#btn-speaker-editor")).toBeVisible();
  40 |   await expect(page.locator("#btn-deverb")).toBeVisible();
  41 | 
  42 |   // Step 8: Enter deverbalization preview
  43 |   await page.locator("#btn-deverb").click();
  44 |   await expect(page.locator("#btn-save-markdown")).toBeVisible();
  45 | 
  46 |   // Step 9: Save to markdown library — navigates to markdown-library
  47 |   await page.locator("#btn-save-markdown").click();
  48 | 
  49 |   // Step 10: Verify markdown library shows the saved meeting (click navigates to preview)
> 50 |   await expect(page.locator(".meeting-library-item").first()).toBeVisible();
     |                                                               ^ Error: expect(locator).toBeVisible() failed
  51 |   await expect(
  52 |     page.locator(".meeting-library-item .meeting-title").first()
  53 |   ).toContainText("演示会议");
  54 | 
  55 |   // Step 11: Click meeting in library → navigates to deverbalization-preview
  56 |   await page.locator(".meeting-library-item").first().click();
  57 |   await expect(page.locator(".deverb-right-content")).toBeVisible({ timeout: 3000 });
  58 | 
  59 |   // Step 12: Back button from library-opened preview returns to markdown-library
  60 |   await page.locator("#btn-back").click();
  61 |   await expect(page.locator(".meeting-library-item").first()).toBeVisible();
  62 | });
  63 | 
```