import { describe, expect, test } from "bun:test";
import {
  MODEL_MATRIX,
  MODEL_OPERATION_ERRORS,
  createDownloadState,
  createPyannoteAuthError,
  validateModelDownloadState,
  validateModelManifest
} from "../src/types/model-manifest";

describe("model matrix", () => {
  test("keeps Whisper large-v3 as the quality-first ASR candidate", () => {
    const whisper = MODEL_MATRIX.find((model) => model.id === "whisper-large-v3");

    expect(whisper).toMatchObject({
      kind: "asr",
      defaultUseCase: "quality-first",
      languageProfile: "zh-primary-en-mixed",
      format: "external",
      source: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
      license: "apache-2.0"
    });
    expect(whisper?.hardware.minimumMemoryGb).toBeGreaterThanOrEqual(10);
    expect(whisper?.downloadSizeBytes).toBeGreaterThan(0);
    expect(whisper?.checksum).toMatch(/^sha256:/);
  });

  test("marks gated pyannote assets as requiring authorization before size pinning", () => {
    const pyannote = MODEL_MATRIX.find((model) => model.id === "pyannote-speaker-diarization-3.1");

    expect(pyannote).toMatchObject({
      kind: "diarization",
      source: "pyannote/speaker-diarization-3.1",
      license: "mit-gated-hugging-face",
      downloadSizeBytes: null
    });
  });

  test("keeps the built-in matrix as candidates until checksums are pinned", () => {
    const result = validateModelManifest({
      schemaVersion: "meetnote.model-manifest.v1",
      models: MODEL_MATRIX
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toContain("MODEL_CHECKSUM_REQUIRED: models[0].checksum must be sha256:<hex>");
    expect(result.errors).toContain("MODEL_CHECKSUM_REQUIRED: models[1].checksum must be sha256:<hex>");
  });
});

describe("model manifest schema", () => {
  test("accepts a complete external model manifest", () => {
    const result = validateModelManifest({
      schemaVersion: "meetnote.model-manifest.v1",
      models: [
        {
          ...MODEL_MATRIX[0],
          checksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
      ]
    });

    expect(result).toEqual({ ok: true, errors: [] });
  });

  test("rejects missing checksum with an explicit error code", () => {
    const result = validateModelManifest({
      schemaVersion: "meetnote.model-manifest.v1",
      models: [
        {
          ...MODEL_MATRIX[0],
          checksum: ""
        }
      ]
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toContain("MODEL_CHECKSUM_REQUIRED: models[0].checksum must be sha256:<hex>");
  });

  test("rejects malformed manifests and model metadata deterministically", () => {
    expect(validateModelManifest(null)).toEqual({
      ok: false,
      errors: ["MODEL_MANIFEST_INVALID: manifest must be an object"]
    });
    expect(
      validateModelManifest({
        schemaVersion: "meetnote.model-manifest.v1",
        models: []
      }).errors
    ).toContain("MODEL_LIST_REQUIRED: models must be a non-empty array");

    const result = validateModelManifest({
      schemaVersion: "wrong",
      models: [
        {
          id: "",
          kind: "tts",
          defaultUseCase: "fast",
          languageProfile: "unknown",
          format: "bundled",
          source: "",
          checksum: "sha256:not-hex",
          license: "",
          downloadSizeBytes: 0,
          hardware: {
            minimumMemoryGb: 0,
            recommendedAcceleration: "quantum"
          }
        }
      ]
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toContain("MODEL_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.model-manifest.v1");
    expect(result.errors).toContain("MODEL_ID_REQUIRED: models[0].id must be a non-empty string");
    expect(result.errors).toContain("MODEL_KIND_INVALID: models[0].kind must be asr or diarization");
    expect(result.errors).toContain("MODEL_DOWNLOAD_SIZE_INVALID: models[0].downloadSizeBytes must be positive or null while gated");
    expect(result.errors).toContain("MODEL_MEMORY_INVALID: models[0].hardware.minimumMemoryGb must be positive");
    expect(result.errors).toContain("MODEL_ACCELERATION_INVALID: models[0].hardware.recommendedAcceleration is unsupported");
  });

  test("rejects non-object model entries and missing hardware blocks", () => {
    expect(
      validateModelManifest({
        schemaVersion: "meetnote.model-manifest.v1",
        models: ["whisper-large-v3"]
      }).errors
    ).toContain("MODEL_ENTRY_INVALID: models[0] must be an object");

    expect(
      validateModelManifest({
        schemaVersion: "meetnote.model-manifest.v1",
        models: [
          {
            ...MODEL_MATRIX[0],
            checksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            hardware: null
          }
        ]
      }).errors
    ).toContain("MODEL_HARDWARE_REQUIRED: models[0].hardware must be an object");
  });

  test("allows null download size only for gated model entries", () => {
    const result = validateModelManifest({
      schemaVersion: "meetnote.model-manifest.v1",
      models: [
        {
          ...MODEL_MATRIX[0],
          checksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
          downloadSizeBytes: null
        }
      ]
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toContain("MODEL_DOWNLOAD_SIZE_INVALID: models[0].downloadSizeBytes must be positive or null while gated");
  });
});

describe("model download state JSON", () => {
  test("serializes the cache path, version, checksum, progress, and resumable bytes", () => {
    const state = createDownloadState({
      modelId: "whisper-large-v3",
      version: "large-v3",
      cachePath: "/Users/me/Library/Application Support/meetnote/models/whisper-large-v3",
      expectedChecksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      downloadedBytes: 1024,
      totalBytes: 4096,
      status: "downloading"
    });

    expect(JSON.parse(JSON.stringify(state))).toEqual({
      schemaVersion: "meetnote.model-download-state.v1",
      modelId: "whisper-large-v3",
      version: "large-v3",
      cachePath: "/Users/me/Library/Application Support/meetnote/models/whisper-large-v3",
      expectedChecksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      downloadedBytes: 1024,
      totalBytes: 4096,
      progress: 0.25,
      status: "downloading",
      error: null
    });
  });

  test("computes download progress at valid lower and upper bounds", () => {
    expect(
      createDownloadState({
        modelId: "whisper-large-v3",
        version: "large-v3",
        cachePath: "/models/whisper-large-v3",
        expectedChecksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        downloadedBytes: 0,
        totalBytes: 100,
        status: "idle"
      }).progress
    ).toBe(0);

    expect(
      createDownloadState({
        modelId: "whisper-large-v3",
        version: "large-v3",
        cachePath: "/models/whisper-large-v3",
        expectedChecksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        downloadedBytes: 100,
        totalBytes: 100,
        status: "verified"
      }).progress
    ).toBe(1);
  });

  test("factory output passes download state validation", () => {
    const state = createDownloadState({
      modelId: "pyannote-speaker-diarization-3.1",
      version: "3.1",
      cachePath: "/models/pyannote-speaker-diarization-3.1",
      expectedChecksum: "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
      downloadedBytes: 0,
      totalBytes: 1,
      status: "idle"
    });

    expect(validateModelDownloadState(state)).toEqual({ ok: true, errors: [] });
  });

  test("factory rejects invalid state before serialization", () => {
    expect(() =>
      createDownloadState({
        modelId: "whisper-large-v3",
        version: "large-v3",
        cachePath: "/models/whisper-large-v3",
        expectedChecksum: "sha256:not-hex",
        downloadedBytes: 10,
        totalBytes: 0,
        status: "idle"
      })
    ).toThrow("MODEL_EXPECTED_CHECKSUM_INVALID: expectedChecksum must be sha256:<hex>");
  });

  test("rejects malformed download state with stable error codes", () => {
    expect(validateModelDownloadState("bad-state")).toEqual({
      ok: false,
      errors: ["MODEL_DOWNLOAD_STATE_INVALID: state must be an object"]
    });

    const result = validateModelDownloadState({
      schemaVersion: "wrong",
      modelId: "",
      version: "",
      cachePath: "",
      expectedChecksum: "sha256:not-hex",
      downloadedBytes: 4096,
      totalBytes: 0,
      progress: 4,
      status: "done",
      error: {
        code: "UNKNOWN",
        message: "",
        recoverable: "yes"
      }
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toContain("MODEL_DOWNLOAD_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.model-download-state.v1");
    expect(result.errors).toContain("MODEL_ID_REQUIRED: modelId must be a non-empty string");
    expect(result.errors).toContain("MODEL_VERSION_REQUIRED: version must be a non-empty string");
    expect(result.errors).toContain("MODEL_CACHE_PATH_REQUIRED: cachePath must be a non-empty string");
    expect(result.errors).toContain("MODEL_EXPECTED_CHECKSUM_INVALID: expectedChecksum must be sha256:<hex>");
    expect(result.errors).toContain("MODEL_DOWNLOAD_TOTAL_BYTES_INVALID: totalBytes must be positive");
    expect(result.errors).toContain("MODEL_DOWNLOAD_BYTES_INVALID: downloadedBytes must be between 0 and totalBytes");
    expect(result.errors).toContain("MODEL_DOWNLOAD_PROGRESS_INVALID: progress must be between 0 and 1");
    expect(result.errors).toContain("MODEL_DOWNLOAD_STATUS_INVALID: status is unsupported");
    expect(result.errors).toContain("MODEL_DOWNLOAD_ERROR_CODE_INVALID: error.code is unsupported");
    expect(result.errors).toContain("MODEL_DOWNLOAD_ERROR_MESSAGE_REQUIRED: error.message must be a non-empty string");
    expect(result.errors).toContain("MODEL_DOWNLOAD_ERROR_RECOVERABLE_INVALID: error.recoverable must be boolean");
    expect(
      validateModelDownloadState({
        schemaVersion: "meetnote.model-download-state.v1",
        modelId: "whisper-large-v3",
        version: "large-v3",
        cachePath: "/models/whisper-large-v3",
        expectedChecksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        downloadedBytes: 0,
        totalBytes: 1024,
        progress: 0,
        status: "failed",
        error: "failed"
      }).errors
    ).toContain("MODEL_DOWNLOAD_ERROR_INVALID: error must be null or an object");
  });
});

describe("pyannote config and error contract", () => {
  test("returns a stable authentication error without exposing tokens", () => {
    const error = createPyannoteAuthError("hf_secret_token");

    expect(error).toEqual({
      code: "PYANNOTE_AUTH_REQUIRED",
      message: "Configure a Hugging Face token before downloading pyannote/speaker-diarization-3.1.",
      modelId: "pyannote-speaker-diarization-3.1",
      recoverable: true
    });
    expect(JSON.stringify(error)).not.toContain("hf_secret_token");
  });
});

describe("model operation errors", () => {
  test("covers missing model, checksum failure, permission denial, and pyannote auth", () => {
    expect(MODEL_OPERATION_ERRORS).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ code: "MODEL_MISSING", recoverable: true }),
        expect.objectContaining({ code: "MODEL_CHECKSUM_FAILED", recoverable: true }),
        expect.objectContaining({ code: "MODEL_CACHE_PERMISSION_DENIED", recoverable: false }),
        expect.objectContaining({ code: "PYANNOTE_AUTH_REQUIRED", recoverable: true })
      ])
    );
  });
});
