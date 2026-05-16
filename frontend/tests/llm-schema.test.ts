import { describe, expect, test } from "bun:test";
import { validateDeverbalizationOutput } from "../src/llm-schema";

describe("LLM deverbalization schema", () => {
  test("accepts faithful segment-level output from Ollama or custom providers", () => {
    const result = validateDeverbalizationOutput({
      kind: "deverbalization",
      schemaVersion: "meetnote.deverbalization.v1",
      provider: "ollama",
      segments: [
        {
          sourceSegmentIds: ["seg-1"],
          speakerId: "speaker-a",
          text: "我们今天讨论项目进度。"
        }
      ]
    });

    expect(result).toEqual({ ok: true, errors: [] });
  });

  test("rejects summaries and action items in MVP output", () => {
    const result = validateDeverbalizationOutput({
      kind: "deverbalization",
      schemaVersion: "meetnote.deverbalization.v1",
      provider: "custom",
      summary: "会议讨论了项目进度。",
      actionItems: ["下周发布"],
      segments: [
        {
          sourceSegmentIds: ["seg-1"],
          speakerId: "speaker-a",
          text: "我们今天讨论项目进度。"
        }
      ]
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toContain("Forbidden top-level key: summary");
    expect(result.errors).toContain("Forbidden top-level key: actionItems");
  });
});
