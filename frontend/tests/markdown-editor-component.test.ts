import { describe, expect, test } from "bun:test";
import {
  isMarkdownBodyChanged,
  parseMarkdownParts,
  reassembleMarkdown,
  splitMarkdownLines,
} from "../src/components/markdown-editor-component.ts";

// ——— parseMarkdownParts ———

describe("parseMarkdownParts", () => {
  test("parses standard frontmatter and body", () => {
    const raw = "---\ntitle: test\n---\n正文内容";
    const parts = parseMarkdownParts(raw);
    expect(parts.frontmatter).toContain("---");
    expect(parts.frontmatter).toBe("---\ntitle: test\n---");
    expect(parts.body).toBe("正文内容");
  });

  test("returns empty frontmatter for plain body text", () => {
    const raw = "Just plain text\nno frontmatter here";
    const parts = parseMarkdownParts(raw);
    expect(parts.frontmatter).toBe("");
    expect(parts.body).toBe(raw);
  });

  test("returns empty body when only frontmatter is present", () => {
    const raw = "---\ntitle: only-fm\n---\n";
    const parts = parseMarkdownParts(raw);
    expect(parts.frontmatter).toBe("---\ntitle: only-fm\n---");
    expect(parts.body).toBe("");
  });

  test("does not mis-read --- inside body as frontmatter closer (only takes opening block)", () => {
    const raw = "---\ntitle: test\n---\n正文\n---\n这不是 frontmatter\n---";
    const parts = parseMarkdownParts(raw);
    // frontmatter should end at the FIRST closing ---
    expect(parts.frontmatter).toBe("---\ntitle: test\n---");
    // body should contain the rest including the later ---
    expect(parts.body).toContain("正文");
    expect(parts.body).toContain("---");
    expect(parts.body).toContain("这不是 frontmatter");
  });

  test("returns empty frontmatter and empty body for empty string", () => {
    const parts = parseMarkdownParts("");
    expect(parts.frontmatter).toBe("");
    expect(parts.body).toBe("");
  });

  test("returns empty frontmatter when text starts with --- but has no closing delimiter", () => {
    const raw = "---\ntitle: test\nno closing";
    const parts = parseMarkdownParts(raw);
    expect(parts.frontmatter).toBe("");
    expect(parts.body).toBe(raw);
  });
});

// ——— reassembleMarkdown ———

describe("reassembleMarkdown", () => {
  test("round-trips correctly when frontmatter is present", () => {
    const raw = "---\ntitle: test\n---\n正文内容";
    const parts = parseMarkdownParts(raw);
    const result = reassembleMarkdown(parts);
    expect(result).toBe(raw);
  });

  test("returns only body when frontmatter is empty string", () => {
    const parts = { frontmatter: "", body: "Just body text" };
    expect(reassembleMarkdown(parts)).toBe("Just body text");
  });

  test("round-trips a document with multi-line frontmatter and body", () => {
    const raw = "---\ntitle: Meeting\ndate: 2026-05-21\n---\n# Heading\n\nParagraph.";
    const parts = parseMarkdownParts(raw);
    expect(reassembleMarkdown(parts)).toBe(raw);
  });
});

// ——— isMarkdownBodyChanged ———

describe("isMarkdownBodyChanged", () => {
  test("returns false for identical content", () => {
    expect(isMarkdownBodyChanged("hello world", "hello world")).toBe(false);
  });

  test("returns false when content is the same after trimming whitespace", () => {
    expect(isMarkdownBodyChanged("  hello  ", "hello")).toBe(false);
    expect(isMarkdownBodyChanged("hello\n", "hello")).toBe(false);
  });

  test("returns true for different content", () => {
    expect(isMarkdownBodyChanged("hello", "world")).toBe(true);
    expect(isMarkdownBodyChanged("line1\nline2", "line1\nline3")).toBe(true);
  });

  test("returns false for two empty strings", () => {
    expect(isMarkdownBodyChanged("", "")).toBe(false);
  });
});

// ——— splitMarkdownLines ———

describe("splitMarkdownLines", () => {
  test("splits two lines correctly", () => {
    expect(splitMarkdownLines("line1\nline2")).toEqual(["line1", "line2"]);
  });

  test("returns array with single empty string for empty input", () => {
    expect(splitMarkdownLines("")).toEqual([""]);
  });

  test("trailing newline produces empty string as last element", () => {
    expect(splitMarkdownLines("a\n")).toEqual(["a", ""]);
  });

  test("splits multiple lines", () => {
    expect(splitMarkdownLines("a\nb\nc")).toEqual(["a", "b", "c"]);
  });

  test("preserves empty lines in the middle", () => {
    expect(splitMarkdownLines("a\n\nb")).toEqual(["a", "", "b"]);
  });
});
