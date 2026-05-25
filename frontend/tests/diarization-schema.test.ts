import { describe, expect, test } from "bun:test";
import {
  createDiarizationProgressEvent,
  createDiarizationResult,
  createDiarizationRunRequest,
  createMockDiarizationResult,
  validateDiarizationCliEnvelope
} from "../src/types/diarization-schema";

const SHA256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

describe("diarization schema", () => {
  test("creates deterministic run requests for pyannote speaker diarization", () => {
    expect(
      createDiarizationRunRequest({
        requestId: "dia-req-1",
        audioPath: "/audio/meeting.wav",
        audioFormat: ".wav",
        audioSha256: SHA256,
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
      })
    ).toMatchObject({
      kind: "diarization.run.request",
      schemaVersion: "meetnote.diarization.v1",
      audio: {
        format: ".wav",
        sha256: SHA256
      },
      model: {
        id: "pyannote-speaker-diarization-3.1",
        version: "3.1"
      },
      auth: {
        provider: "huggingface",
        state: "configured"
      }
    });
  });

  test("rejects unsupported formats, invalid checksums, and invalid auth state", () => {
    const result = validateDiarizationCliEnvelope({
      kind: "diarization.run.request",
      schemaVersion: "meetnote.diarization.v1",
      requestId: "dia-req-1",
      audio: {
        path: "/audio/meeting.aac",
        format: ".aac",
        sha256: "bad"
      },
      model: {
        id: "pyannote-speaker-diarization-3.1",
        version: "3.1",
        path: "/models/pyannote"
      },
      auth: {
        provider: "huggingface",
        state: "unknown"
      },
      options: {
        minSpeakers: 3,
        maxSpeakers: 1,
        collarMs: -1,
        overlapDetection: "yes"
      }
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toContain("DIARIZATION_AUDIO_FORMAT_UNSUPPORTED: audio.format must be .wav, .mp3, or .m4a");
    expect(result.errors).toContain("DIARIZATION_AUDIO_SHA256_INVALID: audio.sha256 must be 64 lowercase hex characters");
    expect(result.errors).toContain("DIARIZATION_AUTH_STATE_INVALID: auth.state is unsupported");
    expect(result.errors).toContain("DIARIZATION_SPEAKER_RANGE_INVALID: options.maxSpeakers cannot be less than minSpeakers");
  });

  test("calculates bounded progress from deterministic durations", () => {
    expect(
      createDiarizationProgressEvent({
        requestId: "dia-req-1",
        stage: "segmenting",
        processedMs: 15_000,
        totalMs: 60_000,
        message: "Segmenting speakers"
      }).progress
    ).toBe(0.25);
    expect(
      validateDiarizationCliEnvelope({
        kind: "diarization.progress",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "dia-req-1",
        stage: "segmenting",
        processedMs: 80_000,
        totalMs: 40_000,
        progress: 1,
        message: "Segmenting speakers"
      }).errors
    ).toContain("DIARIZATION_PROGRESS_RANGE_INVALID: processedMs cannot exceed totalMs");
  });

  test("validates speaker segments and model metadata in diarization results", () => {
    const valid = createDiarizationResult({
      requestId: "dia-req-1",
      model: {
        id: "pyannote-speaker-diarization-3.1",
        version: "3.1",
        parameters: {
          minSpeakers: 2,
          maxSpeakers: 4,
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
          id: "dia-seg-1",
          speakerId: "speaker-1",
          startMs: 0,
          endMs: 1800,
          confidence: 0.93,
          overlap: false
        }
      ]
    });

    expect(validateDiarizationCliEnvelope(valid)).toEqual({ ok: true, errors: [] });
    expect(
      validateDiarizationCliEnvelope({
        ...valid,
        segments: [
          {
            id: "dia-seg-1",
            speakerId: "speaker-missing",
            startMs: 1200,
            endMs: 1200,
            confidence: 2,
            overlap: "no"
          }
        ]
      }).errors
    ).toEqual([
      "DIARIZATION_SEGMENT_END_INVALID: segments[0].endMs must be greater than startMs",
      "DIARIZATION_SEGMENT_CONFIDENCE_INVALID: segments[0].confidence must be between 0 and 1",
      "DIARIZATION_SEGMENT_OVERLAP_INVALID: segments[0].overlap must be boolean",
      "DIARIZATION_SEGMENT_SPEAKER_UNKNOWN: segments[0].speakerId must reference a known speaker"
    ]);
  });

  test("rejects diarization result metadata when flattened fields contradict nested model metadata", () => {
    const valid = createDiarizationResult({
      requestId: "dia-req-1",
      model: {
        id: "pyannote-speaker-diarization-3.1",
        version: "3.1",
        parameters: {
          minSpeakers: 2,
          maxSpeakers: 4,
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
          id: "dia-seg-1",
          speakerId: "speaker-1",
          startMs: 0,
          endMs: 1800,
          confidence: 0.93,
          overlap: false
        }
      ]
    });

    expect(
      validateDiarizationCliEnvelope({
        ...valid,
        modelVersion: "3.0"
      }).errors
    ).toContain("DIARIZATION_MODEL_VERSION_MISMATCH: modelVersion must equal model.version");
    expect(
      validateDiarizationCliEnvelope({
        ...valid,
        parameters: {
          ...valid.parameters,
          collarMs: 500
        }
      }).errors
    ).toContain("DIARIZATION_MODEL_PARAMETERS_MISMATCH: parameters must equal model.parameters");
  });

  test("creates a success fixture with overlap and flattened runtime metadata", () => {
    const result = createMockDiarizationResult({
      requestId: "dia-mock",
      model: {
        id: "pyannote-speaker-diarization-3.1",
        version: "3.1",
        parameters: {
          minSpeakers: 2,
          maxSpeakers: 4,
          collarMs: 250,
          overlapDetection: true
        }
      },
      auth: {
        provider: "huggingface",
        state: "configured"
      }
    });

    expect(result).toMatchObject({
      kind: "diarization.result",
      schemaVersion: "meetnote.diarization.v1",
      requestId: "dia-mock",
      modelVersion: "3.1",
      parameters: {
        minSpeakers: 2,
        maxSpeakers: 4,
        collarMs: 250,
        overlapDetection: true
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
          endMs: 2480,
          confidence: 0.92,
          overlap: false
        },
        {
          id: "dia-seg-0002",
          speakerId: "speaker-2",
          startMs: 2200,
          endMs: 5200,
          confidence: 0.89,
          overlap: true
        }
      ]
    });
    expect(validateDiarizationCliEnvelope(result)).toEqual({ ok: true, errors: [] });
  });

  test("accepts only stable diarization error codes", () => {
    expect(
      validateDiarizationCliEnvelope({
        kind: "diarization.error",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "dia-req-1",
        code: "DIARIZATION_AUTH_REQUIRED",
        message: "Configure a Hugging Face token before running diarization.",
        recoverable: true
      })
    ).toEqual({ ok: true, errors: [] });
    expect(
      validateDiarizationCliEnvelope({
        kind: "diarization.error",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "dia-req-1",
        code: "RANDOM_FAILURE",
        message: "Unknown",
        recoverable: false
      }).errors
    ).toContain("DIARIZATION_ERROR_CODE_INVALID: code is unsupported");
  });

  test("rejects malformed envelopes across request, progress, result, and error shapes", () => {
    expect(validateDiarizationCliEnvelope(null)).toEqual({
      ok: false,
      errors: ["DIARIZATION_ENVELOPE_INVALID: envelope must be an object"]
    });
    expect(
      validateDiarizationCliEnvelope({
        kind: "unknown",
        schemaVersion: "wrong"
      }).errors
    ).toEqual([
      "DIARIZATION_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.diarization.v1",
      "DIARIZATION_KIND_INVALID: kind is unsupported"
    ]);
    expect(
      validateDiarizationCliEnvelope({
        kind: "diarization.run.request",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "",
        audio: null,
        model: null,
        auth: null,
        options: null
      }).errors
    ).toEqual([
      "DIARIZATION_REQUEST_ID_REQUIRED",
      "DIARIZATION_AUDIO_INVALID: audio must be an object",
      "DIARIZATION_MODEL_INVALID: model must be an object",
      "DIARIZATION_AUTH_INVALID: auth must be an object",
      "DIARIZATION_OPTIONS_INVALID: options must be an object"
    ]);
    expect(
      validateDiarizationCliEnvelope({
        kind: "diarization.progress",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "",
        stage: "unknown",
        processedMs: -1,
        totalMs: 0,
        progress: 2,
        message: ""
      }).errors
    ).toEqual([
      "DIARIZATION_REQUEST_ID_REQUIRED",
      "DIARIZATION_PROGRESS_STAGE_INVALID: stage is unsupported",
      "DIARIZATION_PROCESSED_MS_INVALID: processedMs must be non-negative",
      "DIARIZATION_TOTAL_MS_INVALID: totalMs must be positive",
      "DIARIZATION_PROGRESS_INVALID: progress must be between 0 and 1",
      "DIARIZATION_PROGRESS_MESSAGE_REQUIRED"
    ]);
    expect(
      validateDiarizationCliEnvelope({
        kind: "diarization.result",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "",
        modelVersion: "",
        parameters: null,
        model: {
          id: "",
          version: "",
          parameters: null
        },
        auth: null,
        speakers: [],
        segments: []
      }).errors
    ).toEqual([
      "DIARIZATION_REQUEST_ID_REQUIRED",
      "DIARIZATION_MODEL_VERSION_REQUIRED",
      "DIARIZATION_RESULT_PARAMETERS_INVALID: parameters must be an object",
      "DIARIZATION_MODEL_ID_REQUIRED",
      "DIARIZATION_MODEL_VERSION_REQUIRED",
      "DIARIZATION_MODEL_PARAMETERS_INVALID: model.parameters must be an object",
      "DIARIZATION_AUTH_INVALID: auth must be an object",
      "DIARIZATION_SPEAKERS_REQUIRED: speakers must be a non-empty array",
      "DIARIZATION_SEGMENTS_REQUIRED: segments must be a non-empty array"
    ]);
    expect(
      validateDiarizationCliEnvelope({
        kind: "diarization.error",
        schemaVersion: "meetnote.diarization.v1",
        requestId: "",
        code: "BAD",
        message: "",
        recoverable: "yes"
      }).errors
    ).toEqual([
      "DIARIZATION_REQUEST_ID_REQUIRED",
      "DIARIZATION_ERROR_CODE_INVALID: code is unsupported",
      "DIARIZATION_ERROR_MESSAGE_REQUIRED",
      "DIARIZATION_ERROR_RECOVERABLE_INVALID: recoverable must be boolean"
    ]);
  });
});
