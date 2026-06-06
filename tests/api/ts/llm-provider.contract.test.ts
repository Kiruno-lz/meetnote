import { describe, expect, test } from "bun:test";
import {
  createDeverbalizationOutput,
  createLlmDeverbalizationRequest,
  createLlmProviderError,
  validateDeverbalizationOutput,
  validateLlmProviderEnvelope
} from "../../../frontend/src/types/llm-schema";
import {
  createLlmRequestFromCorrectedSpeakerInput,
  renderDeverbalizationPrompt,
  runDeterministicMockLlmProvider
} from "../../../frontend/src/utils/llm-provider";

describe("LLM provider contract", () => {
  test("round-trips request envelopes through JSON serialization", () => {
    const request = createLlmDeverbalizationRequest({
      requestId: "llm-req-001",
      provider: "ollama",
      transcript: {
        originalText: "产品经理: 嗯我们先看 roadmap。\nAlex: 然后我补充风险。",
        segments: [
          {
            id: "seg-1",
            startMs: 0,
            endMs: 2400,
            speakerId: "speaker-pm",
            speakerLabel: "产品经理",
            text: "嗯我们先看 roadmap。"
          },
          {
            id: "seg-2",
            startMs: 2400,
            endMs: 5200,
            speakerId: "speaker-alex",
            speakerLabel: "Alex",
            text: "然后我补充风险。"
          }
        ]
      },
      paragraphBoundaries: [
        {
          id: "para-1",
          speakerId: "speaker-pm",
          speakerLabel: "产品经理",
          sourceSegmentIds: ["seg-1"]
        },
        {
          id: "para-2",
          speakerId: "speaker-alex",
          speakerLabel: "Alex",
          sourceSegmentIds: ["seg-2"]
        }
      ]
    });

    const parsed = JSON.parse(JSON.stringify(request)) as unknown;

    expect(validateLlmProviderEnvelope(parsed)).toEqual({ ok: true, errors: [] });
    expect(parsed).toEqual({
      kind: "llm.deverbalization.request",
      schemaVersion: "meetnote.llm-provider.v1",
      requestId: "llm-req-001",
      provider: "ollama",
      transcript: {
        originalText: "产品经理: 嗯我们先看 roadmap。\nAlex: 然后我补充风险。",
        segments: [
          {
            id: "seg-1",
            startMs: 0,
            endMs: 2400,
            speakerId: "speaker-pm",
            speakerLabel: "产品经理",
            text: "嗯我们先看 roadmap。"
          },
          {
            id: "seg-2",
            startMs: 2400,
            endMs: 5200,
            speakerId: "speaker-alex",
            speakerLabel: "Alex",
            text: "然后我补充风险。"
          }
        ]
      },
      paragraphBoundaries: [
        {
          id: "para-1",
          speakerId: "speaker-pm",
          speakerLabel: "产品经理",
          sourceSegmentIds: ["seg-1"]
        },
        {
          id: "para-2",
          speakerId: "speaker-alex",
          speakerLabel: "Alex",
          sourceSegmentIds: ["seg-2"]
        }
      ]
    });
  });

  test("round-trips result envelopes through JSON serialization", () => {
    const result = createDeverbalizationOutput({
      requestId: "llm-req-001",
      provider: "custom",
      model: {
        id: "custom-provider",
        version: "2026-05-20",
        parameters: {
          temperature: 0,
          maxOutputTokens: 1200
        }
      },
      paragraphs: [
        {
          id: "para-clean-1",
          sourceSegmentIds: ["seg-1"],
          speakerId: "speaker-pm",
          text: "我们先看 roadmap。"
        },
        {
          id: "para-clean-2",
          sourceSegmentIds: ["seg-2"],
          speakerId: "speaker-alex",
          text: "我补充风险。"
        }
      ]
    });

    const parsed = JSON.parse(JSON.stringify(result)) as unknown;

    expect(validateDeverbalizationOutput(parsed)).toEqual({ ok: true, errors: [] });
    expect(parsed).toEqual({
      kind: "llm.deverbalization.result",
      schemaVersion: "meetnote.llm-provider.v1",
      requestId: "llm-req-001",
      provider: "custom",
      model: {
        id: "custom-provider",
        version: "2026-05-20",
        parameters: {
          temperature: 0,
          maxOutputTokens: 1200
        }
      },
      modelVersion: "2026-05-20",
      parameters: {
        temperature: 0,
        maxOutputTokens: 1200
      },
      paragraphs: [
        {
          id: "para-clean-1",
          sourceSegmentIds: ["seg-1"],
          speakerId: "speaker-pm",
          text: "我们先看 roadmap。"
        },
        {
          id: "para-clean-2",
          sourceSegmentIds: ["seg-2"],
          speakerId: "speaker-alex",
          text: "我补充风险。"
        }
      ]
    });
  });

  test("round-trips stable error envelopes through JSON serialization", () => {
    const errors = [
      createLlmProviderError({
        requestId: "llm-req-001",
        provider: "ollama",
        code: "timeout",
        message: "Provider timed out.",
        recoverable: true
      }),
      createLlmProviderError({
        requestId: "llm-req-001",
        provider: "custom",
        code: "token_insufficient",
        message: "Provider token budget is insufficient.",
        recoverable: true
      }),
      createLlmProviderError({
        requestId: "llm-req-001",
        provider: "custom",
        code: "privacy_confirmation_required",
        message: "Confirm cloud processing before sending transcript text.",
        recoverable: true
      }),
      createLlmProviderError({
        requestId: "llm-req-001",
        provider: "ollama",
        code: "user_cancelled",
        message: "The user cancelled deverbalization.",
        recoverable: true
      }),
      createLlmProviderError({
        requestId: "llm-req-001",
        provider: "custom",
        code: "schema_validation_failed",
        message: "Provider output failed schema validation.",
        recoverable: false
      })
    ];

    const parsed = errors.map((envelope) => JSON.parse(JSON.stringify(envelope)) as unknown);

    expect(parsed.map(validateLlmProviderEnvelope)).toEqual([
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] }
    ]);
    expect(parsed).toEqual([
      {
        kind: "llm.deverbalization.error",
        schemaVersion: "meetnote.llm-provider.v1",
        requestId: "llm-req-001",
        provider: "ollama",
        code: "timeout",
        message: "Provider timed out.",
        recoverable: true
      },
      {
        kind: "llm.deverbalization.error",
        schemaVersion: "meetnote.llm-provider.v1",
        requestId: "llm-req-001",
        provider: "custom",
        code: "token_insufficient",
        message: "Provider token budget is insufficient.",
        recoverable: true
      },
      {
        kind: "llm.deverbalization.error",
        schemaVersion: "meetnote.llm-provider.v1",
        requestId: "llm-req-001",
        provider: "custom",
        code: "privacy_confirmation_required",
        message: "Confirm cloud processing before sending transcript text.",
        recoverable: true
      },
      {
        kind: "llm.deverbalization.error",
        schemaVersion: "meetnote.llm-provider.v1",
        requestId: "llm-req-001",
        provider: "ollama",
        code: "user_cancelled",
        message: "The user cancelled deverbalization.",
        recoverable: true
      },
      {
        kind: "llm.deverbalization.error",
        schemaVersion: "meetnote.llm-provider.v1",
        requestId: "llm-req-001",
        provider: "custom",
        code: "schema_validation_failed",
        message: "Provider output failed schema validation.",
        recoverable: false
      }
    ]);
  });

  test("round-trips deterministic mock provider request and result through JSON serialization", () => {
    const request = createLlmRequestFromCorrectedSpeakerInput({
      requestId: "llm-req-contract-001",
      input: {
        kind: "corrected-speaker-deverbalization-input",
        schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
        provider: "ollama",
        text: "主持人: 嗯今天先看进度。\n研发: 然后我补充风险。",
        segments: [
          {
            sourceSegmentIds: ["asr-1"],
            speakerId: "speaker-host",
            speakerLabel: "主持人",
            text: "嗯今天先看进度。"
          },
          {
            sourceSegmentIds: ["asr-2"],
            speakerId: "speaker-dev",
            speakerLabel: "研发",
            text: "然后我补充风险。"
          }
        ]
      },
      segmentTimings: [
        { sourceSegmentId: "asr-1", startMs: 0, endMs: 1800 },
        { sourceSegmentId: "asr-2", startMs: 1800, endMs: 4200 }
      ]
    });
    const result = runDeterministicMockLlmProvider({ request });

    const parsedRequest = JSON.parse(JSON.stringify(request)) as unknown;
    const parsedResult = JSON.parse(JSON.stringify(result)) as unknown;

    expect(validateLlmProviderEnvelope(parsedRequest)).toEqual({ ok: true, errors: [] });
    expect(validateLlmProviderEnvelope(parsedResult)).toEqual({ ok: true, errors: [] });
    expect(parsedResult).toEqual({
      kind: "llm.deverbalization.result",
      schemaVersion: "meetnote.llm-provider.v1",
      requestId: "llm-req-contract-001",
      provider: "ollama",
      model: {
        id: "meetnote-mock-deverbalizer",
        version: "deterministic-v1",
        parameters: {
          temperature: 0,
          maxOutputTokens: 4096
        }
      },
      modelVersion: "deterministic-v1",
      parameters: {
        temperature: 0,
        maxOutputTokens: 4096
      },
      paragraphs: [
        {
          id: "paragraph-001",
          sourceSegmentIds: ["asr-1"],
          speakerId: "speaker-host",
          text: "今天先看进度。"
        },
        {
          id: "paragraph-002",
          sourceSegmentIds: ["asr-2"],
          speakerId: "speaker-dev",
          text: "我补充风险。"
        }
      ]
    });
  });

  test("rejects fabricated traceability when source ids or timings are missing", () => {
    expect(() =>
      createLlmRequestFromCorrectedSpeakerInput({
        requestId: "llm-req-contract-missing-source-001",
        input: {
          kind: "corrected-speaker-deverbalization-input",
          schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
          provider: "ollama",
          text: "主持人: 保留真实追溯。",
          segments: [
            {
              sourceSegmentIds: [],
              speakerId: "speaker-host",
              speakerLabel: "主持人",
              text: "保留真实追溯。"
            }
          ]
        },
        segmentTimings: []
      })
    ).toThrow("LLM_SOURCE_SEGMENT_IDS_REQUIRED: input.segments[0].sourceSegmentIds must be a non-empty array");

    expect(() =>
      createLlmRequestFromCorrectedSpeakerInput({
        requestId: "llm-req-contract-missing-timing-001",
        input: {
          kind: "corrected-speaker-deverbalization-input",
          schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
          provider: "ollama",
          text: "主持人: 保留真实时间戳。",
          segments: [
            {
              sourceSegmentIds: ["asr-1"],
              speakerId: "speaker-host",
              speakerLabel: "主持人",
              text: "保留真实时间戳。"
            }
          ]
        },
        segmentTimings: []
      })
    ).toThrow("LLM_SOURCE_SEGMENT_TIMING_REQUIRED: segmentTimings must include input.segments[0].sourceSegmentIds[0]");
  });

  test("keeps custom provider privacy failure JSON-safe before prompt creation", () => {
    const request = createLlmRequestFromCorrectedSpeakerInput({
      requestId: "llm-req-contract-privacy-001",
      input: {
        kind: "corrected-speaker-deverbalization-input",
        schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
        provider: "custom",
        text: "主持人: 私密会议内容。",
        segments: [
          {
            sourceSegmentIds: ["asr-1"],
            speakerId: "speaker-host",
            speakerLabel: "主持人",
            text: "私密会议内容。"
          }
        ]
      },
      segmentTimings: [{ sourceSegmentId: "asr-1", startMs: 0, endMs: 1200 }]
    });
    const renderedPrompts: string[] = [];

    const error = runDeterministicMockLlmProvider({
      request,
      privacyConfirmed: false,
      onPromptRendered: (prompt) => renderedPrompts.push(prompt)
    });

    expect(renderedPrompts).toEqual([]);
    expect(JSON.parse(JSON.stringify(error))).toEqual({
      kind: "llm.deverbalization.error",
      schemaVersion: "meetnote.llm-provider.v1",
      requestId: "llm-req-contract-privacy-001",
      provider: "custom",
      code: "privacy_confirmation_required",
      message: "Confirm custom provider privacy handling before sending transcript text.",
      recoverable: true
    });
    expect(JSON.stringify(error)).not.toContain("私密会议内容");
    expect(validateLlmProviderEnvelope(error)).toEqual({ ok: true, errors: [] });
  });

  test("renders prompt as a stable text contract for provider adapters", () => {
    const request = createLlmRequestFromCorrectedSpeakerInput({
      requestId: "llm-req-contract-prompt-001",
      input: {
        kind: "corrected-speaker-deverbalization-input",
        schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
        provider: "ollama",
        text: "主持人: 嗯今天先看进度。",
        segments: [
          {
            sourceSegmentIds: ["asr-1"],
            speakerId: "speaker-host",
            speakerLabel: "主持人",
            text: "嗯今天先看进度。"
          }
        ]
      },
      segmentTimings: [{ sourceSegmentId: "asr-1", startMs: 0, endMs: 1800 }]
    });

    expect(renderDeverbalizationPrompt(request)).toContain(
      "Do not create summaries, action items, conclusions, expansions, or inferred content."
    );
    expect(renderDeverbalizationPrompt(request)).toContain("[00:00.000 - 00:01.800] id=asr-1 speakerId=speaker-host speakerLabel=主持人");
  });
});
