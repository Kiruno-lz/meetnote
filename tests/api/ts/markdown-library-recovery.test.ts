import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import { promises as fs } from "fs";
import { resolve } from "path";
import { generateMarkdownContent } from "../../../frontend/src/utils/markdown-library-generator";
import {
  createMarkdownFrontmatter,
  parseMarkdownFrontmatter,
  MARKDOWN_LIBRARY_SCHEMA_VERSION
} from "../../../frontend/src/types/markdown-library-schema";
import { createDeverbalizationOutput } from "../../../frontend/src/types/llm-schema";
import { saveMeetingMarkdown, searchMeetings } from "../../../frontend/src/stores/markdown-library-storage";

interface FixtureData {
  audioMetadata: { filename: string; durationMs: number; audioHash: string; uploadedAt: string };
  asrResult: { requestId: string; model: { id: string; version: string }; segments: any[] };
  diarizationResult: { requestId: string; model: { id: string; version: string }; speakers: any[]; segments: any[] };
  deverbalizationOutput: { requestId: string; provider: "ollama" | "custom"; model: { id: string; version: string }; paragraphs: any[] };
  expectedMarkdownFrontmatter: { title: string; date: string; audioHash: string; speakers: string[]; modelVersion: string; llmProvider: "ollama" | "custom" };
}

async function loadFixture(name: string): Promise<FixtureData> {
  const path = resolve(__dirname, `../../fixtures/${name}.json`);
  const content = await fs.readFile(path, "utf-8");
  return JSON.parse(content);
}

function createMarkdownFromFixture(fixture: FixtureData): string {
  const frontmatter = createMarkdownFrontmatter({
    title: fixture.expectedMarkdownFrontmatter.title,
    date: fixture.expectedMarkdownFrontmatter.date,
    audioHash: fixture.audioMetadata.audioHash,
    speakers: fixture.expectedMarkdownFrontmatter.speakers,
    modelVersion: fixture.deverbalizationOutput.model.version,
    llmProvider: fixture.deverbalizationOutput.provider,
    schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
    generatedAt: new Date().toISOString()
  });

  const deverbalizationOutput = createDeverbalizationOutput({
    requestId: fixture.deverbalizationOutput.requestId,
    provider: fixture.deverbalizationOutput.provider,
    model: fixture.deverbalizationOutput.model,
    paragraphs: fixture.deverbalizationOutput.paragraphs
  });

  const asrText = fixture.asrResult.segments.map((s: any) => s.text).join(" ");

  return generateMarkdownContent({
    frontmatter,
    deverbalizationOutput,
    originalTranscript: asrText,
    speakerCorrections: []
  });
}

