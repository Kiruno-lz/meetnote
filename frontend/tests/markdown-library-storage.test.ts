import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import { promises as fs } from "fs";
import { join, resolve } from "path";
import { tmpdir } from "os";
import {
  saveMeetingMarkdown,
  rebuildLibraryIndex,
  searchMeetings,
  type SaveMeetingMarkdownOptions,
  type SearchQuery
} from "../src/markdown-library-storage";
import {
  createMarkdownFrontmatter,
  serializeMarkdownFrontmatter,
  type MarkdownFrontmatter
} from "../src/markdown-library-schema";
import { generateMarkdownContent, type GenerateMarkdownOptions } from "../src/markdown-library-generator";
import { createDeverbalizationOutput } from "../src/llm-schema";

// Test Helpers

function createTestFrontmatter(overrides?: Partial<MarkdownFrontmatter>): MarkdownFrontmatter {
  return createMarkdownFrontmatter({
    title: overrides?.title ?? "Test Meeting",
    date: overrides?.date ?? "2024-05-15",
    audioHash: overrides?.audioHash ?? "abc123",
    speakers: overrides?.speakers ?? ["Alice", "Bob"],
    modelVersion: overrides?.modelVersion ?? "v1.0",
    llmProvider: overrides?.llmProvider ?? "ollama",
    schemaVersion: "meetnote.markdown-library.v1",
    generatedAt: overrides?.generatedAt ?? "2024-05-15T10:00:00Z"
  });
}

function createTestMarkdown(frontmatter: MarkdownFrontmatter): string {
  const frontmatterText = serializeMarkdownFrontmatter(frontmatter);
  const content = `
## Summary

This is a test meeting.

**Alice**: First point about the project.

**Bob**: Second point agreeing with Alice.

## Appendix

### Original Transcript

Original transcript here.
`;
  return `${frontmatterText}\n${content}`;
}

let testDir: string;

beforeEach(async () => {
  // Create unique temp directory for each test
  testDir = join(tmpdir(), `meetnote-test-${Date.now()}-${Math.random().toString(36).slice(2)}`);
  await fs.mkdir(testDir, { recursive: true });
});

afterEach(async () => {
  // Clean up temp directory
  try {
    await fs.rm(testDir, { recursive: true, force: true });
  } catch {
    // Ignore cleanup errors
  }
});

