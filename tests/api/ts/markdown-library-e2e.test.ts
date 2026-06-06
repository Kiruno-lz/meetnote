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

describe("markdown-library-e2e", () => {
  let tempDir: string;

  beforeEach(async () => {
    tempDir = `/tmp/meetnote-e2e-${Date.now()}-${Math.random().toString(36).substring(7)}`;
    await fs.mkdir(tempDir, { recursive: true });
  });

  afterEach(async () => {
    try {
      await fs.rm(tempDir, { recursive: true, force: true });
    } catch {}
  });

  describe("simple-meeting", () => {
    it("generates valid markdown with single speaker", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      expect(markdown).toContain("---");
      expect(markdown).toContain(`title: ${fixture.expectedMarkdownFrontmatter.title}`);
      expect(markdown).toContain("**Alice**:");
    });

    it("markdown frontmatter parses correctly", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);
      const result = parseMarkdownFrontmatter(markdown);

      expect(result.ok).toBe(true);
      expect(result.frontmatter?.title).toBe(fixture.expectedMarkdownFrontmatter.title);
      expect(result.frontmatter?.speakers).toEqual(["Alice"]);
      expect(result.frontmatter?.audioHash).toBe(fixture.audioMetadata.audioHash);
    });

    it("saves and retrieves from filesystem", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      const saveResult = await saveMeetingMarkdown({ basePath: tempDir, markdown });

      expect(saveResult.success).toBe(true);
      expect(saveResult.savedPath).toBeDefined();

      // savedPath is relative, meeting.md is at the end of the path
      const filePath = resolve(tempDir, saveResult.savedPath!, "meeting.md");
      const fileContent = await fs.readFile(filePath, "utf-8");
      expect(fileContent).toBe(markdown);
    });

    it("search by title works", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      await saveMeetingMarkdown({ basePath: tempDir, markdown });
      const results = await searchMeetings(tempDir, { title: fixture.expectedMarkdownFrontmatter.title });

      expect(results.length).toBe(1);
      expect(results[0].title).toBe(fixture.expectedMarkdownFrontmatter.title);
    });

    it("search by speaker works", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      await saveMeetingMarkdown({ basePath: tempDir, markdown });
      const results = await searchMeetings(tempDir, { speakers: ["Alice"] });

      expect(results.length).toBe(1);
      expect(results[0].speakers).toContain("Alice");
    });

    it("search by date range works", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      await saveMeetingMarkdown({ basePath: tempDir, markdown });
      const results = await searchMeetings(tempDir, {
        dateRange: { startDate: "2024-05-01", endDate: "2024-05-31" }
      });

      expect(results.length).toBeGreaterThan(0);
    });
  });

  describe("multi-speaker-meeting", () => {
    it("generates markdown with 4 speakers", async () => {
      const fixture = await loadFixture("multi-speaker-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      expect(markdown).toContain("**Alice**:");
      expect(markdown).toContain("**Bob**:");
      expect(markdown).toContain("**Charlie**:");
      expect(markdown).toContain("**Diana**:");
    });

    it("preserves all speakers in frontmatter", async () => {
      const fixture = await loadFixture("multi-speaker-meeting");
      const markdown = createMarkdownFromFixture(fixture);
      const result = parseMarkdownFrontmatter(markdown);

      expect(result.frontmatter?.speakers).toEqual(["Alice", "Bob", "Charlie", "Diana"]);
    });

    it("saves multi-speaker meeting", async () => {
      const fixture = await loadFixture("multi-speaker-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      const saveResult = await saveMeetingMarkdown({ basePath: tempDir, markdown });
      expect(saveResult.success).toBe(true);
      expect(saveResult.indexEntry?.speakers.length).toBe(4);
    });
  });

  describe("mixed-language-meeting", () => {
    it("handles Chinese and English mixed content", async () => {
      const fixture = await loadFixture("mixed-language-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      expect(markdown).toContain("API");
      expect(markdown).toContain("讨论");
    });

    it("preserves mixed language accuracy", async () => {
      const fixture = await loadFixture("mixed-language-meeting");
      const markdown = createMarkdownFromFixture(fixture);
      const result = parseMarkdownFrontmatter(markdown);

      expect(result.ok).toBe(true);
    });
  });

  describe("corrected-meeting", () => {
    it("applies corrected speaker names", async () => {
      const fixture = await loadFixture("corrected-meeting");
      const markdown = createMarkdownFromFixture(fixture);
      const result = parseMarkdownFrontmatter(markdown);

      expect(result.frontmatter?.speakers).toContain("Alice");
      expect(result.frontmatter?.speakers).toContain("Bob");
      expect(result.frontmatter?.speakers).toContain("Charlie");
    });
  });

  describe("edge-case-meeting", () => {
    it("handles special characters in title", async () => {
      const fixture = await loadFixture("edge-case-meeting");
      const markdown = createMarkdownFromFixture(fixture);
      const result = parseMarkdownFrontmatter(markdown);

      expect(result.ok).toBe(true);
      expect(result.frontmatter?.title).toBeDefined();
    });

    it("preserves URLs in content", async () => {
      const fixture = await loadFixture("edge-case-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      expect(markdown).toContain("https://");
    });

    it("handles unicode characters", async () => {
      const fixture = await loadFixture("edge-case-meeting");
      const markdown = createMarkdownFromFixture(fixture);
      const result = parseMarkdownFrontmatter(markdown);

      expect(result.ok).toBe(true);
    });

    it("preserves special characters", async () => {
      const fixture = await loadFixture("edge-case-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      expect(markdown).toContain("[");
      expect(markdown).toContain("{");
    });
  });

  describe("data integrity", () => {
    it("recovered meeting matches original", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      const saveResult = await saveMeetingMarkdown({ basePath: tempDir, markdown });
      const filePath = resolve(tempDir, saveResult.savedPath!, "meeting.md");
      const recovered = await fs.readFile(filePath, "utf-8");
      const result = parseMarkdownFrontmatter(recovered);

      expect(result.frontmatter?.title).toBe(fixture.expectedMarkdownFrontmatter.title);
      expect(result.frontmatter?.speakers).toEqual(fixture.expectedMarkdownFrontmatter.speakers);
    });

    it("audioHash preserved through save/load cycle", async () => {
      const fixture = await loadFixture("simple-meeting");
      const markdown = createMarkdownFromFixture(fixture);

      await saveMeetingMarkdown({ basePath: tempDir, markdown });
      const results = await searchMeetings(tempDir, {});

      expect(results[0].audioHash).toBe(fixture.audioMetadata.audioHash);
    });

    it("multiple meetings have distinct metadata", async () => {
      const fixture1 = await loadFixture("simple-meeting");
      const fixture2 = await loadFixture("multi-speaker-meeting");

      const markdown1 = createMarkdownFromFixture(fixture1);
      const markdown2 = createMarkdownFromFixture(fixture2);

      await saveMeetingMarkdown({ basePath: tempDir, markdown: markdown1 });
      await saveMeetingMarkdown({ basePath: tempDir, markdown: markdown2 });

      const results = await searchMeetings(tempDir, {});
      expect(results.length).toBe(2);
      expect(results[0].speakers.length).not.toBe(results[1].speakers.length);
    });
  });
});
