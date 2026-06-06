import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import { promises as fs } from "fs";
import { join } from "path";
import { tmpdir } from "os";
import {
  saveMeetingMarkdown,
  rebuildLibraryIndex,
  searchMeetings,
  type SearchQuery
} from "../../../frontend/src/stores/markdown-library-storage";
import {
  createMarkdownFrontmatter,
  serializeMarkdownFrontmatter,
  type MarkdownFrontmatter
} from "../../../frontend/src/types/markdown-library-schema";
import { generateMarkdownContent, type GenerateMarkdownOptions } from "../../../frontend/src/utils/markdown-library-generator";
import {
  createDeverbalizationOutput,
  type DeverbalizationOutput,
  type LlmProviderModel
} from "../../../frontend/src/types/llm-schema";

// Helper: Create real markdown via full generation pipeline
function createRealMarkdown(title: string, date: string, audioHash: string, speakers: readonly string[], generatedAt: string): string {
  const frontmatter = createMarkdownFrontmatter({
    title,
    date,
    audioHash,
    speakers,
    modelVersion: "llama-2-7b",
    llmProvider: "ollama",
    schemaVersion: "meetnote.markdown-library.v1",
    generatedAt
  });

  const model: LlmProviderModel = {
    id: "llama-2-7b",
    version: "1.0.0",
    parameters: {
      temperature: 0.7,
      maxOutputTokens: 2048
    }
  };

  const output = createDeverbalizationOutput({
    requestId: `req-${audioHash}`,
    provider: "ollama",
    model,
    paragraphs: [
      {
        id: "p1",
        sourceSegmentIds: ["s1"],
        speakerId: "speaker-alice",
        text: "Let's start with the quarterly goals. We need to focus on improving our platform performance."
      },
      {
        id: "p2",
        sourceSegmentIds: ["s2"],
        speakerId: "speaker-bob",
        text: "I agree. We should also prioritize customer feedback integration."
      },
      {
        id: "p3",
        sourceSegmentIds: ["s3"],
        speakerId: "speaker-alice",
        text: "Exactly. Let's make this a key priority for Q2."
      }
    ]
  });

  const options: GenerateMarkdownOptions = {
    frontmatter,
    deverbalizationOutput: output,
    originalTranscript: "Original transcript text here...",
    speakerCorrections: []
  };

  return generateMarkdownContent(options);
}

let testDir: string;

beforeEach(async () => {
  testDir = join(tmpdir(), `meetnote-contract-test-${Date.now()}-${Math.random().toString(36).slice(2)}`);
  await fs.mkdir(testDir, { recursive: true });
});

afterEach(async () => {
  try {
    await fs.rm(testDir, { recursive: true, force: true });
  } catch {
    // Ignore cleanup errors
  }
});

