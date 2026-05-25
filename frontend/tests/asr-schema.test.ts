import { describe, expect, test } from "bun:test";
import {
  createAsrProgressEvent,
  createAsrResult,
  createAsrRunRequest,
  createMockAsrResult,
  validateAsrCliEnvelope
} from "../src/types/asr-schema";

const SHA256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

describe("ASR schema", () => {
  test("creates deterministic run requests for supported audio formats", () => {
    expect(
      createAsrRunRequest({
        requestId: "req-1",
        audioPath: "/audio/meeting.wav",
        audioFormat: ".wav",
        audioSha256: SHA256,
        model: {
          id: "whisper-large-v3",
          version: "large-v3",
          path: "/models/whisper"
        },
        languageHint: "zh-primary-en-mixed",
        options: {
          wordTimestamps: true,
          temperature: 0,
          task: "transcribe"
        }
      })
    ).toMatchObject({
      kind: "asr.run.request",
      schemaVersion: "meetnote.asr.v1",
      audio: {
        format: ".wav",
        sha256: SHA256
      }
    });
  });

  test("rejects unsupported formats and invalid checksums at the CLI boundary", () => {
    const result = validateAsrCliEnvelope({
      kind: "asr.run.request",
      schemaVersion: "meetnote.asr.v1",
      requestId: "req-1",
      audio: {
        path: "/audio/meeting.aac",
        format: ".aac",
        sha256: "bad"
      },
      model: {
        id: "whisper-large-v3",
        version: "large-v3",
        path: "/models/whisper"
      },
      languageHint: "zh-primary-en-mixed",
      options: {
        wordTimestamps: true,
        temperature: 0,
        task: "transcribe"
      }
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toContain("ASR_AUDIO_FORMAT_UNSUPPORTED: audio.format must be .wav, .mp3, or .m4a");
    expect(result.errors).toContain("ASR_AUDIO_SHA256_INVALID: audio.sha256 must be 64 lowercase hex characters");
  });

  test("calculates bounded progress from deterministic durations", () => {
    expect(
      createAsrProgressEvent({
        requestId: "req-1",
        stage: "transcribing",
        processedMs: 10_000,
        totalMs: 40_000,
        message: "Transcribing"
      }).progress
    ).toBe(0.25);
    expect(
      validateAsrCliEnvelope({
        kind: "asr.progress",
        schemaVersion: "meetnote.asr.v1",
        requestId: "req-1",
        stage: "transcribing",
        processedMs: 80_000,
        totalMs: 40_000,
        progress: 1,
        message: "Transcribing"
      }).errors
    ).toContain("ASR_PROGRESS_RANGE_INVALID: processedMs cannot exceed totalMs");
  });

  test("validates transcript segments and model metadata in ASR results", () => {
    const valid = createAsrResult({
      requestId: "req-1",
      model: {
        id: "whisper-large-v3",
        version: "large-v3",
        parameters: {
          language: "zh",
          temperature: 0
        }
      },
      segments: [
        {
          id: "seg-1",
          startMs: 0,
          endMs: 1200,
          text: "开始会议。",
          language: "zh",
          confidence: 0.98
        }
      ]
    });

    expect(validateAsrCliEnvelope(valid)).toEqual({ ok: true, errors: [] });
    expect(
      validateAsrCliEnvelope({
        ...valid,
        segments: [
          {
            id: "seg-1",
            startMs: 1200,
            endMs: 1200,
            text: "",
            language: "fr",
            confidence: 2
          }
        ]
      }).errors
    ).toEqual([
      "ASR_SEGMENT_END_INVALID: segments[0].endMs must be greater than startMs",
      "ASR_SEGMENT_TEXT_REQUIRED: segments[0].text is required",
      "ASR_SEGMENT_LANGUAGE_INVALID: segments[0].language is unsupported",
      "ASR_SEGMENT_CONFIDENCE_INVALID: segments[0].confidence must be between 0 and 1"
    ]);
  });

  test("accepts only stable ASR error codes", () => {
    expect(
      validateAsrCliEnvelope({
        kind: "asr.error",
        schemaVersion: "meetnote.asr.v1",
        requestId: "req-1",
        code: "ASR_USER_CANCELLED",
        message: "Cancelled",
        recoverable: true
      })
    ).toEqual({ ok: true, errors: [] });
    expect(
      validateAsrCliEnvelope({
        kind: "asr.error",
        schemaVersion: "meetnote.asr.v1",
        requestId: "req-1",
        code: "RANDOM_FAILURE",
        message: "Unknown",
        recoverable: false
      }).errors
    ).toContain("ASR_ERROR_CODE_INVALID: code is unsupported");
  });

  test("creates deterministic raw Chinese and English mixed mock transcripts", () => {
    const result = createMockAsrResult({
      requestId: "req-mock",
      model: {
        id: "whisper-large-v3",
        version: "large-v3",
        parameters: {
          language: "mixed",
          temperature: 0
        }
      }
    });

    expect(result).toMatchObject({
      kind: "asr.result",
      schemaVersion: "meetnote.asr.v1",
      requestId: "req-mock",
      modelVersion: "large-v3",
      parameters: {
        language: "mixed",
        temperature: 0
      },
      segments: [
        {
          id: "asr-seg-0001",
          startMs: 0,
          endMs: 2480,
          text: "呃我们今天先看一下产品方案。",
          language: "zh",
          confidence: 0.94
        },
        {
          id: "asr-seg-0002",
          startMs: 2480,
          endMs: 5200,
          text: "然后 Alex 补充一下 Q2 OKR 的风险。",
          language: "mixed",
          confidence: 0.91
        }
      ]
    });
    expect(JSON.stringify(result)).not.toContain("summary");
    expect(result.segments[0]?.text).toContain("呃");
    expect(validateAsrCliEnvelope(result)).toEqual({ ok: true, errors: [] });
  });

  test("requires confidence and accepts explicit user cancellation code", () => {
    const base = createMockAsrResult({
      requestId: "req-confidence",
      model: {
        id: "whisper-large-v3",
        version: "large-v3",
        parameters: {
          language: "mixed",
          temperature: 0
        }
      }
    });

    expect(
      validateAsrCliEnvelope({
        ...base,
        segments: [{ id: "seg-1", startMs: 0, endMs: 1000, text: "原始文本", language: "zh" }]
      }).errors
    ).toContain("ASR_SEGMENT_CONFIDENCE_REQUIRED: segments[0].confidence is required");
    expect(
      validateAsrCliEnvelope({
        kind: "asr.error",
        schemaVersion: "meetnote.asr.v1",
        requestId: "req-confidence",
        code: "ASR_USER_CANCELLED",
        message: "User cancelled transcription",
        recoverable: false
      })
    ).toEqual({ ok: true, errors: [] });
  });

  test("rejects malformed envelopes across request, progress, result, and error shapes", () => {
    expect(validateAsrCliEnvelope(null)).toEqual({
      ok: false,
      errors: ["ASR_ENVELOPE_INVALID: envelope must be an object"]
    });
    expect(
      validateAsrCliEnvelope({
        kind: "unknown",
        schemaVersion: "wrong"
      }).errors
    ).toEqual([
      "ASR_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.asr.v1",
      "ASR_KIND_INVALID: kind is unsupported"
    ]);
    expect(
      validateAsrCliEnvelope({
        kind: "asr.run.request",
        schemaVersion: "meetnote.asr.v1",
        requestId: "",
        audio: null,
        model: null,
        languageHint: "fr",
        options: null
      }).errors
    ).toEqual([
      "ASR_REQUEST_ID_REQUIRED",
      "ASR_AUDIO_INVALID: audio must be an object",
      "ASR_MODEL_INVALID: model must be an object",
      "ASR_LANGUAGE_HINT_INVALID: languageHint is unsupported",
      "ASR_OPTIONS_INVALID: options must be an object"
    ]);
    expect(
      validateAsrCliEnvelope({
        kind: "asr.progress",
        schemaVersion: "meetnote.asr.v1",
        requestId: "",
        stage: "unknown",
        processedMs: -1,
        totalMs: 0,
        progress: 2,
        message: ""
      }).errors
    ).toEqual([
      "ASR_REQUEST_ID_REQUIRED",
      "ASR_PROGRESS_STAGE_INVALID: stage is unsupported",
      "ASR_PROCESSED_MS_INVALID: processedMs must be non-negative",
      "ASR_TOTAL_MS_INVALID: totalMs must be positive",
      "ASR_PROGRESS_INVALID: progress must be between 0 and 1",
      "ASR_PROGRESS_MESSAGE_REQUIRED"
    ]);
    expect(
      validateAsrCliEnvelope({
        kind: "asr.result",
        schemaVersion: "meetnote.asr.v1",
        requestId: "",
        modelVersion: "",
        parameters: null,
        model: {
          id: "",
          version: "",
          parameters: null
        },
        segments: []
      }).errors
    ).toEqual([
      "ASR_REQUEST_ID_REQUIRED",
      "ASR_MODEL_VERSION_REQUIRED",
      "ASR_RESULT_PARAMETERS_INVALID: parameters must be an object",
      "ASR_MODEL_ID_REQUIRED",
      "ASR_MODEL_VERSION_REQUIRED",
      "ASR_MODEL_PARAMETERS_INVALID: model.parameters must be an object",
      "ASR_SEGMENTS_REQUIRED: segments must be a non-empty array"
    ]);
    expect(
      validateAsrCliEnvelope({
        kind: "asr.result",
        schemaVersion: "meetnote.asr.v1",
        requestId: "req",
        modelVersion: "large-v3",
        parameters: { language: "fr", temperature: 2 },
        model: {
          id: "whisper-large-v3",
          version: "large-v3",
          parameters: { language: "fr", temperature: 2 }
        },
        segments: ["bad"]
      }).errors
    ).toEqual([
      "ASR_RESULT_LANGUAGE_INVALID: parameters.language is unsupported",
      "ASR_TEMPERATURE_INVALID: parameters.temperature must be between 0 and 1",
      "ASR_MODEL_LANGUAGE_INVALID: model.parameters.language is unsupported",
      "ASR_TEMPERATURE_INVALID: model.parameters.temperature must be between 0 and 1",
      "ASR_SEGMENT_INVALID: segments[0] must be an object"
    ]);
    expect(
      validateAsrCliEnvelope({
        kind: "asr.error",
        schemaVersion: "meetnote.asr.v1",
        requestId: "",
        code: "BAD",
        message: "",
        recoverable: "yes"
      }).errors
    ).toEqual([
      "ASR_REQUEST_ID_REQUIRED",
      "ASR_ERROR_CODE_INVALID: code is unsupported",
      "ASR_ERROR_MESSAGE_REQUIRED",
      "ASR_ERROR_RECOVERABLE_INVALID: recoverable must be boolean"
    ]);
  });
});
