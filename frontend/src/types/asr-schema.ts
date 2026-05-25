import type { SupportedAudioExtension } from "./audio-project";
import { SUPPORTED_AUDIO_EXTENSIONS } from "./audio-project";

export const ASR_SCHEMA_VERSION = "meetnote.asr.v1" as const;

export type AsrLanguageHint = "zh-primary" | "zh-primary-en-mixed" | "auto";
export type AsrTask = "transcribe";
export type AsrProgressStage = "queued" | "loading-model" | "transcribing" | "finalizing" | "cancelled" | "failed";
export type AsrErrorCode =
  | "ASR_MODEL_MISSING"
  | "ASR_UNSUPPORTED_AUDIO_FORMAT"
  | "ASR_PROCESS_FAILED"
  | "ASR_USER_CANCELLED";

export interface AsrCliValidationResult {
  readonly ok: boolean;
  readonly errors: readonly string[];
}

export interface AsrModelInput {
  readonly id: string;
  readonly version: string;
  readonly path: string;
}

export interface AsrRunOptions {
  readonly wordTimestamps: boolean;
  readonly temperature: number;
  readonly task: AsrTask;
}

export interface AsrRunRequestInput {
  readonly requestId: string;
  readonly audioPath: string;
  readonly audioFormat: SupportedAudioExtension;
  readonly audioSha256: string;
  readonly model: AsrModelInput;
  readonly languageHint: AsrLanguageHint;
  readonly options: AsrRunOptions;
}

export interface AsrRunRequest {
  readonly kind: "asr.run.request";
  readonly schemaVersion: typeof ASR_SCHEMA_VERSION;
  readonly requestId: string;
  readonly audio: {
    readonly path: string;
    readonly format: SupportedAudioExtension;
    readonly sha256: string;
  };
  readonly model: AsrModelInput;
  readonly languageHint: AsrLanguageHint;
  readonly options: AsrRunOptions;
}

export interface AsrProgressInput {
  readonly requestId: string;
  readonly stage: AsrProgressStage;
  readonly processedMs: number;
  readonly totalMs: number;
  readonly message: string;
}

export interface AsrProgressEvent extends AsrProgressInput {
  readonly kind: "asr.progress";
  readonly schemaVersion: typeof ASR_SCHEMA_VERSION;
  readonly progress: number;
}

export interface AsrTranscriptSegment {
  readonly id: string;
  readonly startMs: number;
  readonly endMs: number;
  readonly text: string;
  readonly language: "zh" | "en" | "mixed" | "unknown";
  readonly confidence?: number;
}

export interface AsrResultInput {
  readonly requestId: string;
  readonly model: {
    readonly id: string;
    readonly version: string;
    readonly parameters: {
      readonly language: "zh" | "en" | "mixed" | "auto";
      readonly temperature: number;
    };
  };
  readonly segments: readonly AsrTranscriptSegment[];
}

export interface AsrResult extends AsrResultInput {
  readonly kind: "asr.result";
  readonly schemaVersion: typeof ASR_SCHEMA_VERSION;
  readonly modelVersion: string;
  readonly parameters: AsrResultInput["model"]["parameters"];
}

export interface AsrError {
  readonly kind: "asr.error";
  readonly schemaVersion: typeof ASR_SCHEMA_VERSION;
  readonly requestId: string;
  readonly code: AsrErrorCode;
  readonly message: string;
  readonly recoverable: boolean;
}

export type AsrCliEnvelope = AsrRunRequest | AsrProgressEvent | AsrResult | AsrError;

