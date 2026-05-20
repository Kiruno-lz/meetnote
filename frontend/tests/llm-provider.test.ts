import { describe, expect, test } from "bun:test";
import { attachSpeakerState, attachTranscript, createMeetingProject } from "../src/audio-project";
import {
  createLlmRequestFromCorrectedSpeakerInput,
  renderDeverbalizationPrompt,
  runDeterministicMockLlmProvider
} from "../src/llm-provider";
import { validateLlmProviderEnvelope } from "../src/llm-schema";
import { createCorrectedSpeakerLlmInput, projectMeetingCorrectedSpeakerOutput } from "../src/speaker-output";

describe("LLM provider integration", () => {
  test("returns deterministic valid mock result envelopes traced to corrected speaker input", () => {
    const request = createLlmRequestFromCorrectedSpeakerInput({
      requestId: "llm-req-mock-001",
      input: createSpeakerAwareInput("ollama"),
      segmentTimings: [
        { sourceSegmentId: "asr-1", startMs: 0, endMs: 2400 },
        { sourceSegmentId: "asr-2", startMs: 2400, endMs: 5200 }
      ]
    });

    const first = runDeterministicMockLlmProvider({ request });
    const second = runDeterministicMockLlmProvider({ request });

    expect(first).toEqual(second);
    expect(validateLlmProviderEnvelope(first)).toEqual({ ok: true, errors: [] });
    expect(first).toMatchObject({
      kind: "llm.deverbalization.result",
      requestId: "llm-req-mock-001",
      provider: "ollama",
      model: {
        id: "meetnote-mock-deverbalizer",
        version: "deterministic-v1"
      }
    });
    if (first.kind !== "llm.deverbalization.result") {
      throw new Error("expected mock result envelope");
    }
    expect(first.paragraphs).toEqual([
      {
        id: "paragraph-001",
        sourceSegmentIds: ["asr-1"],
        speakerId: "speaker-pm",
        text: "我们先看 roadmap。"
      },
      {
        id: "paragraph-002",
        sourceSegmentIds: ["asr-2"],
        speakerId: "speaker-alex",
        text: "我补充风险。"
      }
    ]);
  });

  test("renders prompt with transcript text, timestamps, speaker labels, boundaries, and MVP prohibitions", () => {
    const request = createLlmRequestFromCorrectedSpeakerInput({
      requestId: "llm-req-prompt-001",
      input: createSpeakerAwareInput("ollama"),
      segmentTimings: [
        { sourceSegmentId: "asr-1", startMs: 0, endMs: 2400 },
        { sourceSegmentId: "asr-2", startMs: 2400, endMs: 5200 }
      ]
    });

    const prompt = renderDeverbalizationPrompt(request);

    expect(prompt).toContain("Original transcript");
    expect(prompt).toContain("产品经理: 嗯我们先看 roadmap。");
    expect(prompt).toContain("Alex: 然后我补充风险。");
    expect(prompt).toContain("[00:00.000 - 00:02.400]");
    expect(prompt).toContain("[00:02.400 - 00:05.200]");
    expect(prompt).toContain("speakerLabel=产品经理");
    expect(prompt).toContain("speakerLabel=Alex");
    expect(prompt).toContain("Paragraph paragraph-001");
    expect(prompt).toContain("sourceSegmentIds=asr-1");
    expect(prompt).toContain("Paragraph paragraph-002");
    expect(prompt).toContain("sourceSegmentIds=asr-2");
    expect(prompt).toContain("Do not create summaries, action items, conclusions, expansions, or inferred content.");
  });

  test("blocks custom provider transcript sending until privacy confirmation exists", () => {
    const request = createLlmRequestFromCorrectedSpeakerInput({
      requestId: "llm-req-privacy-001",
      input: createSpeakerAwareInput("custom"),
      segmentTimings: [
        { sourceSegmentId: "asr-1", startMs: 0, endMs: 2400 },
        { sourceSegmentId: "asr-2", startMs: 2400, endMs: 5200 }
      ]
    });
    const renderedPrompts: string[] = [];

    const result = runDeterministicMockLlmProvider({
      request,
      privacyConfirmed: false,
      onPromptRendered: (prompt) => renderedPrompts.push(prompt)
    });

    expect(result).toEqual({
      kind: "llm.deverbalization.error",
      schemaVersion: "meetnote.llm-provider.v1",
      requestId: "llm-req-privacy-001",
      provider: "custom",
      code: "privacy_confirmation_required",
      message: "Confirm custom provider privacy handling before sending transcript text.",
      recoverable: true
    });
    expect(renderedPrompts).toEqual([]);
    expect(JSON.stringify(result)).not.toContain("roadmap");
    expect(validateLlmProviderEnvelope(result)).toEqual({ ok: true, errors: [] });
  });

  test("preserves multiple source transcript segment ids inside one speaker paragraph", () => {
    const request = createLlmRequestFromCorrectedSpeakerInput({
      requestId: "llm-req-multi-source-001",
      input: {
        kind: "corrected-speaker-deverbalization-input",
        schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
        provider: "ollama",
        text: "主持人: 嗯第一句。\n主持人: 然后第二句。",
        segments: [
          {
            sourceSegmentIds: ["asr-1", "asr-2"],
            speakerId: "speaker-host",
            speakerLabel: "主持人",
            text: "嗯第一句。\n然后第二句。"
          }
        ]
      },
      segmentTimings: [
        { sourceSegmentId: "asr-1", startMs: 0, endMs: 1000 },
        { sourceSegmentId: "asr-2", startMs: 1000, endMs: 2600 }
      ]
    });

    const result = runDeterministicMockLlmProvider({ request });

    expect(request.transcript.segments.map((segment) => segment.id)).toEqual(["asr-1", "asr-2"]);
    expect(request.paragraphBoundaries).toEqual([
      {
        id: "paragraph-001",
        speakerId: "speaker-host",
        speakerLabel: "主持人",
        sourceSegmentIds: ["asr-1", "asr-2"]
      }
    ]);
    expect(validateLlmProviderEnvelope(result)).toEqual({ ok: true, errors: [] });
    if (result.kind !== "llm.deverbalization.result") {
      throw new Error("expected mock result envelope");
    }
    expect(result.paragraphs).toEqual([
      {
        id: "paragraph-001",
        sourceSegmentIds: ["asr-1", "asr-2"],
        speakerId: "speaker-host",
        text: "第一句。\n第二句。"
      }
    ]);
  });

  test("rejects corrected speaker input without source segment ids", () => {
    expect(() =>
      createLlmRequestFromCorrectedSpeakerInput({
        requestId: "llm-req-missing-source-001",
        input: {
          kind: "corrected-speaker-deverbalization-input",
          schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
          provider: "ollama",
          text: "主持人: 需要保留真实追溯。",
          segments: [
            {
              sourceSegmentIds: [],
              speakerId: "speaker-host",
              speakerLabel: "主持人",
              text: "需要保留真实追溯。"
            }
          ]
        },
        segmentTimings: []
      })
    ).toThrow("LLM_SOURCE_SEGMENT_IDS_REQUIRED: input.segments[0].sourceSegmentIds must be a non-empty array");
  });

  test("rejects corrected speaker input when referenced segment timings are missing", () => {
    expect(() =>
      createLlmRequestFromCorrectedSpeakerInput({
        requestId: "llm-req-missing-timing-001",
        input: {
          kind: "corrected-speaker-deverbalization-input",
          schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
          provider: "ollama",
          text: "主持人: 需要真实时间戳。",
          segments: [
            {
              sourceSegmentIds: ["asr-1", "asr-2"],
              speakerId: "speaker-host",
              speakerLabel: "主持人",
              text: "需要真实时间戳。"
            }
          ]
        },
        segmentTimings: [{ sourceSegmentId: "asr-1", startMs: 0, endMs: 1000 }]
      })
    ).toThrow("LLM_SOURCE_SEGMENT_TIMING_REQUIRED: segmentTimings must include input.segments[0].sourceSegmentIds[1]");
  });

  test("turns project speaker output into request and mock result without forbidden MVP fields", () => {
    const projected = projectMeetingCorrectedSpeakerOutput(
      attachSpeakerState(
        attachTranscript(
          createMeetingProject({
            title: "Roadmap review",
            path: "/meetings/roadmap-review.m4a",
            hash: "roadmap-review-hash",
            durationSeconds: 8,
            createdAt: "2026-05-20T09:00:00.000Z"
          }),
          [
            { id: "asr-1", startMs: 0, endMs: 2400, text: "嗯我们先看 roadmap。", language: "mixed" },
            { id: "asr-2", startMs: 2400, endMs: 5200, text: "然后我补充风险。", language: "zh" }
          ]
        ),
        [
          { id: "speaker-pm", label: "产品经理" },
          { id: "speaker-alex", label: "Alex" }
        ],
        [
          { id: "spk-1", speakerId: "speaker-pm", startMs: 0, endMs: 2500 },
          { id: "spk-2", speakerId: "speaker-alex", startMs: 2500, endMs: 5600 }
        ],
        []
      )
    );
    const request = createLlmRequestFromCorrectedSpeakerInput({
      requestId: "llm-req-project-001",
      input: createCorrectedSpeakerLlmInput("ollama", projected),
      segmentTimings: projected.lines.map((line) => ({
        sourceSegmentId: line.transcriptSegmentId,
        startMs: line.startMs,
        endMs: line.endMs
      }))
    });

    const result = runDeterministicMockLlmProvider({ request });

    expect(validateLlmProviderEnvelope(request)).toEqual({ ok: true, errors: [] });
    expect(validateLlmProviderEnvelope(result)).toEqual({ ok: true, errors: [] });
    expect(result).not.toHaveProperty("summary");
    expect(result).not.toHaveProperty("actionItems");
    expect(result).not.toHaveProperty("conclusions");
    if (result.kind !== "llm.deverbalization.result") {
      throw new Error("expected mock result envelope");
    }
    expect(result.paragraphs.map((paragraph) => paragraph.sourceSegmentIds)).toEqual([["asr-1"], ["asr-2"]]);
    expect(result.paragraphs.map((paragraph) => paragraph.speakerId)).toEqual(["speaker-pm", "speaker-alex"]);
  });
});

function createSpeakerAwareInput(provider: "ollama" | "custom") {
  return {
    kind: "corrected-speaker-deverbalization-input",
    schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
    provider,
    text: "产品经理: 嗯我们先看 roadmap。\nAlex: 然后我补充风险。",
    segments: [
      {
        sourceSegmentIds: ["asr-1"],
        speakerId: "speaker-pm",
        speakerLabel: "产品经理",
        text: "嗯我们先看 roadmap。"
      },
      {
        sourceSegmentIds: ["asr-2"],
        speakerId: "speaker-alex",
        speakerLabel: "Alex",
        text: "然后我补充风险。"
      }
    ]
  } as const;
}
