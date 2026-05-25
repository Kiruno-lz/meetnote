import { describe, it, expect } from "bun:test";
import {
  generateMarkdownContent,
  type GenerateMarkdownOptions,
  type SpeakerCorrection
} from "../src/utils/markdown-library-generator";
import {
  MARKDOWN_LIBRARY_SCHEMA_VERSION,
  createMarkdownFrontmatter,
  parseMarkdownFrontmatter,
  type MarkdownFrontmatter
} from "../src/types/markdown-library-schema";
import { createDeverbalizationOutput, type DeverbalizationOutput } from "../src/types/llm-schema";

describe("markdown-library-generator", () => {
  describe("generateMarkdownContent", () => {
    it("generates complete markdown with frontmatter, content, and appendix", () => {
      const options = createTestOptions();
      const result = generateMarkdownContent(options);

      expect(result).toContain("---");
      expect(result).toContain("title: Team Standup");
      expect(result).toContain("**Alice**: First paragraph.");
      expect(result).toContain("## Appendix");
      expect(result).toContain("### Original Transcript");
    });

    it("frontmatter is parseable after generation", () => {
      const options = createTestOptions();
      const result = generateMarkdownContent(options);
      const parsed = parseMarkdownFrontmatter(result);

      expect(parsed.ok).toBe(true);
      expect(parsed.frontmatter?.title).toBe("Team Standup");
      expect(parsed.frontmatter?.speakers).toEqual(["Alice", "Bob"]);
    });

    it("preserves all paragraph content without loss", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "First point." },
          { id: "p2", sourceSegmentIds: ["s2"], speakerId: "speaker-bob", text: "Second point." },
          { id: "p3", sourceSegmentIds: ["s3"], speakerId: "speaker-alice", text: "Third point." }
        ]
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("First point.");
      expect(result).toContain("Second point.");
      expect(result).toContain("Third point.");
    });

    it("formats paragraphs as **Speaker**: text", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "Hello world." }
        ]
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**Alice**: Hello world.");
    });

    it("handles multiple speakers correctly", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "Alice speaks." },
          { id: "p2", sourceSegmentIds: ["s2"], speakerId: "speaker-bob", text: "Bob responds." },
          { id: "p3", sourceSegmentIds: ["s3"], speakerId: "speaker-charlie", text: "Charlie adds." }
        ],
        frontmatterSpeakers: ["Alice", "Bob", "Charlie"]
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**Alice**: Alice speaks.");
      expect(result).toContain("**Bob**: Bob responds.");
      expect(result).toContain("**Charlie**: Charlie adds.");
    });

    it("escapes markdown special characters in content", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "Use *asterisks* and _underscores_." }
        ]
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**Alice**: Use *asterisks* and _underscores_.");
    });

    it("escapes markdown special characters in original transcript", () => {
      const options = createTestOptions({
        originalTranscript: "This has [brackets] and `backticks` and *stars*."
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("\\[brackets\\]");
      expect(result).toContain("\\`backticks\\`");
      expect(result).toContain("\\*stars\\*");
    });

    it("preserves original transcript in collapsed appendix", () => {
      const transcript = "Original: Alice speaks.\nOriginal: Bob responds.";
      const options = createTestOptions({ originalTranscript: transcript });

      const result = generateMarkdownContent(options);

      expect(result).toContain("<details>");
      expect(result).toContain("<summary>Click to expand</summary>");
      expect(result).toContain("</details>");
    });

    it("includes speaker corrections in appendix as JSON", () => {
      const corrections: SpeakerCorrection[] = [
        {
          paragraphId: "p1",
          originalSpeakerId: "speaker-unknown",
          correctedSpeakerId: "speaker-alice",
          correctedSpeakerLabel: "Alice",
          correctedAt: "2026-05-20T12:00:00.000Z"
        }
      ];

      const options = createTestOptions({ speakerCorrections: corrections });
      const result = generateMarkdownContent(options);

      expect(result).toContain("### Speaker Corrections");
      expect(result).toContain("```json");
      expect(result).toContain("paragraphId");
      expect(result).toContain("correctedSpeakerId");
    });

    it("applies speaker corrections to paragraph labels", () => {
      const corrections: SpeakerCorrection[] = [
        {
          paragraphId: "p1",
          originalSpeakerId: "speaker-unknown",
          correctedSpeakerId: "speaker-alice",
          correctedSpeakerLabel: "Alice Johnson",
          correctedAt: "2026-05-20T12:00:00.000Z"
        }
      ];

      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-unknown", text: "Corrected text." }
        ],
        speakerCorrections: corrections
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**Alice Johnson**: Corrected text.");
    });

    it("handles single paragraph without speaker corrections", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "Only one paragraph." }
        ],
        speakerCorrections: []
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**Alice**: Only one paragraph.");
      expect(result).not.toContain("### Speaker Corrections");
    });

    it("handles long text without performance degradation", () => {
      const longText = "This is a long paragraph. ".repeat(100);
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: longText }
        ]
      });

      const start = performance.now();
      const result = generateMarkdownContent(options);
      const duration = performance.now() - start;

      expect(result).toContain(longText);
      expect(duration).toBeLessThan(100);
    });

    it("handles 1000+ paragraphs within performance budget", () => {
      const paragraphs = Array.from({ length: 1000 }, (_, i) => ({
        id: `p${i}`,
        sourceSegmentIds: [`s${i}`],
        speakerId: i % 2 === 0 ? "speaker-alice" : "speaker-bob",
        text: `Paragraph ${i}.`
      }));

      const options = createTestOptions({ paragraphs });

      const start = performance.now();
      const result = generateMarkdownContent(options);
      const duration = performance.now() - start;

      expect(result).toContain("Paragraph 0.");
      expect(result).toContain("Paragraph 999.");
      expect(duration).toBeLessThan(100);
    });

    it("handles Chinese characters correctly", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "我们先看 roadmap。" }
        ],
        frontmatterSpeakers: ["Alice"]
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**Alice**: 我们先看 roadmap。");
    });

    it("handles mixed Chinese and English", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "Let's review the 计划 and 风险。" }
        ]
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**Alice**: Let's review the 计划 and 风险。");
    });

    it("handles Chinese punctuation without escaping", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "这是一个问题？是的，确实。" }
        ]
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**Alice**: 这是一个问题？是的，确实。");
    });

    it("preserves newlines within paragraph text", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "Line one.\nLine two.\nLine three." }
        ]
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**Alice**: Line one.\nLine two.\nLine three.");
    });

    it("separates paragraphs with double newlines", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "First." },
          { id: "p2", sourceSegmentIds: ["s2"], speakerId: "speaker-bob", text: "Second." }
        ]
      });

      const result = generateMarkdownContent(options);
      const contentStart = result.indexOf("**Alice**");
      const contentEnd = result.indexOf("## Appendix");
      const content = result.substring(contentStart, contentEnd);

      expect(content).toContain("**Alice**: First.\n\n**Bob**: Second.");
    });

    it("handles empty speaker corrections array", () => {
      const options = createTestOptions({ speakerCorrections: [] });
      const result = generateMarkdownContent(options);

      expect(result).not.toContain("### Speaker Corrections");
    });

    it("handles multiple corrections for different paragraphs", () => {
      const corrections: SpeakerCorrection[] = [
        {
          paragraphId: "p1",
          originalSpeakerId: "speaker-unknown-1",
          correctedSpeakerId: "speaker-alice",
          correctedSpeakerLabel: "Alice",
          correctedAt: "2026-05-20T12:00:00.000Z"
        },
        {
          paragraphId: "p2",
          originalSpeakerId: "speaker-unknown-2",
          correctedSpeakerId: "speaker-bob",
          correctedSpeakerLabel: "Bob",
          correctedAt: "2026-05-20T12:01:00.000Z"
        }
      ];

      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-unknown-1", text: "First." },
          { id: "p2", sourceSegmentIds: ["s2"], speakerId: "speaker-unknown-2", text: "Second." }
        ],
        speakerCorrections: corrections
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**Alice**: First.");
      expect(result).toContain("**Bob**: Second.");
    });

    it("handles speaker id with hyphens correctly", () => {
      const options = createTestOptions({
        paragraphs: [
          { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-product-manager", text: "Product update." }
        ]
      });

      const result = generateMarkdownContent(options);

      expect(result).toContain("**product manager**: Product update.");
    });

    it("generates valid markdown that can be parsed back", () => {
      const options = createTestOptions();
      const generated = generateMarkdownContent(options);
      const parsed = parseMarkdownFrontmatter(generated);

      expect(parsed.ok).toBe(true);
      expect(parsed.frontmatter).toBeDefined();
      expect(parsed.frontmatter?.title).toBe(options.frontmatter.title);
      expect(parsed.frontmatter?.audioHash).toBe(options.frontmatter.audioHash);
    });
  });
});