const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const LANGUAGE_HINTS: readonly AsrLanguageHint[] = ["zh-primary", "zh-primary-en-mixed", "auto"] as const;
const PROGRESS_STAGES: readonly AsrProgressStage[] = ["queued", "loading-model", "transcribing", "finalizing", "cancelled", "failed"] as const;
const ERROR_CODES: readonly AsrErrorCode[] = [
  "ASR_MODEL_MISSING",
  "ASR_UNSUPPORTED_AUDIO_FORMAT",
  "ASR_PROCESS_FAILED",
  "ASR_USER_CANCELLED"
] as const;
const TRANSCRIPT_LANGUAGES = ["zh", "en", "mixed", "unknown"] as const;
const RESULT_PARAMETER_LANGUAGES = ["zh", "en", "mixed", "auto"] as const;

export function createAsrRunRequest(input: AsrRunRequestInput): AsrRunRequest {
  const request: AsrRunRequest = {
    kind: "asr.run.request",
    schemaVersion: ASR_SCHEMA_VERSION,
    requestId: input.requestId,
    audio: {
      path: input.audioPath,
      format: input.audioFormat,
      sha256: input.audioSha256
    },
    model: { ...input.model },
    languageHint: input.languageHint,
    options: { ...input.options }
  };
  assertValidEnvelope(request);
  return request;
}

export function createAsrProgressEvent(input: AsrProgressInput): AsrProgressEvent {
  const event: AsrProgressEvent = {
    kind: "asr.progress",
    schemaVersion: ASR_SCHEMA_VERSION,
    requestId: input.requestId,
    stage: input.stage,
    processedMs: input.processedMs,
    totalMs: input.totalMs,
    progress: calculateProgress(input.processedMs, input.totalMs),
    message: input.message
  };
  assertValidEnvelope(event);
  return event;
}

export function createAsrResult(input: AsrResultInput): AsrResult {
  const result: AsrResult = {
    kind: "asr.result",
    schemaVersion: ASR_SCHEMA_VERSION,
    requestId: input.requestId,
    model: {
      id: input.model.id,
      version: input.model.version,
      parameters: { ...input.model.parameters }
    },
    modelVersion: input.model.version,
    parameters: { ...input.model.parameters },
    segments: input.segments.map((segment) => ({ ...segment }))
  };
  assertValidEnvelope(result);
  return result;
}

