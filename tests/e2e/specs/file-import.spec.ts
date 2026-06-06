import { test, expect } from "@playwright/test";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";

/**
 * Generate a minimal valid WAV file (44-byte header + 1764 bytes of silence).
 * This avoids git's binary file tracking issues while still producing a file
 * the app can read via the File API for hash + duration extraction.
 */
function generateMinimalWav(): Buffer {
  const sampleRate = 16000;
  const numChannels = 1;
  const bitsPerSample = 16;
  const numSamples = sampleRate * 1; // 1 second of silence
  const dataSize = numSamples * numChannels * (bitsPerSample / 8);
  const header = Buffer.alloc(44);

  let offset = 0;
  // RIFF header
  header.write("RIFF", offset); offset += 4;
  header.writeUInt32LE(36 + dataSize, offset); offset += 4;
  header.write("WAVE", offset); offset += 4;
  // fmt subchunk
  header.write("fmt ", offset); offset += 4;
  header.writeUInt32LE(16, offset); offset += 4;          // subchunk size
  header.writeUInt16LE(1, offset); offset += 2;           // PCM format
  header.writeUInt16LE(numChannels, offset); offset += 2;
  header.writeUInt32LE(sampleRate, offset); offset += 4;
  header.writeUInt32LE(sampleRate * numChannels * (bitsPerSample / 8), offset); offset += 4;
  header.writeUInt16LE(numChannels * (bitsPerSample / 8), offset); offset += 2;
  header.writeUInt16LE(bitsPerSample, offset); offset += 2;
  // data subchunk
  header.write("data", offset); offset += 4;
  header.writeUInt32LE(dataSize, offset); offset += 4;

  // Silence data (all zeros)
  const data = Buffer.alloc(dataSize, 0);
  return Buffer.concat([header, data]);
}

test.describe("audio file import", () => {
  let tmpWavPath: string;

  test.beforeEach(async ({ page }) => {
    tmpWavPath = path.join(os.tmpdir(), `meetnote-test-${Date.now()}.wav`);
    fs.writeFileSync(tmpWavPath, generateMinimalWav());

    await page.addInitScript(() => {
      try { localStorage.clear(); } catch {}
    });
    await page.goto("/");
  });

  test.afterEach(() => {
    if (tmpWavPath && fs.existsSync(tmpWavPath)) {
      fs.unlinkSync(tmpWavPath);
    }
  });

  test("valid WAV file import triggers progress flow", async ({ page }) => {
    // Trigger file input via setInputFiles (bypasses native dialog)
    const fileInput = page.locator("#audio-file-input");
    await fileInput.setInputFiles(tmpWavPath);

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