describe("library-rebuild.contract", () => {
  describe("complete import workflow", () => {
    it("imports 3 different meetings with correct structure and index", async () => {
      // Save 3 different meetings
      const meeting1 = createRealMarkdown(
        "Q1 Planning Session",
        "2024-01-15",
        "audio-hash-001",
        ["Alice", "Bob", "Charlie"],
        "2024-01-15T10:00:00Z"
      );

      const meeting2 = createRealMarkdown(
        "Q2 Review and Planning",
        "2024-04-20",
        "audio-hash-002",
        ["Alice", "Diana"],
        "2024-04-20T14:30:00Z"
      );

      const meeting3 = createRealMarkdown(
        "Sprint Planning",
        "2024-05-10",
        "audio-hash-003",
        ["Bob", "Charlie", "Eve"],
        "2024-05-10T09:15:00Z"
      );

      // Save all three
      const result1 = await saveMeetingMarkdown({ basePath: testDir, markdown: meeting1 });
      const result2 = await saveMeetingMarkdown({ basePath: testDir, markdown: meeting2 });
      const result3 = await saveMeetingMarkdown({ basePath: testDir, markdown: meeting3 });

      expect(result1.success).toBe(true);
      expect(result2.success).toBe(true);
      expect(result3.success).toBe(true);

      // Verify directory structure
      const dir1 = join(testDir, "meetings", "2024", "01");
      const dir2 = join(testDir, "meetings", "2024", "04");
      const dir3 = join(testDir, "meetings", "2024", "05");

      const stat1 = await fs.stat(dir1);
      const stat2 = await fs.stat(dir2);
      const stat3 = await fs.stat(dir3);

      expect(stat1.isDirectory()).toBe(true);
      expect(stat2.isDirectory()).toBe(true);
      expect(stat3.isDirectory()).toBe(true);

      // Verify index contains all 3
      const indexPath = join(testDir, "index.json");
      const indexContent = await fs.readFile(indexPath, "utf-8");
      const index = JSON.parse(indexContent);

      expect(index.meetings).toHaveLength(3);
      expect(index.meetings.map((m: any) => m.title)).toContain("Q1 Planning Session");
      expect(index.meetings.map((m: any) => m.title)).toContain("Q2 Review and Planning");
      expect(index.meetings.map((m: any) => m.title)).toContain("Sprint Planning");

      // Verify no duplicates
      const audioHashes = index.meetings.map((m: any) => `${m.audioHash}|${m.generatedAt}`);
      const uniqueHashes = new Set(audioHashes);
      expect(uniqueHashes.size).toBe(3);
    });
  });

  describe("duplicate import protection", () => {
    it("prevents duplicate imports with same audioHash + generatedAt", async () => {
      const audioHash = "duplicate-test-hash";
      const generatedAt = "2024-05-10T10:00:00Z";

      const meeting1 = createRealMarkdown(
        "Original Meeting",
        "2024-05-10",
        audioHash,
        ["Alice"],
        generatedAt
      );

      // Save first time
      const result1 = await saveMeetingMarkdown({ basePath: testDir, markdown: meeting1 });
      expect(result1.success).toBe(true);
      expect(result1.savedPath).toBeDefined();

      // Attempt to save again with same audioHash + generatedAt
      const result2 = await saveMeetingMarkdown({ basePath: testDir, markdown: meeting1 });
      expect(result2.success).toBe(false);
      expect(result2.errorCode).toBe("DUPLICATE_IMPORT");

      // Verify original file unchanged
      const originalPath = join(testDir, result1.savedPath!, "meeting.md");
      const originalContent = await fs.readFile(originalPath, "utf-8");
      expect(originalContent).toContain("Original Meeting");

      // Verify index has only 1 entry
      const indexPath = join(testDir, "index.json");
      const indexContent = await fs.readFile(indexPath, "utf-8");
      const index = JSON.parse(indexContent);
      expect(index.meetings).toHaveLength(1);
    });

    it("allows same title/date with different audioHash", async () => {
      const meeting1 = createRealMarkdown(
        "Same Title",
        "2024-05-10",
        "hash-1",
        ["Alice"],
        "2024-05-10T10:00:00Z"
      );

      const meeting2 = createRealMarkdown(
        "Same Title",
        "2024-05-10",
        "hash-2",
        ["Alice"],
        "2024-05-10T10:00:00Z"
      );

      const result1 = await saveMeetingMarkdown({ basePath: testDir, markdown: meeting1 });
      const result2 = await saveMeetingMarkdown({ basePath: testDir, markdown: meeting2 });

      expect(result1.success).toBe(true);
      expect(result2.success).toBe(true);

      const indexPath = join(testDir, "index.json");
      const indexContent = await fs.readFile(indexPath, "utf-8");
      const index = JSON.parse(indexContent);
      expect(index.meetings).toHaveLength(2);
    });
  });

  describe("file independence", () => {
    it("survives deletion of one meeting and maintains index integrity", async () => {
      // Save meetings A and B
      const meetingA = createRealMarkdown(
        "Meeting A",
        "2024-05-10",
        "hash-a",
        ["Alice"],
        "2024-05-10T10:00:00Z"
      );

      const meetingB = createRealMarkdown(
        "Meeting B",
        "2024-05-15",
        "hash-b",
        ["Bob"],
        "2024-05-15T11:00:00Z"
      );

      const resultA = await saveMeetingMarkdown({ basePath: testDir, markdown: meetingA });
      const resultB = await saveMeetingMarkdown({ basePath: testDir, markdown: meetingB });

      expect(resultA.success).toBe(true);
      expect(resultB.success).toBe(true);

      // Delete meeting A's directory
      const dirA = join(testDir, resultA.savedPath!);
      await fs.rm(dirA, { recursive: true });

      // Rebuild index
      const rebuildResult = await rebuildLibraryIndex(testDir);
      expect(rebuildResult.success).toBe(true);

      // Verify index has only B
      expect(rebuildResult.index.meetings).toHaveLength(1);
      expect(rebuildResult.index.meetings[0].title).toBe("Meeting B");

      // Search should return only B
      const searchResults = await searchMeetings(testDir, {});
      expect(searchResults).toHaveLength(1);
      expect(searchResults[0].title).toBe("Meeting B");
    });

    it("maintains search results after file deletion and rebuild", async () => {
      const meeting1 = createRealMarkdown(
        "Engineering Team Meeting",
        "2024-05-10",
        "hash-1",
        ["Alice", "Charlie"],
        "2024-05-10T10:00:00Z"
      );

      const meeting2 = createRealMarkdown(
        "Design Review",
        "2024-05-12",
        "hash-2",
        ["Bob", "Alice"],
        "2024-05-12T14:00:00Z"
      );

      const result1 = await saveMeetingMarkdown({ basePath: testDir, markdown: meeting1 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting2 });

      // Delete meeting1
      await fs.rm(join(testDir, result1.savedPath!), { recursive: true });
      await rebuildLibraryIndex(testDir);

      // Search for Alice should return only meeting2
      const results = await searchMeetings(testDir, { speakers: ["alice"] });
      expect(results).toHaveLength(1);
      expect(results[0].title).toBe("Design Review");
    });
  });

  describe("data integrity across operations", () => {
    it("preserves content through save, search, rebuild, and search cycle", async () => {
      const specialContent = createRealMarkdown(
        "Special Content Meeting 🚀",
        "2024-05-10",
        "hash-special",
        ["Alice", "Bob"],
        "2024-05-10T10:00:00Z"
      );

      // Step 1: Save
      const saveResult = await saveMeetingMarkdown({ basePath: testDir, markdown: specialContent });
      expect(saveResult.success).toBe(true);

      // Step 2: Search before rebuild
      const beforeRebuild = await searchMeetings(testDir, { speakers: ["alice"] });
      expect(beforeRebuild).toHaveLength(1);
      expect(beforeRebuild[0].title).toContain("Special Content Meeting");

      // Step 3: Rebuild index
      const rebuildResult = await rebuildLibraryIndex(testDir);
      expect(rebuildResult.success).toBe(true);

      // Step 4: Search after rebuild
      const afterRebuild = await searchMeetings(testDir, { speakers: ["alice"] });
      expect(afterRebuild).toHaveLength(1);
      expect(afterRebuild[0].title).toContain("Special Content Meeting");

      // Verify content is identical
      const beforeTitle = beforeRebuild[0].title;
      const afterTitle = afterRebuild[0].title;
      expect(beforeTitle).toBe(afterTitle);
    });

    it("handles multilingual content with CJK characters", async () => {
      const multilingualMeeting = createRealMarkdown(
        "2024年5月企划会议 - May Planning",
        "2024-05-10",
        "hash-cjk",
        ["张三", "李四"],
        "2024-05-10T10:00:00Z"
      );

      const result = await saveMeetingMarkdown({ basePath: testDir, markdown: multilingualMeeting });
      expect(result.success).toBe(true);

      await rebuildLibraryIndex(testDir);

      const searchResults = await searchMeetings(testDir, {});
      expect(searchResults).toHaveLength(1);
      expect(searchResults[0].title).toContain("2024年5月");
    });

    it("preserves speaker names through full cycle", async () => {
      const speakers = ["Alice Chen", "Bob 王", "Charlie@Corp"];
      const meeting = createRealMarkdown(
        "Team Meeting",
        "2024-05-10",
        "hash-speakers",
        speakers,
        "2024-05-10T10:00:00Z"
      );

      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting });
      await rebuildLibraryIndex(testDir);

      const results = await searchMeetings(testDir, {});
      expect(results[0].speakers).toContain("Alice Chen");
      expect(results[0].speakers).toContain("Bob 王");
      expect(results[0].speakers).toContain("Charlie@Corp");
    });
  });

  describe("corrupted file recovery", () => {
    it("handles corrupted markdown and continues indexing", async () => {
      // Save 3 valid meetings
      const valid1 = createRealMarkdown(
        "Meeting 1",
        "2024-05-10",
        "hash-1",
        ["Alice"],
        "2024-05-10T10:00:00Z"
      );

      const valid2 = createRealMarkdown(
        "Meeting 2",
        "2024-05-12",
        "hash-2",
        ["Bob"],
        "2024-05-12T10:00:00Z"
      );

      const valid3 = createRealMarkdown(
        "Meeting 3",
        "2024-05-15",
        "hash-3",
        ["Charlie"],
        "2024-05-15T10:00:00Z"
      );

      await saveMeetingMarkdown({ basePath: testDir, markdown: valid1 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: valid2 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: valid3 });

      // Corrupt the second meeting's file manually
      const corruptedPath = join(testDir, "meetings", "2024", "05", "2024-05-12-meeting-2", "meeting.md");
      await fs.writeFile(corruptedPath, "CORRUPTED CONTENT - no frontmatter", "utf-8");

      // Rebuild index
      const result = await rebuildLibraryIndex(testDir);

      // Should partially succeed: 2 meetings indexed, 1 error
      expect(result.index.meetings).toHaveLength(2);
      expect(result.errors.length).toBeGreaterThan(0);
      expect(result.errors.some((e) => e.includes("Failed to parse"))).toBe(true);

      // Verify the 2 valid meetings are in the index
      const titles = result.index.meetings.map((m) => m.title);
      expect(titles).toContain("Meeting 1");
      expect(titles).toContain("Meeting 3");
    });

    it("reports corrupted files in rebuild errors", async () => {
      const validMeeting = createRealMarkdown(
        "Valid Meeting",
        "2024-05-10",
        "hash-valid",
        ["Alice"],
        "2024-05-10T10:00:00Z"
      );

      await saveMeetingMarkdown({ basePath: testDir, markdown: validMeeting });

      // Create corrupted file manually
      const corruptDir = join(testDir, "meetings", "2024", "05", "2024-05-15-corrupt");
      await fs.mkdir(corruptDir, { recursive: true });
      await fs.writeFile(join(corruptDir, "meeting.md"), "invalid frontmatter", "utf-8");

      const result = await rebuildLibraryIndex(testDir);

      expect(result.errors.length).toBeGreaterThan(0);
      expect(result.errors.some((e) => e.includes("2024-05-15-corrupt"))).toBe(true);
    });
  });

  describe("search functionality", () => {
    beforeEach(async () => {
      const meeting1 = createRealMarkdown(
        "Performance Analysis",
        "2024-05-10",
        "hash-1",
        ["Alice", "Bob"],
        "2024-05-10T10:00:00Z"
      );

      const meeting2 = createRealMarkdown(
        "Security Review",
        "2024-05-12",
        "hash-2",
        ["Charlie", "Diana"],
        "2024-05-12T14:00:00Z"
      );

      const meeting3 = createRealMarkdown(
        "Architecture Discussion",
        "2024-05-15",
        "hash-3",
        ["Alice", "Eve"],
        "2024-05-15T11:00:00Z"
      );

      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting1 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting2 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting3 });
      await rebuildLibraryIndex(testDir);
    });

    it("searches across multiple dimensions", async () => {
      // Search by speaker + date range
      const results = await searchMeetings(testDir, {
        speakers: ["Alice"],
        dateRange: {
          startDate: "2024-05-10",
          endDate: "2024-05-12"
        }
      });

      expect(results).toHaveLength(1);
      expect(results[0].title).toBe("Performance Analysis");
    });

    it("returns correct count with filtering", async () => {
      const allResults = await searchMeetings(testDir, {});
      expect(allResults).toHaveLength(3);

      const aliceResults = await searchMeetings(testDir, { speakers: ["Alice"] });
      expect(aliceResults).toHaveLength(2);
    });

    it("maintains sort order after filtering", async () => {
      const results = await searchMeetings(testDir, { speakers: ["Alice"] });

      const dates = results.map((m) => m.date);
      const sorted = [...dates].sort().reverse();
      expect(dates).toEqual(sorted);
    });
  });
});
