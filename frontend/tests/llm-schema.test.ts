import { describe, expect, test } from "bun:test";
import {
  createDeverbalizationOutput,
  createLlmDeverbalizationRequest,
  createLlmProviderError,
  validateDeverbalizationOutput,
  validateLlmProviderEnvelope
} from "../src/llm-schema";

describe("LLM deverbalization schema", () => {
  test("accepts provider request envelopes with original transcript, timestamps, corrected speakers, and paragraph boundaries", () => {
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

    expect(validateLlmProviderEnvelope(request)).toEqual({ ok: true, errors: [] });
    expect(request).toMatchObject({
      kind: "llm.deverbalization.request",
      schemaVersion: "meetnote.llm-provider.v1",
      provider: "ollama",
      transcript: {
        originalText: "产品经理: 嗯我们先看 roadmap。\nAlex: 然后我补充风险。"
      }
    });
    expect(request.transcript.segments).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: "seg-1",
          startMs: 0,
          endMs: 2400,
          speakerId: "speaker-pm",
          speakerLabel: "产品经理",
          text: "嗯我们先看 roadmap。"
        })
      ])
    );
    expect(request.paragraphBoundaries).toEqual(
      expect.arrayContaining([
        {
          id: "para-1",
          speakerId: "speaker-pm",
          speakerLabel: "产品经理",
          sourceSegmentIds: ["seg-1"]
        }
      ])
    );
  });

  test("accepts provider result envelopes with every paragraph traced to source segments and speaker id", () => {
    const output = createDeverbalizationOutput({
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

    expect(validateDeverbalizationOutput(output)).toEqual({ ok: true, errors: [] });
    expect(validateLlmProviderEnvelope(output)).toEqual({ ok: true, errors: [] });
    expect(output).toMatchObject({
      kind: "llm.deverbalization.result",
      schemaVersion: "meetnote.llm-provider.v1",
      modelVersion: "2026-05-20",
      parameters: {
        temperature: 0,
        maxOutputTokens: 1200
      }
    });
    expect(output.paragraphs).toEqual(
      expect.arrayContaining([
        {
          id: "para-clean-1",
          sourceSegmentIds: ["seg-1"],
          speakerId: "speaker-pm",
          text: "我们先看 roadmap。"
        }
      ])
    );
  });

  test("rejects forbidden MVP fields in result envelopes", () => {
    const result = validateDeverbalizationOutput({
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
      summary: "会议讨论了项目进度。",
      actionItems: ["下周发布"],
      actions: ["发布"],
      todos: ["跟进风险"],
      conclusions: ["可以继续"],
      paragraphs: [
        {
          id: "para-clean-1",
          sourceSegmentIds: ["seg-1"],
          speakerId: "speaker-pm",
          text: "我们先看 roadmap。"
        }
      ]
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        "LLM_FORBIDDEN_FIELD: summary is not allowed in MVP output",
        "LLM_FORBIDDEN_FIELD: actionItems is not allowed in MVP output",
        "LLM_FORBIDDEN_FIELD: actions is not allowed in MVP output",
        "LLM_FORBIDDEN_FIELD: todos is not allowed in MVP output",
        "LLM_FORBIDDEN_FIELD: conclusions is not allowed in MVP output"
      ])
    );
  });

  test("rejects forbidden MVP fields inside result paragraphs", () => {
    const result = validateDeverbalizationOutput({
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
          text: "我们先看 roadmap。",
          summary: "会议讨论了 roadmap。",
          actionItems: ["跟进 roadmap"],
          actions: ["跟进"],
          todos: ["整理"],
          conclusions: ["继续推进"]
        }
      ]
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        "LLM_FORBIDDEN_FIELD: paragraphs[0].summary is not allowed in MVP output",
        "LLM_FORBIDDEN_FIELD: paragraphs[0].actionItems is not allowed in MVP output",
        "LLM_FORBIDDEN_FIELD: paragraphs[0].actions is not allowed in MVP output",
        "LLM_FORBIDDEN_FIELD: paragraphs[0].todos is not allowed in MVP output",
        "LLM_FORBIDDEN_FIELD: paragraphs[0].conclusions is not allowed in MVP output"
      ])
    );
  });

  test("accepts legacy deverbalization outputs through the public output validator", () => {
    expect(
      validateDeverbalizationOutput({
        kind: "deverbalization",
        schemaVersion: "meetnote.deverbalization.v1",
        provider: "ollama",
        segments: [
          {
            sourceSegmentIds: ["seg-1"],
            speakerId: "speaker-pm",
            text: "我们先看 roadmap。"
          }
        ]
      })
    ).toEqual({ ok: true, errors: [] });
  });

  test("rejects malformed and forbidden legacy deverbalization outputs", () => {
    const result = validateDeverbalizationOutput({
      kind: "deverbalization",
      schemaVersion: "meetnote.deverbalization.v1",
      provider: "custom",
      summary: "会议总结。",
      segments: [
        {
          sourceSegmentIds: [],
          speakerId: "",
          text: "",
          conclusions: ["继续推进"]
        }
      ]
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        "LLM_FORBIDDEN_FIELD: summary is not allowed in MVP output",
        "LLM_FORBIDDEN_FIELD: segments[0].conclusions is not allowed in MVP output",
        "LLM_LEGACY_SEGMENT_TRACE_REQUIRED: segments[0].sourceSegmentIds must be a non-empty array",
        "LLM_LEGACY_SEGMENT_SPEAKER_REQUIRED: segments[0].speakerId is required",
        "LLM_LEGACY_SEGMENT_TEXT_REQUIRED: segments[0].text is required"
      ])
    );
  });

  test("rejects missing traceability, invalid provider, schemaVersion, and kind", () => {
    const result = validateLlmProviderEnvelope({
      kind: "llm.deverbalization.summary",
      schemaVersion: "meetnote.deverbalization.v1",
      requestId: "llm-req-001",
      provider: "openai",
      paragraphs: [
        {
          id: "para-clean-1",
          sourceSegmentIds: [],
          speakerId: "",
          text: "我们先看 roadmap。"
        }
      ]
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        "LLM_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.llm-provider.v1",
        "LLM_KIND_INVALID: kind is unsupported",
        "LLM_PROVIDER_INVALID: provider must be ollama or custom",
        "LLM_PARAGRAPH_TRACE_REQUIRED: paragraphs[0].sourceSegmentIds must be a non-empty array",
        "LLM_PARAGRAPH_SPEAKER_REQUIRED: paragraphs[0].speakerId is required"
      ])
    );
  });

  test("accepts provider error envelopes for planned failure cases", () => {
    const envelopes = [
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

    expect(envelopes.map(validateLlmProviderEnvelope)).toEqual([
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] }
    ]);
    expect(envelopes.map((envelope) => envelope.code)).toEqual([
      "timeout",
      "token_insufficient",
      "privacy_confirmation_required",
      "user_cancelled",
      "schema_validation_failed"
    ]);
  });

  test("rejects malformed request envelope internals deterministically", () => {
    const result = validateLlmProviderEnvelope({
      kind: "llm.deverbalization.request",
      schemaVersion: "meetnote.llm-provider.v1",
      requestId: "",
      provider: "custom",
      transcript: {
        originalText: "",
        segments: [
          "bad",
          {
            id: "seg-1",
            startMs: -1,
            endMs: 0,
            speakerId: "speaker-a",
            speakerLabel: "",
            text: ""
          }
        ]
      },
      paragraphBoundaries: [
        "bad",
        {
          id: "",
          speakerId: "speaker-b",
          speakerLabel: "",
          sourceSegmentIds: ["", "missing", "seg-1"]
        }
      ]
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        "LLM_REQUEST_ID_REQUIRED: requestId is required",
        "LLM_TRANSCRIPT_TEXT_REQUIRED: transcript.originalText is required",
        "LLM_TRANSCRIPT_SEGMENT_INVALID: transcript.segments[0] must be an object",
        "LLM_TRANSCRIPT_SEGMENT_START_INVALID: transcript.segments[1].startMs must be non-negative",
        "LLM_TRANSCRIPT_SEGMENT_SPEAKER_LABEL_REQUIRED: transcript.segments[1].speakerLabel is required",
        "LLM_TRANSCRIPT_SEGMENT_TEXT_REQUIRED: transcript.segments[1].text is required",
        "LLM_PARAGRAPH_BOUNDARY_INVALID: paragraphBoundaries[0] must be an object",
        "LLM_PARAGRAPH_BOUNDARY_ID_REQUIRED: paragraphBoundaries[1].id is required",
        "LLM_PARAGRAPH_BOUNDARY_SPEAKER_LABEL_REQUIRED: paragraphBoundaries[1].speakerLabel is required",
        "LLM_SOURCE_SEGMENT_ID_INVALID: paragraphBoundaries[1].sourceSegmentIds[0] must be a non-empty string",
        "LLM_PARAGRAPH_BOUNDARY_SOURCE_UNKNOWN: paragraphBoundaries[1].sourceSegmentIds[1] must reference transcript.segments",
        "LLM_PARAGRAPH_BOUNDARY_SPEAKER_MISMATCH: paragraphBoundaries[1].speakerId must match referenced transcript segment speakerId"
      ])
    );
  });

  test("rejects malformed result metadata, paragraphs, and non-object outputs", () => {
    expect(validateDeverbalizationOutput(null)).toEqual({
      ok: false,
      errors: ["LLM_ENVELOPE_INVALID: envelope must be an object"]
    });

    const result = validateDeverbalizationOutput({
      kind: "llm.deverbalization.result",
      schemaVersion: "meetnote.llm-provider.v1",
      requestId: "",
      provider: "openai",
      modelVersion: "v2",
      parameters: {
        temperature: 2,
        maxOutputTokens: 0
      },
      model: {
        id: "",
        version: "v1",
        parameters: {
          temperature: -1,
          maxOutputTokens: 1.5
        }
      },
      paragraphs: [
        "bad",
        {
          id: "",
          sourceSegmentIds: [""],
          speakerId: "",
          text: ""
        }
      ]
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        "LLM_REQUEST_ID_REQUIRED: requestId is required",
        "LLM_PROVIDER_INVALID: provider must be ollama or custom",
        "LLM_TEMPERATURE_INVALID: parameters.temperature must be between 0 and 1",
        "LLM_MAX_OUTPUT_TOKENS_INVALID: parameters.maxOutputTokens must be a positive integer",
        "LLM_MODEL_ID_REQUIRED: model.id is required",
        "LLM_MODEL_VERSION_MISMATCH: modelVersion must equal model.version",
        "LLM_TEMPERATURE_INVALID: model.parameters.temperature must be between 0 and 1",
        "LLM_MAX_OUTPUT_TOKENS_INVALID: model.parameters.maxOutputTokens must be a positive integer",
        "LLM_MODEL_PARAMETERS_MISMATCH: parameters must equal model.parameters",
        "LLM_PARAGRAPH_INVALID: paragraphs[0] must be an object",
        "LLM_PARAGRAPH_ID_REQUIRED: paragraphs[1].id is required",
        "LLM_SOURCE_SEGMENT_ID_INVALID: paragraphs[1].sourceSegmentIds[0] must be a non-empty string",
        "LLM_PARAGRAPH_SPEAKER_REQUIRED: paragraphs[1].speakerId is required",
        "LLM_PARAGRAPH_TEXT_REQUIRED: paragraphs[1].text is required"
      ])
    );
  });

  test("rejects malformed error envelopes and non-object provider envelopes", () => {
    expect(validateLlmProviderEnvelope([])).toEqual({
      ok: false,
      errors: ["LLM_ENVELOPE_INVALID: envelope must be an object"]
    });

    const result = validateLlmProviderEnvelope({
      kind: "llm.deverbalization.error",
      schemaVersion: "meetnote.llm-provider.v1",
      requestId: "llm-req-001",
      provider: "ollama",
      code: "rate_limited",
      message: "",
      recoverable: "yes"
    });

    expect(result).toEqual({
      ok: false,
      errors: [
        "LLM_ERROR_CODE_INVALID: code is unsupported",
        "LLM_ERROR_MESSAGE_REQUIRED: message is required",
        "LLM_ERROR_RECOVERABLE_INVALID: recoverable must be boolean"
      ]
    });
  });
});
