import { describe, it, expect } from "bun:test";
import {
  MARKDOWN_LIBRARY_SCHEMA_VERSION,
  createMarkdownFrontmatter,
  createMarkdownMeeting,
  createMarkdownLibraryManifest,
  validateMarkdownFrontmatter,
  validateMarkdownMeeting,
  validateMarkdownLibraryManifest,
  serializeMarkdownFrontmatter,
  parseMarkdownFrontmatter,
  type MarkdownFrontmatter,
  type MarkdownMeeting,
  type MarkdownLibraryManifest
} from "../src/types/markdown-library-schema";

describe("markdown-library-schema", () => {
  describe("createMarkdownFrontmatter", () => {
    it("creates frontmatter with all required fields matching the input", () => {
      const input = {
        title: "Team Standup",
        date: "2026-05-20",
        audioHash: "abc123def456",
        speakers: ["Alice", "Bob"],
        modelVersion: "claude-3-haiku",
        llmProvider: "ollama" as const,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      const frontmatter = createMarkdownFrontmatter(input);

      expect(frontmatter.kind).toBe("markdown.frontmatter");
      expect(frontmatter.title).toBe("Team Standup");
      expect(frontmatter.date).toBe("2026-05-20");
      expect(frontmatter.audioHash).toBe("abc123def456");
      expect(frontmatter.speakers).toEqual(["Alice", "Bob"]);
      expect(frontmatter.modelVersion).toBe("claude-3-haiku");
      expect(frontmatter.llmProvider).toBe("ollama");
      expect(frontmatter.schemaVersion).toBe(MARKDOWN_LIBRARY_SCHEMA_VERSION);
    });

    it("throws on invalid title", () => {
      const input = {
        title: "",
        date: "2026-05-20",
        audioHash: "abc123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "ollama" as const,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      expect(() => createMarkdownFrontmatter(input)).toThrow();
    });

    it("throws on invalid date format", () => {
      const input = {
        title: "Meeting",
        date: "05-20-2026",
        audioHash: "abc123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "ollama" as const,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      expect(() => createMarkdownFrontmatter(input)).toThrow();
    });

    it("throws on empty speakers", () => {
      const input = {
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "abc123",
        speakers: [],
        modelVersion: "v1",
        llmProvider: "ollama" as const,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      expect(() => createMarkdownFrontmatter(input)).toThrow();
    });

    it("throws on invalid llmProvider", () => {
      const input = {
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "abc123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "invalid" as any,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      expect(() => createMarkdownFrontmatter(input)).toThrow();
    });
  });

  describe("validateMarkdownFrontmatter", () => {
    it("validates correct frontmatter", () => {
      const frontmatter = {
        kind: "markdown.frontmatter" as const,
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "hash123",
        speakers: ["Alice", "Bob"],
        modelVersion: "v1",
        llmProvider: "ollama" as const,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      const result = validateMarkdownFrontmatter(frontmatter);
      expect(result.ok).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it("rejects non-object", () => {
      const result = validateMarkdownFrontmatter("not an object");
      expect(result.ok).toBe(false);
      expect(result.errors[0]).toContain("must be an object");
    });

    it("rejects missing title", () => {
      const frontmatter = {
        date: "2026-05-20",
        audioHash: "hash123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "ollama" as const,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      const result = validateMarkdownFrontmatter(frontmatter);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_TITLE_REQUIRED"))).toBe(true);
    });

    it("rejects invalid date format", () => {
      const frontmatter = {
        title: "Meeting",
        date: "invalid-date",
        audioHash: "hash123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "ollama" as const,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      const result = validateMarkdownFrontmatter(frontmatter);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_DATE_INVALID"))).toBe(true);
    });

    it("accepts ISO 8601 date format", () => {
      const frontmatter = {
        title: "Meeting",
        date: "2026-05-20T12:31:21.565Z",
        audioHash: "hash123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "ollama" as const,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      const result = validateMarkdownFrontmatter(frontmatter);
      expect(result.ok).toBe(true);
    });

    it("rejects empty speakers array", () => {
      const frontmatter = {
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "hash123",
        speakers: [],
        modelVersion: "v1",
        llmProvider: "ollama" as const,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      const result = validateMarkdownFrontmatter(frontmatter);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_SPEAKERS_EMPTY"))).toBe(true);
    });

    it("rejects speaker with empty string", () => {
      const frontmatter = {
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "hash123",
        speakers: ["Alice", ""],
        modelVersion: "v1",
        llmProvider: "ollama" as const,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      const result = validateMarkdownFrontmatter(frontmatter);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_SPEAKER_INVALID"))).toBe(true);
    });

    it("rejects wrong schema version", () => {
      const frontmatter = {
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "hash123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "ollama" as const,
        schemaVersion: "wrong.version" as any,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      const result = validateMarkdownFrontmatter(frontmatter);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_SCHEMA_VERSION_INVALID"))).toBe(true);
    });

    it("rejects invalid llmProvider", () => {
      const frontmatter = {
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "hash123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "invalid" as any,
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      const result = validateMarkdownFrontmatter(frontmatter);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_LLM_PROVIDER_INVALID"))).toBe(true);
    });
  });

  describe("createMarkdownMeeting", () => {
    it("creates valid meeting", () => {
      const frontmatter = createMarkdownFrontmatter({
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "hash123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "ollama",
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      });

      const meeting = createMarkdownMeeting({
        frontmatter,
        content: "This is the meeting content."
      });

      expect(meeting.kind).toBe("markdown.meeting");
      expect(meeting.content).toBe("This is the meeting content.");
    });

    it("throws on empty content", () => {
      const frontmatter = createMarkdownFrontmatter({
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "hash123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "ollama",
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      });

      expect(() =>
        createMarkdownMeeting({
          frontmatter,
          content: ""
        })
      ).toThrow();
    });
  });

  describe("validateMarkdownMeeting", () => {
    it("validates correct meeting", () => {
      const meeting = {
        kind: "markdown.meeting" as const,
        frontmatter: {
          kind: "markdown.frontmatter" as const,
          title: "Meeting",
          date: "2026-05-20",
          audioHash: "hash123",
          speakers: ["Alice"],
          modelVersion: "v1",
          llmProvider: "ollama" as const,
          schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
          generatedAt: "2026-05-20T12:31:21.565Z"
        },
        content: "Meeting content"
      };

      const result = validateMarkdownMeeting(meeting);
      expect(result.ok).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it("rejects missing frontmatter", () => {
      const meeting = {
        content: "Meeting content"
      };

      const result = validateMarkdownMeeting(meeting);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_FRONTMATTER_MISSING"))).toBe(true);
    });

    it("rejects empty content", () => {
      const meeting = {
        frontmatter: {
          kind: "markdown.frontmatter" as const,
          title: "Meeting",
          date: "2026-05-20",
          audioHash: "hash123",
          speakers: ["Alice"],
          modelVersion: "v1",
          llmProvider: "ollama" as const,
          schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
          generatedAt: "2026-05-20T12:31:21.565Z"
        },
        content: "   "
      };

      const result = validateMarkdownMeeting(meeting);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_CONTENT_EMPTY"))).toBe(true);
    });

    it("rejects non-string content", () => {
      const meeting = {
        frontmatter: {
          kind: "markdown.frontmatter" as const,
          title: "Meeting",
          date: "2026-05-20",
          audioHash: "hash123",
          speakers: ["Alice"],
          modelVersion: "v1",
          llmProvider: "ollama" as const,
          schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
          generatedAt: "2026-05-20T12:31:21.565Z"
        },
        content: 123
      };

      const result = validateMarkdownMeeting(meeting);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_CONTENT_INVALID"))).toBe(true);
    });
  });

  describe("createMarkdownLibraryManifest", () => {
    it("creates valid manifest", () => {
      const manifest = createMarkdownLibraryManifest({
        version: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        baseDir: "/meetings",
        indexedAt: "2026-05-20T12:31:21.565Z",
        meetingCount: 42
      });

      expect(manifest.kind).toBe("markdown.library.manifest");
      expect(manifest.meetingCount).toBe(42);
    });

    it("throws on negative meeting count", () => {
      expect(() =>
        createMarkdownLibraryManifest({
          version: MARKDOWN_LIBRARY_SCHEMA_VERSION,
          baseDir: "/meetings",
          indexedAt: "2026-05-20T12:31:21.565Z",
          meetingCount: -1
        })
      ).toThrow();
    });
  });

  describe("validateMarkdownLibraryManifest", () => {
    it("validates correct manifest", () => {
      const manifest = {
        kind: "markdown.library.manifest" as const,
        version: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        baseDir: "/meetings",
        indexedAt: "2026-05-20T12:31:21.565Z",
        meetingCount: 10
      };

      const result = validateMarkdownLibraryManifest(manifest);
      expect(result.ok).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it("rejects non-integer meeting count", () => {
      const manifest = {
        version: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        baseDir: "/meetings",
        indexedAt: "2026-05-20T12:31:21.565Z",
        meetingCount: 10.5
      };

      const result = validateMarkdownLibraryManifest(manifest);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_MEETING_COUNT_INVALID"))).toBe(true);
    });

    it("rejects negative meeting count", () => {
      const manifest = {
        version: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        baseDir: "/meetings",
        indexedAt: "2026-05-20T12:31:21.565Z",
        meetingCount: -5
      };

      const result = validateMarkdownLibraryManifest(manifest);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_MEETING_COUNT_INVALID"))).toBe(true);
    });

    it("rejects wrong version", () => {
      const manifest = {
        version: "wrong.version" as any,
        baseDir: "/meetings",
        indexedAt: "2026-05-20T12:31:21.565Z",
        meetingCount: 10
      };

      const result = validateMarkdownLibraryManifest(manifest);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_MANIFEST_VERSION_INVALID"))).toBe(true);
    });
  });

  describe("serializeMarkdownFrontmatter", () => {
    it("serializes frontmatter to YAML", () => {
      const frontmatter = createMarkdownFrontmatter({
        title: "Team Standup",
        date: "2026-05-20",
        audioHash: "abc123",
        speakers: ["Alice", "Bob"],
        modelVersion: "claude-3-haiku",
        llmProvider: "ollama",
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      });

      const serialized = serializeMarkdownFrontmatter(frontmatter);

      expect(serialized).toContain("---");
      expect(serialized).toContain("title: Team Standup");
      expect(serialized).toContain("date: 2026-05-20");
      expect(serialized).toContain("audioHash: abc123");
      expect(serialized).toContain("speakers: [Alice, Bob]");
      expect(serialized).toContain("modelVersion: claude-3-haiku");
      expect(serialized).toContain("llmProvider: ollama");
    });

    it("escapes special characters in title", () => {
      const frontmatter = createMarkdownFrontmatter({
        title: "Meeting: Q1 Review",
        date: "2026-05-20",
        audioHash: "abc123",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "ollama",
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      });

      const serialized = serializeMarkdownFrontmatter(frontmatter);
      expect(serialized).toContain('title: "Meeting: Q1 Review"');
    });

    it("escapes special characters in speaker names", () => {
      const frontmatter = createMarkdownFrontmatter({
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "abc123",
        speakers: ["Alice Smith", "Bob, Jr."],
        modelVersion: "v1",
        llmProvider: "ollama",
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      });

      const serialized = serializeMarkdownFrontmatter(frontmatter);
      expect(serialized).toContain("Alice Smith");
      expect(serialized).toContain('"Bob, Jr."');
    });
  });

  describe("parseMarkdownFrontmatter", () => {
    it("parses valid frontmatter block", () => {
      const text = `---
title: Team Standup
date: 2026-05-20
audioHash: abc123
speakers: [Alice, Bob]
modelVersion: claude-3-haiku
llmProvider: ollama
schemaVersion: meetnote.markdown-library.v1
generatedAt: 2026-05-20T12:31:21.565Z
---

Content here`;

      const result = parseMarkdownFrontmatter(text);

      expect(result.ok).toBe(true);
      expect(result.frontmatter?.title).toBe("Team Standup");
      expect(result.frontmatter?.date).toBe("2026-05-20");
      expect(result.frontmatter?.speakers).toEqual(["Alice", "Bob"]);
    });

    it("parses frontmatter with quoted speaker names", () => {
      const text = `---
title: Meeting
date: 2026-05-20
audioHash: abc123
speakers: ["Alice Smith", "Bob, Jr."]
modelVersion: v1
llmProvider: ollama
schemaVersion: meetnote.markdown-library.v1
generatedAt: 2026-05-20T12:31:21.565Z
---`;

      const result = parseMarkdownFrontmatter(text);

      expect(result.ok).toBe(true);
      expect(result.frontmatter?.speakers).toEqual(["Alice Smith", "Bob, Jr."]);
    });

    it("rejects missing frontmatter block", () => {
      const text = "No frontmatter here";

      const result = parseMarkdownFrontmatter(text);

      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_FRONTMATTER_MISSING"))).toBe(true);
    });

    it("rejects invalid parsed frontmatter", () => {
      const text = `---
title:
date: 2026-05-20
audioHash: abc123
speakers: [Alice]
modelVersion: v1
llmProvider: ollama
schemaVersion: meetnote.markdown-library.v1
generatedAt: 2026-05-20T12:31:21.565Z
---`;

      const result = parseMarkdownFrontmatter(text);

      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_TITLE_REQUIRED"))).toBe(true);
    });
  });

  describe("roundtrip serialization", () => {
    it("serializes and parses frontmatter consistently", () => {
      const original = createMarkdownFrontmatter({
        title: "Q1 Planning: Strategy & Goals",
        date: "2026-05-20",
        audioHash: "abc123def456",
        speakers: ["Alice Johnson", "Bob Smith, PhD"],
        modelVersion: "claude-3-haiku",
        llmProvider: "custom",
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      });

      const serialized = serializeMarkdownFrontmatter(original);
      const parsed = parseMarkdownFrontmatter(serialized);

      expect(parsed.ok).toBe(true);
      expect(parsed.frontmatter?.title).toBe(original.title);
      expect(parsed.frontmatter?.date).toBe(original.date);
      expect(parsed.frontmatter?.audioHash).toBe(original.audioHash);
      expect(parsed.frontmatter?.speakers).toEqual(original.speakers);
      expect(parsed.frontmatter?.modelVersion).toBe(original.modelVersion);
      expect(parsed.frontmatter?.llmProvider).toBe(original.llmProvider);
    });
  });

  describe("edge cases", () => {
    it("handles multiple speakers preserving order and content", () => {
      const speakers = ["Alice", "Bob", "Charlie", "Diana"];
      const frontmatter = createMarkdownFrontmatter({
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "hash",
        speakers,
        modelVersion: "v1",
        llmProvider: "ollama",
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      });

      expect(frontmatter.speakers).toHaveLength(4);
      expect(frontmatter.speakers).toEqual(speakers);
    });

    it("handles whitespace-only speaker names as invalid", () => {
      const frontmatter = {
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "hash",
        speakers: ["   "],
        modelVersion: "v1",
        llmProvider: "ollama",
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      };

      const result = validateMarkdownFrontmatter(frontmatter);
      expect(result.ok).toBe(false);
      expect(result.errors.some((e) => e.includes("MARKDOWN_SPEAKER_INVALID"))).toBe(true);
    });

    it("handles zero meeting count in manifest", () => {
      const manifest = createMarkdownLibraryManifest({
        version: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        baseDir: "/meetings",
        indexedAt: "2026-05-20T12:31:21.565Z",
        meetingCount: 0
      });

      expect(manifest.meetingCount).toBe(0);
    });

    it("handles multiline content in meeting", () => {
      const frontmatter = createMarkdownFrontmatter({
        title: "Meeting",
        date: "2026-05-20",
        audioHash: "hash",
        speakers: ["Alice"],
        modelVersion: "v1",
        llmProvider: "ollama",
        schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        generatedAt: "2026-05-20T12:31:21.565Z"
      });

      const content = `## Agenda

- Item 1
- Item 2

## Discussion

This is a detailed discussion.`;

      const meeting = createMarkdownMeeting({
        frontmatter,
        content
      });

      expect(meeting.content).toContain("## Agenda");
      expect(meeting.content).toContain("## Discussion");
    });
  });
});
