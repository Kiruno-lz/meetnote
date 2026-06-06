import { describe, expect, test } from "bun:test";
import {
  createDownloadState,
  createPyannoteAuthError,
  validateModelDownloadState
} from "../../../frontend/src/types/model-manifest";

describe("model download contract", () => {
  test("keeps download state stable across JSON boundaries", () => {
    const state = createDownloadState({
      modelId: "whisper-large-v3",
      version: "large-v3",
      cachePath: "/models/whisper-large-v3/large-v3",
      expectedChecksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      downloadedBytes: 512,
      totalBytes: 2048,
      status: "downloading"
    });

    const parsed = JSON.parse(JSON.stringify(state)) as unknown;

    expect(validateModelDownloadState(parsed)).toEqual({ ok: true, errors: [] });
    expect(parsed).toEqual({
      schemaVersion: "meetnote.model-download-state.v1",
      modelId: "whisper-large-v3",
      version: "large-v3",
      cachePath: "/models/whisper-large-v3/large-v3",
      expectedChecksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      downloadedBytes: 512,
      totalBytes: 2048,
      progress: 0.25,
      status: "downloading",
      error: null
    });
  });

  test("does not leak pyannote tokens through auth errors", () => {
    const parsed = JSON.parse(JSON.stringify(createPyannoteAuthError("hf_secret_token"))) as unknown;

    expect(parsed).toEqual({
      code: "PYANNOTE_AUTH_REQUIRED",
      message: "Configure a Hugging Face token before downloading pyannote/speaker-diarization-3.1.",
      modelId: "pyannote-speaker-diarization-3.1",
      recoverable: true
    });
    expect(JSON.stringify(parsed)).not.toContain("hf_secret_token");
  });
});
