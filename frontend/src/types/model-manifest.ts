export type ModelKind = "asr" | "diarization";
export type ModelFormat = "external" | "local-path";
export type ModelUseCase = "quality-first" | "speaker-diarization";
export type ModelLanguageProfile = "zh-primary-en-mixed" | "speaker-agnostic";
export type ModelBackend = "whisper" | "paraformer" | "qwen3-asr" | "mimo";
export type ModelDownloadMethod = "http" | "library-managed";

export interface ModelMatrixEntry {
  readonly id: string;
  readonly kind: ModelKind;
  readonly defaultUseCase: ModelUseCase;
  readonly languageProfile: ModelLanguageProfile;
  readonly backend: ModelBackend;
  readonly downloadMethod: ModelDownloadMethod;
  readonly libraryDependencies?: readonly string[];
  readonly hardware: {
    readonly minimumMemoryGb: number;
    readonly recommendedAcceleration: "cpu" | "gpu" | "apple-silicon";
    readonly requiresGpu?: boolean;
  };
  readonly format: ModelFormat;
  readonly source: string;
  readonly checksum: string;
  readonly license: string;
  readonly downloadSizeBytes: number | null;
}

export interface ModelManifest {
  readonly schemaVersion: "meetnote.model-manifest.v1";
  readonly models: readonly ModelMatrixEntry[];
}

export interface ValidationResult {
  readonly ok: boolean;
  readonly errors: readonly string[];
}

export type ModelDownloadStatus = "idle" | "downloading" | "verified" | "failed";

export interface ModelDownloadStateInput {
  readonly modelId: string;
  readonly version: string;
  readonly cachePath: string;
  readonly expectedChecksum: string;
  readonly downloadedBytes: number;
  readonly totalBytes: number;
  readonly status: ModelDownloadStatus;
  readonly error?: ModelDownloadError | null;
}

export interface ModelDownloadError {
  readonly code: ModelOperationErrorCode;
  readonly message: string;
  readonly recoverable: boolean;
}

export interface ModelDownloadState extends ModelDownloadStateInput {
  readonly schemaVersion: "meetnote.model-download-state.v1";
  readonly progress: number;
  readonly error: ModelDownloadError | null;
}

export interface PyannoteError {
  readonly code: "PYANNOTE_AUTH_REQUIRED";
  readonly message: string;
  readonly modelId: "pyannote-speaker-diarization-3.1";
  readonly recoverable: true;
}

export type ModelOperationErrorCode =
  | "MODEL_MISSING"
  | "MODEL_CHECKSUM_FAILED"
  | "MODEL_CACHE_PERMISSION_DENIED"
  | "PYANNOTE_AUTH_REQUIRED";

export interface ModelOperationError {
  readonly code: ModelOperationErrorCode;
  readonly message: string;
  readonly recoverable: boolean;
}

const SHA256_PATTERN = /^sha256:[0-9a-f]{64}$/;
const DOWNLOAD_STATUSES: readonly ModelDownloadStatus[] = ["idle", "downloading", "verified", "failed"] as const;