function createTestOptions(overrides?: {
  paragraphs?: Array<{ id: string; sourceSegmentIds: string[]; speakerId: string; text: string }>;
  originalTranscript?: string;
  speakerCorrections?: SpeakerCorrection[];
  frontmatterSpeakers?: string[];
}): GenerateMarkdownOptions {
  const paragraphs = overrides?.paragraphs ?? [
    { id: "p1", sourceSegmentIds: ["s1"], speakerId: "speaker-alice", text: "First paragraph." },
    { id: "p2", sourceSegmentIds: ["s2"], speakerId: "speaker-bob", text: "Second paragraph." }
  ];

  const frontmatterSpeakers = overrides?.frontmatterSpeakers ?? ["Alice", "Bob"];

  const frontmatter = createMarkdownFrontmatter({
    title: "Team Standup",
    date: "2026-05-20",
    audioHash: "test-hash-123",
    speakers: frontmatterSpeakers,
    modelVersion: "test-model-v1",
    llmProvider: "ollama",
    schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
    generatedAt: "2026-05-20T12:00:00.000Z"
  });

  const deverbalizationOutput = createDeverbalizationOutput({
    requestId: "test-req-001",
    provider: "ollama",
    model: {
      id: "test-model",
      version: "test-model-v1",
      parameters: { temperature: 0, maxOutputTokens: 4096 }
    },
    paragraphs
  });

  return {
    frontmatter,
    deverbalizationOutput,
    originalTranscript: overrides?.originalTranscript ?? "Original: Alice speaks.\nOriginal: Bob responds.",
    speakerCorrections: overrides?.speakerCorrections ?? []
  };
}