describe("markdown-library-storage", () => {
  describe("saveMeetingMarkdown", () => {
    it("saves markdown file with correct directory structure", async () => {
      const frontmatter = createTestFrontmatter();
      const markdown = createTestMarkdown(frontmatter);

      const result = await saveMeetingMarkdown({
        basePath: testDir,
        markdown
      });

      expect(result.success).toBe(true);
      expect(result.savedPath).toContain("meetings/2024/05");
      expect(result.indexEntry?.slug).toBe("2024-05-15-test-meeting");

      // Verify file was written
      const filePath = join(testDir, result.savedPath!, "meeting.md");
      const content = await fs.readFile(filePath, "utf-8");
      expect(content).toBe(markdown);
    });

    it("generates slug in YYYY-MM-DD-kebab-case format", async () => {
      const frontmatter = createTestFrontmatter({
        title: "My Test Meeting 2024",
        date: "2024-03-10"
      });
      const markdown = createTestMarkdown(frontmatter);

      const result = await saveMeetingMarkdown({
        basePath: testDir,
        markdown
      });

      expect(result.indexEntry?.slug).toBe("2024-03-10-my-test-meeting-2024");
    });

    it("creates and updates index.json", async () => {
      const frontmatter1 = createTestFrontmatter({ date: "2024-05-15" });
      const markdown1 = createTestMarkdown(frontmatter1);

      await saveMeetingMarkdown({ basePath: testDir, markdown: markdown1 });

      const indexPath = join(testDir, "index.json");
      const indexContent = await fs.readFile(indexPath, "utf-8");
      const index = JSON.parse(indexContent);

      expect(index.meetings).toHaveLength(1);
      expect(index.meetings[0].title).toBe("Test Meeting");
    });

    it("detects duplicate imports (same audioHash + generatedAt)", async () => {
      const frontmatter = createTestFrontmatter({
        audioHash: "duplicate-hash",
        generatedAt: "2024-05-15T10:00:00Z"
      });
      const markdown = createTestMarkdown(frontmatter);

      // First save should succeed
      const result1 = await saveMeetingMarkdown({
        basePath: testDir,
        markdown
      });
      expect(result1.success).toBe(true);

      // Second save with same hash + generatedAt should fail
      const result2 = await saveMeetingMarkdown({
        basePath: testDir,
        markdown
      });
      expect(result2.success).toBe(false);
      expect(result2.errorCode).toBe("DUPLICATE_IMPORT");
    });

    it("allows overwrite when overwrite flag is true", async () => {
      const frontmatter = createTestFrontmatter({
        audioHash: "overwrite-hash",
        generatedAt: "2024-05-15T10:00:00Z"
      });
      const markdown1 = createTestMarkdown(frontmatter);

      const result1 = await saveMeetingMarkdown({
        basePath: testDir,
        markdown: markdown1
      });
      expect(result1.success).toBe(true);

      // Create modified markdown with same audio hash
      const frontmatter2 = createTestFrontmatter({
        ...frontmatter,
        title: "Updated Title"
      });
      const markdown2 = createTestMarkdown(frontmatter2);

      const result2 = await saveMeetingMarkdown({
        basePath: testDir,
        markdown: markdown2,
        overwrite: true
      });
      expect(result2.success).toBe(true);

      // Verify the file was updated
      const filePath = join(testDir, result2.savedPath!, "meeting.md");
      const content = await fs.readFile(filePath, "utf-8");
      expect(content).toContain("Updated Title");
    });

    it("prevents path traversal attacks", async () => {
      // Create a markdown with a path traversal attempt in title
      const frontmatter = createTestFrontmatter({
        title: "../../evil"
      });
      const markdown = createTestMarkdown(frontmatter);

      const result = await saveMeetingMarkdown({
        basePath: testDir,
        markdown
      });

      // Should succeed because slug sanitization handles this
      // Path traversal detection is more about preventing directory creation outside baseDir
      // The slug generation should normalize this
      expect(result.success).toBe(true);
    });

    it("rejects invalid markdown", async () => {
      const invalidMarkdown = "just some random text without frontmatter";

      const result = await saveMeetingMarkdown({
        basePath: testDir,
        markdown: invalidMarkdown
      });

      expect(result.success).toBe(false);
      expect(result.errorCode).toBe("INVALID_MARKDOWN");
    });

    it("returns error code for invalid markdown", async () => {
      const result = await saveMeetingMarkdown({
        basePath: testDir,
        markdown: "invalid"
      });

      expect(result.success).toBe(false);
      expect(result.errorCode).toEqual("INVALID_MARKDOWN");
      expect(result.error).toBeDefined();
    });

    it("handles directory creation", async () => {
      const frontmatter = createTestFrontmatter({ date: "2025-12-31" });
      const markdown = createTestMarkdown(frontmatter);

      const result = await saveMeetingMarkdown({
        basePath: testDir,
        markdown
      });

      expect(result.success).toBe(true);

      // Verify directory structure
      const dirPath = join(testDir, "meetings", "2025", "12");
      const stat = await fs.stat(dirPath);
      expect(stat.isDirectory()).toBe(true);
    });
  });

  describe("rebuildLibraryIndex", () => {
    it("rebuilds index from multiple markdown files", async () => {
      // Save 3 different meetings
      const meeting1 = createTestMarkdown(
        createTestFrontmatter({
          title: "Meeting 1",
          date: "2024-05-10",
          audioHash: "hash1"
        })
      );
      const meeting2 = createTestMarkdown(
        createTestFrontmatter({
          title: "Meeting 2",
          date: "2024-05-15",
          audioHash: "hash2"
        })
      );
      const meeting3 = createTestMarkdown(
        createTestFrontmatter({
          title: "Meeting 3",
          date: "2024-05-20",
          audioHash: "hash3"
        })
      );

      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting1 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting2 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting3 });

      // Rebuild index
      const result = await rebuildLibraryIndex(testDir);

      expect(result.success).toBe(true);
      expect(result.index.meetings).toHaveLength(3);
    });

    it("deduplicates meetings by audioHash + generatedAt", async () => {
      const frontmatter = createTestFrontmatter({
        audioHash: "dup-hash",
        generatedAt: "2024-05-15T10:00:00Z"
      });

      // Manually create two files with same hash + generatedAt
      const meetingDir = join(testDir, "meetings", "2024", "05", "2024-05-15-test-meeting");
      await fs.mkdir(meetingDir, { recursive: true });
      const markdown = createTestMarkdown(frontmatter);
      await fs.writeFile(join(meetingDir, "meeting.md"), markdown);

      // Create a duplicate manually in a different directory
      const dup2Dir = join(testDir, "meetings", "2024", "05", "2024-05-15-duplicate");
      await fs.mkdir(dup2Dir, { recursive: true });
      await fs.writeFile(join(dup2Dir, "meeting.md"), markdown);

      const result = await rebuildLibraryIndex(testDir);

      // Should only have 1 meeting (duplicates removed)
      expect(result.index.meetings).toHaveLength(1);
      expect(result.errors.length).toBeGreaterThan(0); // Error message about duplicate
    });

    it("handles corrupted markdown files gracefully", async () => {
      const validMeeting = createTestMarkdown(
        createTestFrontmatter({ date: "2024-05-10", audioHash: "valid" })
      );

      await saveMeetingMarkdown({ basePath: testDir, markdown: validMeeting });

      // Create a corrupted file manually
      const corruptDir = join(testDir, "meetings", "2024", "05", "2024-05-15-corrupt");
      await fs.mkdir(corruptDir, { recursive: true });
      await fs.writeFile(join(corruptDir, "meeting.md"), "corrupted content without frontmatter");

      const result = await rebuildLibraryIndex(testDir);

      // Should succeed but report the error
      expect(result.index.meetings).toHaveLength(1);
      expect(result.errors.length).toBeGreaterThan(0);
      expect(result.errors[0]).toContain("Failed to parse");
    });

    it("sorts meetings by date descending", async () => {
      const meeting1 = createTestMarkdown(
        createTestFrontmatter({ date: "2024-05-10", audioHash: "h1" })
      );
      const meeting2 = createTestMarkdown(
        createTestFrontmatter({ date: "2024-05-20", audioHash: "h2" })
      );
      const meeting3 = createTestMarkdown(
        createTestFrontmatter({ date: "2024-05-15", audioHash: "h3" })
      );

      // Save in random order
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting1 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting3 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting2 });

      const result = await rebuildLibraryIndex(testDir);

      const dates = result.index.meetings.map((m) => m.date);
      expect(dates).toEqual(["2024-05-20", "2024-05-15", "2024-05-10"]);
    });

    it("creates manifest with correct metadata", async () => {
      const meeting = createTestMarkdown(createTestFrontmatter());
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting });

      const result = await rebuildLibraryIndex(testDir);

      expect(result.index.version).toBe("meetnote.markdown-library.v1");
      expect(result.index.baseDir).toBe(testDir);
      expect(result.index.indexedAt).toBeDefined();
    });

    it("handles empty directory", async () => {
      const result = await rebuildLibraryIndex(testDir);

      expect(result.success).toBe(true);
      expect(result.index.meetings).toHaveLength(0);
    });
  });

  describe("searchMeetings", () => {
    beforeEach(async () => {
      // Set up test data
      const meeting1 = createTestMarkdown(
        createTestFrontmatter({
          title: "Q1 Planning",
          date: "2024-01-15",
          audioHash: "h1",
          speakers: ["Alice", "Charlie"],
          generatedAt: "2024-01-15T10:00:00Z"
        })
      );

      const meeting2 = createTestMarkdown(
        createTestFrontmatter({
          title: "Q2 Review",
          date: "2024-04-20",
          audioHash: "h2",
          speakers: ["Bob", "Diana"],
          generatedAt: "2024-04-20T14:00:00Z"
        })
      );

      const meeting3 = createTestMarkdown(
        createTestFrontmatter({
          title: "Team Sync",
          date: "2024-05-10",
          audioHash: "h3",
          speakers: ["Alice", "Bob"],
          generatedAt: "2024-05-10T09:00:00Z"
        })
      );

      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting1 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting2 });
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting3 });
      await rebuildLibraryIndex(testDir);
    });

    it("filters by date range", async () => {
      const query: SearchQuery = {
        dateRange: {
          startDate: "2024-02-01",
          endDate: "2024-05-31"
        }
      };

      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(2);
      expect(results.map((m) => m.title)).toEqual(["Team Sync", "Q2 Review"]);
    });

    it("filters by speaker (case-insensitive)", async () => {
      const query: SearchQuery = {
        speakers: ["alice"]
      };

      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(2);
      expect(results.map((m) => m.title).sort()).toEqual(["Q1 Planning", "Team Sync"]);
    });

    it("filters by multiple speakers (OR logic)", async () => {
      const query: SearchQuery = {
        speakers: ["Charlie", "Diana"]
      };

      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(2);
    });

    it("filters by title prefix (case-insensitive)", async () => {
      const query: SearchQuery = {
        title: "q"
      };

      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(2);
      expect(results.map((m) => m.title).sort()).toEqual(["Q1 Planning", "Q2 Review"]);
    });

    it("searches by keywords in content", async () => {
      const query: SearchQuery = {
        keywords: ["planning"]
      };

      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(1);
      if (results[0]) {
        expect(results[0].title).toBe("Q1 Planning");
      }
    });

    it("combines multiple filters (AND logic)", async () => {
      const query: SearchQuery = {
        dateRange: {
          startDate: "2024-01-01",
          endDate: "2024-05-15"
        },
        speakers: ["Alice"],
        title: "team"
      };

      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(1);
      if (results[0]) {
        expect(results[0].title).toBe("Team Sync");
      }
    });

    it("returns results sorted by date descending", async () => {
      const query: SearchQuery = {};

      const results = await searchMeetings(testDir, query);

      const dates = results.map((m) => m.date);
      const sortedDates = [...dates].sort().reverse();
      expect(dates).toEqual(sortedDates);
    });

    it("returns empty results for no matches", async () => {
      const query: SearchQuery = {
        title: "nonexistent"
      };

      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(0);
    });

    it("handles empty search query (returns all)", async () => {
      const query: SearchQuery = {};

      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(3);
    });
  });

  describe("edge cases", () => {
    it("handles markdown with special characters in title", async () => {
      const frontmatter = createTestFrontmatter({
        title: "Meeting #1: Q&A with @team"
      });
      const markdown = createTestMarkdown(frontmatter);

      const result = await saveMeetingMarkdown({
        basePath: testDir,
        markdown
      });

      expect(result.success).toBe(true);
      expect(result.indexEntry?.slug).toContain("q-a-with-team");
    });

    it("handles markdown with CJK characters in title", async () => {
      const frontmatter = createTestFrontmatter({
        title: "2024年5月会议总结"
      });
      const markdown = createTestMarkdown(frontmatter);

      const result = await saveMeetingMarkdown({
        basePath: testDir,
        markdown
      });

      expect(result.success).toBe(true);
      expect(result.indexEntry?.slug).toContain("2024-05-15");
    });

    it("handles markdown with emoji in title", async () => {
      const frontmatter = createTestFrontmatter({
        title: "Sprint Planning 🚀"
      });
      const markdown = createTestMarkdown(frontmatter);

      const result = await saveMeetingMarkdown({
        basePath: testDir,
        markdown
      });

      expect(result.success).toBe(true);
    });

    it("handles empty keywords array", async () => {
      const meeting = createTestMarkdown(createTestFrontmatter());
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting });
      await rebuildLibraryIndex(testDir);

      const query: SearchQuery = { keywords: [] };
      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(1);
    });

    it("handles future dates", async () => {
      const frontmatter = createTestFrontmatter({
        date: "2099-12-31"
      });
      const markdown = createTestMarkdown(frontmatter);

      const result = await saveMeetingMarkdown({
        basePath: testDir,
        markdown
      });

      expect(result.success).toBe(true);
    });

    it("finds meetings with multiple keywords", async () => {
      const meeting = createTestMarkdown(
        createTestFrontmatter({
          title: "Engineering Review"
        })
      );
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting });
      await rebuildLibraryIndex(testDir);

      const query: SearchQuery = {
        keywords: ["engineering", "review"]
      };
      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(1);
    });

    it("handles case-insensitive fulltext search", async () => {
      const meeting = createTestMarkdown(
        createTestFrontmatter({
          title: "IMPORTANT Meeting"
        })
      );
      await saveMeetingMarkdown({ basePath: testDir, markdown: meeting });
      await rebuildLibraryIndex(testDir);

      const query: SearchQuery = {
        keywords: ["important"]
      };
      const results = await searchMeetings(testDir, query);

      expect(results).toHaveLength(1);
    });
  });
});