export const MODEL_MATRIX: readonly ModelMatrixEntry[] = [
  {
    id: "whisper-large-v3",
    kind: "asr",
    defaultUseCase: "quality-first",
    languageProfile: "zh-primary-en-mixed",
    backend: "whisper",
    downloadMethod: "http",
    hardware: { minimumMemoryGb: 10, recommendedAcceleration: "apple-silicon" },
    format: "external",
    source: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
    checksum: "sha256:<required-at-download>",
    license: "apache-2.0",
    downloadSizeBytes: 3_100_000_000
  },
  {
    id: "whisper-large-v3-turbo",
    kind: "asr",
    defaultUseCase: "quality-first",
    languageProfile: "zh-primary-en-mixed",
    backend: "whisper",
    downloadMethod: "http",
    hardware: { minimumMemoryGb: 3, recommendedAcceleration: "apple-silicon" },
    format: "external",
    source: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
    checksum: "sha256:<required-at-download>",
    license: "apache-2.0",
    downloadSizeBytes: 874_000_000
  },
  {
    id: "whisper-large-v3-turbo-q5_0",
    kind: "asr",
    defaultUseCase: "quality-first",
    languageProfile: "zh-primary-en-mixed",
    backend: "whisper",
    downloadMethod: "http",
    hardware: { minimumMemoryGb: 2, recommendedAcceleration: "apple-silicon" },
    format: "external",
    source: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
    checksum: "sha256:<required-at-download>",
    license: "apache-2.0",
    downloadSizeBytes: 547_000_000
  },
  {
    id: "whisper-medium",
    kind: "asr",
    defaultUseCase: "quality-first",
    languageProfile: "zh-primary-en-mixed",
    backend: "whisper",
    downloadMethod: "http",
    hardware: { minimumMemoryGb: 5, recommendedAcceleration: "cpu" },
    format: "external",
    source: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
    checksum: "sha256:<required-at-download>",
    license: "apache-2.0",
    downloadSizeBytes: 1_528_000_000
  },
  {
    id: "whisper-small",
    kind: "asr",
    defaultUseCase: "quality-first",
    languageProfile: "zh-primary-en-mixed",
    backend: "whisper",
    downloadMethod: "http",
    hardware: { minimumMemoryGb: 2, recommendedAcceleration: "cpu" },
    format: "external",
    source: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
    checksum: "sha256:<required-at-download>",
    license: "apache-2.0",
    downloadSizeBytes: 488_000_000
  },
  {
    id: "whisper-base",
    kind: "asr",
    defaultUseCase: "quality-first",
    languageProfile: "zh-primary-en-mixed",
    backend: "whisper",
    downloadMethod: "http",
    hardware: { minimumMemoryGb: 1, recommendedAcceleration: "cpu" },
    format: "external",
    source: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
    checksum: "sha256:<required-at-download>",
    license: "apache-2.0",
    downloadSizeBytes: 148_000_000
  },
  {
    id: "whisper-tiny",
    kind: "asr",
    defaultUseCase: "quality-first",
    languageProfile: "zh-primary-en-mixed",
    backend: "whisper",
    downloadMethod: "http",
    hardware: { minimumMemoryGb: 1, recommendedAcceleration: "cpu" },
    format: "external",
    source: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
    checksum: "sha256:<required-at-download>",
    license: "apache-2.0",
    downloadSizeBytes: 77_000_000
  },
  {
    id: "paraformer-zh",
    kind: "asr",
    defaultUseCase: "quality-first",
    languageProfile: "zh-primary-en-mixed",
    backend: "paraformer",
    downloadMethod: "library-managed",
    libraryDependencies: ["funasr"],
    hardware: { minimumMemoryGb: 4, recommendedAcceleration: "cpu" },
    format: "external",
    source: "funasr/paraformer-zh",
    checksum: "sha256:<required-at-download>",
    license: "apache-2.0",
    downloadSizeBytes: 440_000_000
  },
  {
    id: "qwen3-asr-1.7b",
    kind: "asr",
    defaultUseCase: "quality-first",
    languageProfile: "zh-primary-en-mixed",
    backend: "qwen3-asr",
    downloadMethod: "library-managed",
    libraryDependencies: ["transformers", "torch"],
    hardware: { minimumMemoryGb: 8, recommendedAcceleration: "gpu" },
    format: "external",
    source: "Qwen/Qwen3-ASR-1.7B",
    checksum: "sha256:<required-at-download>",
    license: "apache-2.0",
    downloadSizeBytes: 3_400_000_000
  },
  {
    id: "mimo-v2.5-asr",
    kind: "asr",
    defaultUseCase: "quality-first",
    languageProfile: "zh-primary-en-mixed",
    backend: "mimo",
    downloadMethod: "library-managed",
    libraryDependencies: ["transformers", "torch"],
    hardware: { minimumMemoryGb: 20, recommendedAcceleration: "gpu", requiresGpu: true },
    format: "external",
    source: "XiaomiMiMo/MiMo-V2.5-ASR",
    checksum: "sha256:<required-at-download>",
    license: "apache-2.0",
    downloadSizeBytes: 16_000_000_000
  },
  {
    id: "pyannote-speaker-diarization-3.1",
    kind: "diarization",
    defaultUseCase: "speaker-diarization",
    languageProfile: "speaker-agnostic",
    backend: "whisper",
    downloadMethod: "library-managed",
    libraryDependencies: ["pyannote.audio"],
    hardware: {
      minimumMemoryGb: 8,
      recommendedAcceleration: "gpu"
    },
    format: "external",
    source: "pyannote/speaker-diarization-3.1",
    checksum: "sha256:<required-at-download>",
    license: "mit-gated-hugging-face",
    downloadSizeBytes: null
  }
] as const;

export const MODEL_OPERATION_ERRORS: readonly ModelOperationError[] = [
  {
    code: "MODEL_MISSING",
    message: "Download the model or select a local model path before running this step.",
    recoverable: true
  },
  {
    code: "MODEL_CHECKSUM_FAILED",
    message: "The downloaded model did not match the expected checksum. Delete it and download again.",
    recoverable: true
  },
  {
    code: "MODEL_CACHE_PERMISSION_DENIED",
    message: "meetnote cannot write to the configured model cache directory.",
    recoverable: false
  },
  {
    code: "PYANNOTE_AUTH_REQUIRED",
    message: "Configure a Hugging Face token before downloading pyannote/speaker-diarization-3.1.",
    recoverable: true
  }
] as const;

