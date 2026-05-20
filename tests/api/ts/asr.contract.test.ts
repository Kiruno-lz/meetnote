import { describe, expect, test } from "bun:test";
import {
  createAsrProgressEvent,
  createAsrResult,
  createAsrRunRequest,
  validateAsrCliEnvelope
} from "../../../frontend/src/asr-schema";

describe("ASR CLI contract", () => {
  test("round-trips a mock ASR request through JSON serialization", () => {
    const request = createAsrRunRequest({
      requestId: "asr-req-001",
      audioPath: "/meetings/product-review.zh-en.m4a",
      audioFormat: ".m4a",
      audioSha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      model: {
        id: "whisper-large-v3",
        version: "large-v3",
        path: "/models/whisper-large-v3"
      },
      languageHint: "zh-primary-en-mixed",
      options: {
        wordTimestamps: true,
        temperature: 0,
        task: "transcribe"
      }
    });

    const parsed = JSON.parse(JSON.stringify(request)) as unknown;

    expect(validateAsrCliEnvelope(parsed)).toEqual({ ok: true, errors: [] });
    expect(parsed).toEqual({
      kind: "asr.run.request",
      schemaVersion: "meetnote.asr.v1",
      requestId: "asr-req-001",
      audio: {
        path: "/meetings/product-review.zh-en.m4a",
        format: ".m4a",
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
      },
      model: {
        id: "whisper-large-v3",
        version: "large-v3",
        path: "/models/whisper-large-v3"
      },
      languageHint: "zh-primary-en-mixed",
      options: {
        wordTimestamps: true,
        temperature: 0,
        task: "transcribe"
      }
    });
  });

  test("round-trips progress, result, and stable errors through JSON serialization", () => {
    const progress = createAsrProgressEvent({
      requestId: "asr-req-001",
      stage: "transcribing",
      processedMs: 30_000,
      totalMs: 120_000,
      message: "Transcribing audio"
    });
    const result = createAsrResult({
      requestId: "asr-req-001",
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
          id: "seg-0001",
          startMs: 0,
          endMs: 2400,
          text: "我们先看一下 roadmap。",
          language: "mixed",
          confidence: 0.94
        }
      ]
    });
    const errors = [
      {
        kind: "asr.error",
        schemaVersion: "meetnote.asr.v1",
        requestId: "asr-req-001",
        code: "ASR_MODEL_MISSING",
        message: "Select or download an ASR model before transcription.",
        recoverable: true
      },
      {
        kind: "asr.error",
        schemaVersion: "meetnote.asr.v1",
        requestId: "asr-req-001",
        code: "ASR_UNSUPPORTED_AUDIO_FORMAT",
        message: "Only .wav, .mp3, and .m4a files are supported.",
        recoverable: true
      },
      {
        kind: "asr.error",
        schemaVersion: "meetnote.asr.v1",
        requestId: "asr-req-001",
        code: "ASR_PROCESS_FAILED",
        message: "The ASR subprocess exited with a non-zero status.",
        recoverable: false
      },
      {
        kind: "asr.error",
        schemaVersion: "meetnote.asr.v1",
        requestId: "asr-req-001",
        code: "ASR_USER_CANCELLED",
        message: "Transcription was cancelled by the user.",
        recoverable: true
      }
    ] as const;

    const envelopes = [progress, result, ...errors].map((envelope) => JSON.parse(JSON.stringify(envelope)) as unknown);

    expect(envelopes.map(validateAsrCliEnvelope)).toEqual([
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] }
    ]);
    expect(envelopes[0]).toMatchObject({
      kind: "asr.progress",
      progress: 0.25,
      stage: "transcribing"
    });
    expect(envelopes[1]).toMatchObject({
      kind: "asr.result",
      segments: [
        {
          id: "seg-0001",
          startMs: 0,
          endMs: 2400,
          text: "我们先看一下 roadmap。",
          language: "mixed",
          confidence: 0.94
        }
      ]
    });
    expect(errors.map((error) => error.code)).toEqual([
      "ASR_MODEL_MISSING",
      "ASR_UNSUPPORTED_AUDIO_FORMAT",
      "ASR_PROCESS_FAILED",
      "ASR_USER_CANCELLED"
    ]);
  });
});