export function createMockAsrResult(input: {
  readonly requestId: string;
  readonly model: AsrResultInput["model"];
}): AsrResult {
  return createAsrResult({
    requestId: input.requestId,
    model: input.model,
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
}

export function validateAsrCliEnvelope(value: unknown): AsrCliValidationResult {
  const errors: string[] = [];

  if (!isRecord(value)) {
    return { ok: false, errors: ["ASR_ENVELOPE_INVALID: envelope must be an object"] };
  }

  if (value.schemaVersion !== ASR_SCHEMA_VERSION) {
    errors.push("ASR_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.asr.v1");
  }

  switch (value.kind) {
    case "asr.run.request":
      validateRunRequest(value, errors);
      break;
    case "asr.progress":
      validateProgressEvent(value, errors);
      break;
    case "asr.result":
      validateResult(value, errors);
      break;
    case "asr.error":
      validateError(value, errors);
      break;
    default:
      errors.push("ASR_KIND_INVALID: kind is unsupported");
  }

  return { ok: errors.length === 0, errors };
}

function validateRunRequest(value: Record<string, unknown>, errors: string[]): void {
  requireNonEmptyString(value.requestId, "ASR_REQUEST_ID_REQUIRED", errors);

  if (!isRecord(value.audio)) {
    errors.push("ASR_AUDIO_INVALID: audio must be an object");
  } else {
    requireNonEmptyString(value.audio.path, "ASR_AUDIO_PATH_REQUIRED", errors);
    if (!SUPPORTED_AUDIO_EXTENSIONS.includes(value.audio.format as SupportedAudioExtension)) {
      errors.push("ASR_AUDIO_FORMAT_UNSUPPORTED: audio.format must be .wav, .mp3, or .m4a");
    }
    if (typeof value.audio.sha256 !== "string" || !SHA256_PATTERN.test(value.audio.sha256)) {
      errors.push("ASR_AUDIO_SHA256_INVALID: audio.sha256 must be 64 lowercase hex characters");
    }
  }

  validateModel(value.model, errors);

  if (!LANGUAGE_HINTS.includes(value.languageHint as AsrLanguageHint)) {
    errors.push("ASR_LANGUAGE_HINT_INVALID: languageHint is unsupported");
  }

  if (!isRecord(value.options)) {
    errors.push("ASR_OPTIONS_INVALID: options must be an object");
  } else {
    if (typeof value.options.wordTimestamps !== "boolean") {
      errors.push("ASR_WORD_TIMESTAMPS_INVALID: options.wordTimestamps must be boolean");
    }
    validateTemperature(value.options.temperature, "options.temperature", errors);
    if (value.options.task !== "transcribe") {
      errors.push("ASR_TASK_INVALID: options.task must be transcribe");
    }
  }
}

function validateProgressEvent(value: Record<string, unknown>, errors: string[]): void {
  requireNonEmptyString(value.requestId, "ASR_REQUEST_ID_REQUIRED", errors);

  if (!PROGRESS_STAGES.includes(value.stage as AsrProgressStage)) {
    errors.push("ASR_PROGRESS_STAGE_INVALID: stage is unsupported");
  }
  if (!isNonNegativeNumber(value.processedMs)) {
    errors.push("ASR_PROCESSED_MS_INVALID: processedMs must be non-negative");
  }
  if (!isPositiveNumber(value.totalMs)) {
    errors.push("ASR_TOTAL_MS_INVALID: totalMs must be positive");
  }
  if (typeof value.processedMs === "number" && typeof value.totalMs === "number" && value.processedMs > value.totalMs) {
    errors.push("ASR_PROGRESS_RANGE_INVALID: processedMs cannot exceed totalMs");
  }
  if (typeof value.progress !== "number" || !Number.isFinite(value.progress) || value.progress < 0 || value.progress > 1) {
    errors.push("ASR_PROGRESS_INVALID: progress must be between 0 and 1");
  }
  requireNonEmptyString(value.message, "ASR_PROGRESS_MESSAGE_REQUIRED", errors);
}

function validateResult(value: Record<string, unknown>, errors: string[]): void {
  requireNonEmptyString(value.requestId, "ASR_REQUEST_ID_REQUIRED", errors);
  requireNonEmptyString(value.modelVersion, "ASR_MODEL_VERSION_REQUIRED", errors);

  if (!isRecord(value.parameters)) {
    errors.push("ASR_RESULT_PARAMETERS_INVALID: parameters must be an object");
  } else {
    validateResultParameters(value.parameters, "parameters", "ASR_RESULT_LANGUAGE_INVALID", errors);
  }

  if (!isRecord(value.model)) {
    errors.push("ASR_RESULT_MODEL_INVALID: model must be an object");
  } else {
    requireNonEmptyString(value.model.id, "ASR_MODEL_ID_REQUIRED", errors);
    requireNonEmptyString(value.model.version, "ASR_MODEL_VERSION_REQUIRED", errors);
    if (!isRecord(value.model.parameters)) {
      errors.push("ASR_MODEL_PARAMETERS_INVALID: model.parameters must be an object");
    } else {
      validateResultParameters(value.model.parameters, "model.parameters", "ASR_MODEL_LANGUAGE_INVALID", errors);
    }
  }

  validateTranscriptSegments(value.segments, errors);
}

function validateError(value: Record<string, unknown>, errors: string[]): void {
  requireNonEmptyString(value.requestId, "ASR_REQUEST_ID_REQUIRED", errors);
  if (!ERROR_CODES.includes(value.code as AsrErrorCode)) {
    errors.push("ASR_ERROR_CODE_INVALID: code is unsupported");
  }
  requireNonEmptyString(value.message, "ASR_ERROR_MESSAGE_REQUIRED", errors);
  if (typeof value.recoverable !== "boolean") {
    errors.push("ASR_ERROR_RECOVERABLE_INVALID: recoverable must be boolean");
  }
}

function validateModel(value: unknown, errors: string[]): void {
  if (!isRecord(value)) {
    errors.push("ASR_MODEL_INVALID: model must be an object");
    return;
  }

  requireNonEmptyString(value.id, "ASR_MODEL_ID_REQUIRED", errors);
  requireNonEmptyString(value.version, "ASR_MODEL_VERSION_REQUIRED", errors);
  requireNonEmptyString(value.path, "ASR_MODEL_PATH_REQUIRED", errors);
}

function validateResultParameters(
  value: Record<string, unknown>,
  fieldName: string,
  languageCode: "ASR_RESULT_LANGUAGE_INVALID" | "ASR_MODEL_LANGUAGE_INVALID",
  errors: string[]
): void {
  if (!RESULT_PARAMETER_LANGUAGES.includes(value.language as "zh" | "en" | "mixed" | "auto")) {
    errors.push(`${languageCode}: ${fieldName}.language is unsupported`);
  }
  validateTemperature(value.temperature, `${fieldName}.temperature`, errors);
}

function validateTranscriptSegments(value: unknown, errors: string[]): void {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push("ASR_SEGMENTS_REQUIRED: segments must be a non-empty array");
    return;
  }

  value.forEach((segment, index) => {
    if (!isRecord(segment)) {
      errors.push(`ASR_SEGMENT_INVALID: segments[${index}] must be an object`);
      return;
    }
    requireNonEmptyString(segment.id, `ASR_SEGMENT_ID_REQUIRED: segments[${index}].id is required`, errors);
    if (!isNonNegativeNumber(segment.startMs)) {
      errors.push(`ASR_SEGMENT_START_INVALID: segments[${index}].startMs must be non-negative`);
    }
    if (
      typeof segment.endMs !== "number" ||
      !Number.isFinite(segment.endMs) ||
      typeof segment.startMs !== "number" ||
      segment.endMs <= segment.startMs
    ) {
      errors.push(`ASR_SEGMENT_END_INVALID: segments[${index}].endMs must be greater than startMs`);
    }
    requireNonEmptyString(segment.text, `ASR_SEGMENT_TEXT_REQUIRED: segments[${index}].text is required`, errors);
    if (!TRANSCRIPT_LANGUAGES.includes(segment.language as "zh" | "en" | "mixed" | "unknown")) {
      errors.push(`ASR_SEGMENT_LANGUAGE_INVALID: segments[${index}].language is unsupported`);
    }
    if (segment.confidence === undefined) {
      errors.push(`ASR_SEGMENT_CONFIDENCE_REQUIRED: segments[${index}].confidence is required`);
    } else if (!isConfidence(segment.confidence)) {
      errors.push(`ASR_SEGMENT_CONFIDENCE_INVALID: segments[${index}].confidence must be between 0 and 1`);
    }
  });
}

function assertValidEnvelope(envelope: AsrCliEnvelope): void {
  const result = validateAsrCliEnvelope(envelope);
  if (!result.ok) {
    throw new Error(result.errors.join("; "));
  }
}

function calculateProgress(processedMs: number, totalMs: number): number {
  if (totalMs <= 0) {
    return 0;
  }
  return Math.min(1, Math.max(0, processedMs / totalMs));
}

function validateTemperature(value: unknown, fieldName: string, errors: string[]): void {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || value > 1) {
    errors.push(`ASR_TEMPERATURE_INVALID: ${fieldName} must be between 0 and 1`);
  }
}

function requireNonEmptyString(value: unknown, code: string, errors: string[]): void {
  if (!isNonEmptyString(value)) {
    errors.push(code);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isPositiveNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value > 0;
}

function isNonNegativeNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function isConfidence(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 1;
}
