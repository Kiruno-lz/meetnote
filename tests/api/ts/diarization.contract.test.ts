import { describe, expect, test } from "bun:test";
import {
  createDiarizationProgressEvent,
  createDiarizationResult,
  createDiarizationRunRequest,
  validateDiarizationCliEnvelope
} from "../../../frontend/src/diarization-schema";

describe("diarization CLI contract", () => {
  test("round-trips a pyannote diarization request through JSON serialization", () => {
    const request = createDiarizationRunRequest({
      requestId: "dia-req-001",
      audioPath: "/meetings/product-review.zh-en.m4a",
      audioFormat: ".m4a",
      audioSha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      model: {
        id: "pyannote-speaker-diarization-3.1",
        version: "3.1",
        path: "/models/pyannote-speaker-diarization-3.1"
      },
      auth: {
        provider: "huggingface",
        state: "configured"
      },
      options: {
        minSpeakers: 2,
        maxSpeakers: 6,
        collarMs: 250,
        overlapDetection: true
      }
    });

    const parsed = JSON.parse(JSON.stringify(request)) as unknown;

    expect(validateDiarizationCliEnvelope(parsed)).toEqual({ ok: true, errors: [] });
    expect(parsed).toEqual({
      kind: "diarization.run.request",
      schemaVersion: "meetnote.diarization.v1",
      requestId: "dia-req-001",
      audio: {
        path: "/meetings/product-review.zh-en.m4a",
        format: ".m4a",
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
      },
      model: {
        id: "pyannote-speaker-diarization-3.1",
        version: "3.1",
        path: "/models/pyannote-speaker-diarization-3.1"
      },
      auth: {
        provider: "huggingface",
        state: "configured"
      },
      options: {
        minSpeakers: 2,
        maxSpeakers: 6,
        collarMs: 250,
        overlapDetection: true
      }
    });
  });

  test("round-trips progress, result, and stable errors through JSON serialization", () => {
    const progress = createDiarizationProgressEvent({
      requestId: "dia-req-001",
      stage: "clustering",
      processedMs: 30_000,
      totalMs: 120_000,
      message: "Clustering speaker embeddings"
    });
    const result = createDiarizationResult({
      requestId: "dia-req-001",
      model: {
        id: "pyannote-speaker-diarization-3.1",
        version: "3.1",
        parameters: {
          minSpeakers: 2,
          maxSpeakers: 6,
          collarMs: 250,
          overlapDetection: true
        }
      },
      auth: {
        provider: "huggingface",
        state: "configured"
      },
      speakers: [
        { id: "speaker-1", label: "Speaker 1" },
        { id: "speaker-2", label: "Speaker 2" }
      ],
      segments: [
        {
          id: "dia-seg-0001",
          speakerId: "speaker-1",
          startMs: 0,
          endMs: 2400,
          confidence: 0.94,
          overlap: false
        },
        {
          id: "dia-seg-0002",
          speakerId: "speaker-2",
          startMs: 2200,
          endMs: 3600,
          confidence: 0.88,
          overlap: true
        }
      ]
    });
    const errors = [
      {
        kind: "diarization.error",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "dia-req-001",
        code: "DIARIZATION_MODEL_MISSING",
        message: "Select or download a diarization model before speaker detection.",
        recoverable: true
      },
      {
        kind: "diarization.error",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "dia-req-001",
        code: "DIARIZATION_AUTH_REQUIRED",
        message: "Configure a Hugging Face token before running pyannote.",
        recoverable: true
      },
      {
        kind: "diarization.error",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "dia-req-001",
        code: "DIARIZATION_PROCESS_FAILED",
        message: "The diarization subprocess exited with a non-zero status.",
        recoverable: false
      },
      {
        kind: "diarization.error",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "dia-req-001",
        code: "DIARIZATION_USER_CANCELLED",
        message: "Speaker diarization was cancelled by the user.",
        recoverable: true
      }
    ] as const;

    const envelopes = [progress, result, ...errors].map((envelope) => JSON.parse(JSON.stringify(envelope)) as unknown);

    expect(envelopes.map(validateDiarizationCliEnvelope)).toEqual([
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] },
      { ok: true, errors: [] }
    ]);
    expect(envelopes[0]).toMatchObject({
      kind: "diarization.progress",
      progress: 0.25,
      stage: "clustering"
    });
    expect(envelopes[1]).toMatchObject({
      kind: "diarization.result",
      modelVersion: "3.1",
      parameters: {
        minSpeakers: 2,
        maxSpeakers: 6,
        collarMs: 250,
        overlapDetection: true
      },
      segments: [
        {
          id: "dia-seg-0001",
          speakerId: "speaker-1",
          startMs: 0,
          endMs: 2400,
          confidence: 0.94,
          overlap: false
        },
        {
          id: "dia-seg-0002",
          speakerId: "speaker-2",
          startMs: 2200,
          endMs: 3600,
          confidence: 0.88,
          overlap: true
        }
      ]
    });
    expect(errors.map((error) => error.code)).toEqual([
      "DIARIZATION_MODEL_MISSING",
      "DIARIZATION_AUTH_REQUIRED",
      "DIARIZATION_PROCESS_FAILED",
      "DIARIZATION_USER_CANCELLED"
    ]);
  });

  test("rejects JSON result envelopes with contradictory flattened and nested metadata", () => {
    const result = createDiarizationResult({
      requestId: "dia-req-001",
      model: {
        id: "pyannote-speaker-diarization-3.1",
        version: "3.1",
        parameters: {
          minSpeakers: 2,
          maxSpeakers: 6,
          collarMs: 250,
          overlapDetection: true
        }
      },
      auth: {
        provider: "huggingface",
        state: "configured"
      },
      speakers: [{ id: "speaker-1", label: "Speaker 1" }],
      segments: [
        {
          id: "dia-seg-0001",
          speakerId: "speaker-1",
          startMs: 0,
          endMs: 2400,
          confidence: 0.94,
          overlap: false
        }
      ]
    });

    const mismatched = JSON.parse(
      JSON.stringify({
        ...result,
        modelVersion: "3.0",
        parameters: {
          ...result.parameters,
          overlapDetection: false
        }
      })
    ) as unknown;

    expect(validateDiarizationCliEnvelope(mismatched)).toEqual({
      ok: false,
      errors: [
        "DIARIZATION_MODEL_VERSION_MISMATCH: modelVersion must equal model.version",
        "DIARIZATION_MODEL_PARAMETERS_MISMATCH: parameters must equal model.parameters"
      ]
    });
  });
});