export function validateModelManifest(value: unknown): ValidationResult {
  const errors: string[] = [];

  if (!isRecord(value)) {
    return { ok: false, errors: ["MODEL_MANIFEST_INVALID: manifest must be an object"] };
  }

  if (value.schemaVersion !== "meetnote.model-manifest.v1") {
    errors.push("MODEL_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.model-manifest.v1");
  }

  if (!Array.isArray(value.models) || value.models.length === 0) {
    errors.push("MODEL_LIST_REQUIRED: models must be a non-empty array");
  } else {
    value.models.forEach((model, index) => {
      validateModelEntry(model, index, errors);
    });
  }

  return { ok: errors.length === 0, errors };
}

export function createDownloadState(input: ModelDownloadStateInput): ModelDownloadState {
  const state: ModelDownloadState = {
    ...input,
    schemaVersion: "meetnote.model-download-state.v1",
    progress: calculateProgress(input.downloadedBytes, input.totalBytes),
    error: input.error ?? null
  };

  const result = validateModelDownloadState(state);
  if (!result.ok) {
    throw new Error(result.errors.join("; "));
  }

  return state;
}

export function validateModelDownloadState(value: unknown): ValidationResult {
  const errors: string[] = [];

  if (!isRecord(value)) {
    return { ok: false, errors: ["MODEL_DOWNLOAD_STATE_INVALID: state must be an object"] };
  }

  if (value.schemaVersion !== "meetnote.model-download-state.v1") {
    errors.push("MODEL_DOWNLOAD_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.model-download-state.v1");
  }

  requireNonEmptyString(value.modelId, "modelId", "MODEL_ID_REQUIRED", errors);
  requireNonEmptyString(value.version, "version", "MODEL_VERSION_REQUIRED", errors);
  requireNonEmptyString(value.cachePath, "cachePath", "MODEL_CACHE_PATH_REQUIRED", errors);

  if (typeof value.expectedChecksum !== "string" || !SHA256_PATTERN.test(value.expectedChecksum)) {
    errors.push("MODEL_EXPECTED_CHECKSUM_INVALID: expectedChecksum must be sha256:<hex>");
  }

  if (
    typeof value.totalBytes !== "number" ||
    !Number.isFinite(value.totalBytes) ||
    value.totalBytes <= 0
  ) {
    errors.push("MODEL_DOWNLOAD_TOTAL_BYTES_INVALID: totalBytes must be positive");
  }

  if (
    typeof value.downloadedBytes !== "number" ||
    !Number.isFinite(value.downloadedBytes) ||
    value.downloadedBytes < 0 ||
    (typeof value.totalBytes === "number" && value.downloadedBytes > value.totalBytes)
  ) {
    errors.push("MODEL_DOWNLOAD_BYTES_INVALID: downloadedBytes must be between 0 and totalBytes");
  }

  if (typeof value.progress !== "number" || !Number.isFinite(value.progress) || value.progress < 0 || value.progress > 1) {
    errors.push("MODEL_DOWNLOAD_PROGRESS_INVALID: progress must be between 0 and 1");
  }

  if (typeof value.status !== "string" || !DOWNLOAD_STATUSES.includes(value.status as ModelDownloadStatus)) {
    errors.push("MODEL_DOWNLOAD_STATUS_INVALID: status is unsupported");
  }

  if (value.error !== null && value.error !== undefined) {
    validateDownloadError(value.error, errors);
  }

  return { ok: errors.length === 0, errors };
}

export function createPyannoteAuthError(_token?: string): PyannoteError {
  return {
    code: "PYANNOTE_AUTH_REQUIRED",
    message: "Configure a Hugging Face token before downloading pyannote/speaker-diarization-3.1.",
    modelId: "pyannote-speaker-diarization-3.1",
    recoverable: true
  };
}