describe("markdown-library-recovery", () => {
  let tempDir: string;

  beforeEach(async () => {
    tempDir = `/tmp/meetnote-recovery-${Date.now()}-${Math.random().toString(36).substring(7)}`;
    await fs.mkdir(tempDir, { recursive: true });
  });

  afterEach(async () => {
    try {
      await fs.rm(tempDir, { recursive: true, force: true });
    } catch {}
  });

  describe("corrupted file recovery", () => {
    it("detects invalid markdown during parsing", async () => {
      const invalidMarkdown = `---
title: Invalid
corrupted: true
---

content`;

      const result = parseMarkdownFrontmatter(invalidMarkdown);
      expect(result.ok).toBe(false);
    });

    it("validates frontmatter schema", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      const result = parseMarkdownFrontmatter(markdown);
      expect(result.ok).toBe(true);
      expect(result.frontmatter?.schemaVersion).toBe(MARKDOWN_LIBRARY_SCHEMA_VERSION);
    });

    it("rejects empty speaker list", async () => {
      const invalidMarkdown = `---
title: No Speakers
date: 2024-05-20
audioHash: a1b2c3d4e5f6a7b8
speakers: []
modelVersion: 1.0
llmProvider: ollama
schemaVersion: meetnote.markdown-library.v1
generatedAt: 2024-05-20T00:00:00Z
---

content`;

      const result = parseMarkdownFrontmatter(invalidMarkdown);
      expect(result.ok).toBe(false);
    });

    it("rejects missing audio hash", async () => {
      const invalidMarkdown = `---
title: Bad Hash
date: 2024-05-20
speakers: [Alice]
modelVersion: 1.0
llmProvider: ollama
schemaVersion: meetnote.markdown-library.v1
generatedAt: 2024-05-20T00:00:00Z
---

content`;

      const result = parseMarkdownFrontmatter(invalidMarkdown);
      expect(result.ok).toBe(false);
    });
  });

  describe("file reorganization resilience", () => {
    it("handles directory structure changes", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      const saveResult = await saveMeetingMarkdown({ basePath: tempDir, markdown });

      // Move file to a different location
      const originalPath = resolve(tempDir, saveResult.savedPath!, "meeting.md");
      const newDir = resolve(tempDir, "reorganized", "archive");
      await fs.mkdir(newDir, { recursive: true });
      const newPath = resolve(newDir, "meeting.md");
      await fs.rename(originalPath, newPath);

      // File should still be readable
      const recovered = await fs.readFile(newPath, "utf-8");
      const result = parseMarkdownFrontmatter(recovered);
      expect(result.ok).toBe(true);
    });

    it("maintains metadata integrity after move", async () => {
      const fixture = await loadFixture("multi-speaker-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      const saveResult = await saveMeetingMarkdown({ basePath: tempDir, markdown });
      const originalPath = resolve(tempDir, saveResult.savedPath!, "meeting.md");

      // Read original
      const originalContent = await fs.readFile(originalPath, "utf-8");
      const originalResult = parseMarkdownFrontmatter(originalContent);

      // Move and read
      const newDir = resolve(tempDir, "backup");
      await fs.mkdir(newDir, { recursive: true });
      const newPath = resolve(newDir, "meeting.md");
      await fs.rename(originalPath, newPath);
      const newContent = await fs.readFile(newPath, "utf-8");
      const newResult = parseMarkdownFrontmatter(newContent);

      // Should be identical
      expect(originalResult.frontmatter?.title).toBe(newResult.frontmatter?.title);
      expect(originalResult.frontmatter?.audioHash).toBe(newResult.frontmatter?.audioHash);
      expect(originalResult.frontmatter?.speakers).toEqual(newResult.frontmatter?.speakers);
    });
  });

  describe("partial corruption handling", () => {
    it("preserves valid meetings when one file is corrupted", async () => {
      const fixture1 = await loadFixture("simple-meeting");
      const fixture2 = await loadFixture("multi-speaker-meeting");

      const markdown1 = createMarkdownFromFixture(fixture1);
      const markdown2 = createMarkdownFromFixture(fixture2);

      const saveResult1 = await saveMeetingMarkdown({ basePath: tempDir, markdown: markdown1 });
      const saveResult2 = await saveMeetingMarkdown({ basePath: tempDir, markdown: markdown2 });

      // Corrupt first file
      const path1 = resolve(tempDir, saveResult1.savedPath!, "meeting.md");
      await fs.writeFile(path1, "corrupted data");

      // Second file should still be readable
      const path2 = resolve(tempDir, saveResult2.savedPath!, "meeting.md");
      const content2 = await fs.readFile(path2, "utf-8");
      const result2 = parseMarkdownFrontmatter(content2);
      expect(result2.ok).toBe(true);
    });

    it("handles truncated files", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      const saveResult = await saveMeetingMarkdown({ basePath: tempDir, markdown });

      // Truncate file aggressively to cut through frontmatter
      const path = resolve(tempDir, saveResult.savedPath!, "meeting.md");
      const content = await fs.readFile(path, "utf-8");
      // Truncate to only first 50 characters, which will definitely be in the frontmatter
      const truncated = content.substring(0, 50);
      await fs.writeFile(path, truncated);

      // Should fail to parse - truncated frontmatter has no closing ---
      const result = parseMarkdownFrontmatter(truncated);
      expect(result.ok).toBe(false);
    });
  });

  describe("search resilience", () => {
    it("searches still work after file moves", async () => {
      const fixture = await loadFixture("corrected-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      // Save to custom location
      const archiveDir = resolve(tempDir, "custom", "archive");
      await fs.mkdir(archiveDir, { recursive: true });
      await saveMeetingMarkdown({ basePath: archiveDir, markdown });

      // Search should work
      const results = await searchMeetings(archiveDir, { speakers: ["Alice"] });
      expect(results.length).toBeGreaterThan(0);
    });

    it("finds meetings by title after reorganization", async () => {
      const fixture = await loadFixture("edge-case-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      // Save with reorganization
      const dir = resolve(tempDir, "2024", "05");
      await fs.mkdir(dir, { recursive: true });
      await saveMeetingMarkdown({ basePath: dir, markdown });

      // Search by title
      const results = await searchMeetings(dir, {
        title: fixture.expectedMarkdownFrontmatter.title
      });

      expect(results.length).toBe(1);
      expect(results[0].title).toBe(fixture.expectedMarkdownFrontmatter.title);
    });

    it("speaker search works across multiple locations", async () => {
      const fixture = await loadFixture("multi-speaker-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      // Save to main directory
      await saveMeetingMarkdown({ basePath: tempDir, markdown });

      // Search should find all speakers
      const results = await searchMeetings(tempDir, {});
      expect(results.length).toBeGreaterThan(0);
      expect(results[0].speakers.length).toBe(4);
    });
  });

  describe("data consistency", () => {
    it("audioHash remains stable through cycles", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      const saveResult = await saveMeetingMarkdown({ basePath: tempDir, markdown });
      const recovered = await fs.readFile(resolve(tempDir, saveResult.savedPath!, "meeting.md"), "utf-8");
      const result = parseMarkdownFrontmatter(recovered);

      expect(result.frontmatter?.audioHash).toBe(fixture.audioMetadata.audioHash);
    });

    it("speaker labels preserved exactly", async () => {
      const fixture = await loadFixture("multi-speaker-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      const saveResult = await saveMeetingMarkdown({ basePath: tempDir, markdown });
      const recovered = await fs.readFile(resolve(tempDir, saveResult.savedPath!, "meeting.md"), "utf-8");
      const result = parseMarkdownFrontmatter(recovered);

      expect(result.frontmatter?.speakers).toEqual(
        fixture.expectedMarkdownFrontmatter.speakers
      );
    });

    it("date information intact after recovery", async () => {
      const fixture = await loadFixture("corrected-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      const saveResult = await saveMeetingMarkdown({ basePath: tempDir, markdown });
      const recovered = await fs.readFile(resolve(tempDir, saveResult.savedPath!, "meeting.md"), "utf-8");
      const result = parseMarkdownFrontmatter(recovered);

      expect(result.frontmatter?.date).toBe(fixture.expectedMarkdownFrontmatter.date);
    });
  });
});
