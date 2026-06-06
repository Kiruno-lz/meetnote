# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: file-import.spec.ts >> audio file import >> valid WAV file import triggers progress flow
- Location: specs/file-import.spec.ts:14:7

# Error details

```
Error: ENOENT: no such file or directory, stat '/Users/kiruno/Documents/_code/vibe/meetnote/.claude/worktrees/add-asr-models/tests/e2e/fixtures/test-audio.wav'
```

# Page snapshot

```yaml
- main [ref=e2]:
  - generic [ref=e3]:
    - navigation [ref=e4]:
      - generic [ref=e5]: meetnote
      - generic [ref=e6]:
        - button "会议项目" [ref=e7] [cursor=pointer]
        - button "Markdown 库" [ref=e8] [cursor=pointer]
        - button "模型管理" [ref=e9] [cursor=pointer]
      - button "设置" [ref=e10] [cursor=pointer]: ⚙️
    - generic [ref=e12]:
      - heading "会议项目" [level=2] [ref=e14]
      - generic [ref=e16]:
        - text: 暂无会议记录
        - text: 点击下方按钮导入演示数据
      - generic [ref=e18]:
        - button "导入音频文件" [ref=e19] [cursor=pointer]
        - button "导入 Mock 音频" [ref=e20] [cursor=pointer]
```

# Test source

```ts
  1  | import { test, expect } from "@playwright/test";
  2  | import * as path from "path";
  3  | 
  4  | test.describe("audio file import", () => {
  5  |   test.beforeEach(async ({ page }) => {
  6  |     await page.addInitScript(() => {
  7  |       try {
  8  |         localStorage.clear();
  9  |       } catch {}
  10 |     });
  11 |     await page.goto("/");
  12 |   });
  13 | 
  14 |   test("valid WAV file import triggers progress flow", async ({ page }) => {
  15 |     const wavPath = path.join(__dirname, "../fixtures/test-audio.wav");
  16 | 
  17 |     // Trigger file input via setInputFiles (bypasses native dialog)
  18 |     const fileInput = page.locator("#audio-file-input");
> 19 |     await fileInput.setInputFiles(wavPath);
     |     ^ Error: ENOENT: no such file or directory, stat '/Users/kiruno/Documents/_code/vibe/meetnote/.claude/worktrees/add-asr-models/tests/e2e/fixtures/test-audio.wav'
  20 | 
  21 |     // Should navigate to transcription-progress
  22 |     await expect(page.locator(".progress-bar-container")).toBeVisible({ timeout: 5000 });
  23 |     // After progress completes, should navigate to recording-detail
  24 |     await expect(page.locator("#btn-speaker-editor")).toBeVisible({ timeout: 10000 });
  25 |   });
  26 | 
  27 |   test("invalid file extension shows error message", async ({ page }) => {
  28 |     // Simulate a .txt file — extension is not supported
  29 |     await page.locator("#audio-file-input").setInputFiles({
  30 |       name: "document.txt",
  31 |       mimeType: "text/plain",
  32 |       buffer: Buffer.from("hello world"),
  33 |     });
  34 | 
  35 |     // Should show error message in #import-error
  36 |     const errorDiv = page.locator("#import-error");
  37 |     await expect(errorDiv).toContainText(/.+/, { timeout: 3000 });
  38 |   });
  39 | });
  40 | 
```