function validateDownloadError(value: unknown, errors: string[]): void {
  if (!isRecord(value)) {
    errors.push("MODEL_DOWNLOAD_ERROR_INVALID: error must be null or an object");
    return;
  }

  if (
    value.code !== "MODEL_MISSING" &&
    value.code !== "MODEL_CHECKSUM_FAILED" &&
    value.code !== "MODEL_CACHE_PERMISSION_DENIED" &&
    value.code !== "PYANNOTE_AUTH_REQUIRED"
  ) {
    errors.push("MODEL_DOWNLOAD_ERROR_CODE_INVALID: error.code is unsupported");
  }

  requireNonEmptyString(value.message, "error.message", "MODEL_DOWNLOAD_ERROR_MESSAGE_REQUIRED", errors);

  if (typeof value.recoverable !== "boolean") {
    errors.push("MODEL_DOWNLOAD_ERROR_RECOVERABLE_INVALID: error.recoverable must be boolean");
  }
}

function validateModelEntry(value: unknown, index: number, errors: string[]): void {
  if (!isRecord(value)) {
    errors.push(`MODEL_ENTRY_INVALID: models[${index}] must be an object`);
    return;
  }

  requireNonEmptyString(value.id, `models[${index}].id`, "MODEL_ID_REQUIRED", errors);

  if (value.kind !== "asr" && value.kind !== "diarization") {
    errors.push(`MODEL_KIND_INVALID: models[${index}].kind must be asr or diarization`);
  }

  if (value.defaultUseCase !== "quality-first" && value.defaultUseCase !== "speaker-diarization") {
    errors.push(`MODEL_USE_CASE_INVALID: models[${index}].defaultUseCase is unsupported`);
  }

  if (value.languageProfile !== "zh-primary-en-mixed" && value.languageProfile !== "speaker-agnostic") {
    errors.push(`MODEL_LANGUAGE_PROFILE_INVALID: models[${index}].languageProfile is unsupported`);
  }

  if (value.format !== "external" && value.format !== "local-path") {
    errors.push(`MODEL_FORMAT_INVALID: models[${index}].format must be external or local-path`);
  }

  if (
    value.backend !== "whisper" &&
    value.backend !== "paraformer" &&
    value.backend !== "qwen3-asr" &&
    value.backend !== "mimo"
  ) {
    errors.push(`MODEL_BACKEND_INVALID: models[${index}].backend is unsupported`);
  }

  if (value.downloadMethod !== "http" && value.downloadMethod !== "library-managed") {
    errors.push(`MODEL_DOWNLOAD_METHOD_INVALID: models[${index}].downloadMethod must be http or library-managed`);
  }

  requireNonEmptyString(value.source, `models[${index}].source`, "MODEL_SOURCE_REQUIRED", errors);
  requireNonEmptyString(value.license, `models[${index}].license`, "MODEL_LICENSE_REQUIRED", errors);

  if (typeof value.checksum !== "string" || !SHA256_PATTERN.test(value.checksum)) {
    errors.push(`MODEL_CHECKSUM_REQUIRED: models[${index}].checksum must be sha256:<hex>`);
  }

  if (
    value.downloadSizeBytes !== null &&
    (typeof value.downloadSizeBytes !== "number" || !Number.isFinite(value.downloadSizeBytes) || value.downloadSizeBytes <= 0)
  ) {
    errors.push(`MODEL_DOWNLOAD_SIZE_INVALID: models[${index}].downloadSizeBytes must be positive or null while gated`);
  }

  if (value.downloadSizeBytes === null && typeof value.license === "string" && !value.license.includes("gated")) {
    errors.push(`MODEL_DOWNLOAD_SIZE_INVALID: models[${index}].downloadSizeBytes must be positive or null while gated`);
  }

  if (!isRecord(value.hardware)) {
    errors.push(`MODEL_HARDWARE_REQUIRED: models[${index}].hardware must be an object`);
    return;
  }

  if (
    typeof value.hardware.minimumMemoryGb !== "number" ||
    !Number.isFinite(value.hardware.minimumMemoryGb) ||
    value.hardware.minimumMemoryGb <= 0
  ) {
    errors.push(`MODEL_MEMORY_INVALID: models[${index}].hardware.minimumMemoryGb must be positive`);
  }

  if (
    value.hardware.recommendedAcceleration !== "cpu" &&
    value.hardware.recommendedAcceleration !== "gpu" &&
    value.hardware.recommendedAcceleration !== "apple-silicon"
  ) {
    errors.push(`MODEL_ACCELERATION_INVALID: models[${index}].hardware.recommendedAcceleration is unsupported`);
  }
}

function requireNonEmptyString(value: unknown, path: string, code: string, errors: string[]): void {
  if (typeof value !== "string" || value.trim() === "") {
    errors.push(`${code}: ${path} must be a non-empty string`);
  }
}

function calculateProgress(downloadedBytes: number, totalBytes: number): number {
  if (totalBytes <= 0) {
    return 0;
  }

  return Math.min(1, Math.max(0, downloadedBytes / totalBytes));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
