import type { SupportedAudioExtension } from "./audio-project";
import { SUPPORTED_AUDIO_EXTENSIONS } from "./audio-project";

export const DIARIZATION_SCHEMA_VERSION = "meetnote.diarization.v1" as const;

export type DiarizationAuthProvider = "huggingface";
export type DiarizationAuthState = "configured" | "missing" | "expired";
export type DiarizationStage = "queued" | "loading-model" | "segmenting" | "clustering" | "finalizing" | "cancelled" | "failed";
export type DiarizationErrorCode =
  | "DIARIZATION_MODEL_MISSING"
  | "DIARIZATION_AUTH_REQUIRED"
  | "DIARIZATION_AUTH_EXPIRED"
  | "DIARIZATION_PROCESS_FAILED"
  | "DIARIZATION_USER_CANCELLED";

export interface DiarizationCliValidationResult {
  readonly ok: boolean;
  readonly errors: readonly string[];
}

export interface DiarizationModelInput {
  readonly id: string;
  readonly version: string;
  readonly path: string;
}

export interface DiarizationAuthInput {
  readonly provider: DiarizationAuthProvider;
  readonly state: DiarizationAuthState;
}

export interface DiarizationRunOptions {
  readonly minSpeakers: number;
  readonly maxSpeakers: number;
  readonly collarMs: number;
  readonly overlapDetection: boolean;
}

export interface DiarizationRunRequestInput {
  readonly requestId: string;
  readonly audioPath: string;
  readonly audioFormat: SupportedAudioExtension;
  readonly audioSha256: string;
  readonly model: DiarizationModelInput;
  readonly auth: DiarizationAuthInput;
  readonly options: DiarizationRunOptions;
}

export interface DiarizationRunRequest {
  readonly kind: "diarization.run.request";
  readonly schemaVersion: typeof DIARIZATION_SCHEMA_VERSION;
  readonly requestId: string;
  readonly audio: {
    readonly path: string;
    readonly format: SupportedAudioExtension;
    readonly sha256: string;
  };
  readonly model: DiarizationModelInput;
  readonly auth: DiarizationAuthInput;
  readonly options: DiarizationRunOptions;
}

export interface DiarizationProgressInput {
  readonly requestId: string;
  readonly stage: DiarizationStage;
  readonly processedMs: number;
  readonly totalMs: number;
  readonly message: string;
}

export interface DiarizationProgressEvent extends DiarizationProgressInput {
  readonly kind: "diarization.progress";
  readonly schemaVersion: typeof DIARIZATION_SCHEMA_VERSION;
  readonly progress: number;
}

export interface DiarizationSpeakerProfile {
  readonly id: string;
  readonly label: string;
}

export interface DiarizationSegment {
  readonly id: string;
  readonly speakerId: string;
  readonly startMs: number;
  readonly endMs: number;
  readonly confidence: number;
  readonly overlap: boolean;
}

export interface DiarizationResultInput {
  readonly requestId: string;
  readonly model: {
    readonly id: string;
    readonly version: string;
    readonly parameters: DiarizationRunOptions;
  };
  readonly auth: DiarizationAuthInput;
  readonly speakers: readonly DiarizationSpeakerProfile[];
  readonly segments: readonly DiarizationSegment[];
}

export interface DiarizationResult extends DiarizationResultInput {
  readonly kind: "diarization.result";
  readonly schemaVersion: typeof DIARIZATION_SCHEMA_VERSION;
  readonly modelVersion: string;
  readonly parameters: DiarizationRunOptions;
}

export interface DiarizationError {
  readonly kind: "diarization.error";
  readonly schemaVersion: typeof DIARIZATION_SCHEMA_VERSION;
  readonly requestId: string;
  readonly code: DiarizationErrorCode;
  readonly message: string;
  readonly recoverable: boolean;
}

export type DiarizationCliEnvelope = DiarizationRunRequest | DiarizationProgressEvent | DiarizationResult | DiarizationError;

const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const STAGES: readonly DiarizationStage[] = [
  "queued",
  "loading-model",
  "segmenting",
  "clustering",
  "finalizing",
  "cancelled",
  "failed"
] as const;
const ERROR_CODES: readonly DiarizationErrorCode[] = [
  "DIARIZATION_MODEL_MISSING",
  "DIARIZATION_AUTH_REQUIRED",
  "DIARIZATION_AUTH_EXPIRED",
  "DIARIZATION_PROCESS_FAILED",
  "DIARIZATION_USER_CANCELLED"
] as const;
const AUTH_STATES: readonly DiarizationAuthState[] = ["configured", "missing", "expired"] as const;

export function createDiarizationRunRequest(input: DiarizationRunRequestInput): DiarizationRunRequest {
  const request: DiarizationRunRequest = {
    kind: "diarization.run.request",
    schemaVersion: DIARIZATION_SCHEMA_VERSION,
    requestId: input.requestId,
    audio: {
      path: input.audioPath,
      format: input.audioFormat,
      sha256: input.audioSha256
    },
    model: { ...input.model },
    auth: { ...input.auth },
    options: { ...input.options }
  };
  assertValidEnvelope(request);
  return request;
}

export function createDiarizationProgressEvent(input: DiarizationProgressInput): DiarizationProgressEvent {
  const event: DiarizationProgressEvent = {
    kind: "diarization.progress",
    schemaVersion: DIARIZATION_SCHEMA_VERSION,
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

export function createDiarizationResult(input: DiarizationResultInput): DiarizationResult {
  const result: DiarizationResult = {
    kind: "diarization.result",
    schemaVersion: DIARIZATION_SCHEMA_VERSION,
    requestId: input.requestId,
    model: {
      id: input.model.id,
      version: input.model.version,
      parameters: { ...input.model.parameters }
    },
    modelVersion: input.model.version,
    parameters: { ...input.model.parameters },
    auth: { ...input.auth },
    speakers: input.speakers.map((speaker) => ({ ...speaker })),
    segments: input.segments.map((segment) => ({ ...segment }))
  };
  assertValidEnvelope(result);
  return result;
}

export function createMockDiarizationResult(input: {
  readonly requestId: string;
  readonly model: DiarizationResultInput["model"];
  readonly auth: DiarizationAuthInput;
}): DiarizationResult {
  return createDiarizationResult({
    requestId: input.requestId,
    model: input.model,
    auth: input.auth,
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
}

export function validateDiarizationCliEnvelope(value: unknown): DiarizationCliValidationResult {
  const errors: string[] = [];

  if (!isRecord(value)) {
    return { ok: false, errors: ["DIARIZATION_ENVELOPE_INVALID: envelope must be an object"] };
  }

  if (value.schemaVersion !== DIARIZATION_SCHEMA_VERSION) {
    errors.push("DIARIZATION_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.diarization.v1");
  }

  switch (value.kind) {
    case "diarization.run.request":
      validateRunRequest(value, errors);
      break;
    case "diarization.progress":
      validateProgressEvent(value, errors);
      break;
    case "diarization.result":
      validateResult(value, errors);
      break;
    case "diarization.error":
      validateError(value, errors);
      break;
    default:
      errors.push("DIARIZATION_KIND_INVALID: kind is unsupported");
  }

  return { ok: errors.length === 0, errors };
}

function validateRunRequest(value: Record<string, unknown>, errors: string[]): void {
  requireNonEmptyString(value.requestId, "DIARIZATION_REQUEST_ID_REQUIRED", errors);

  if (!isRecord(value.audio)) {
    errors.push("DIARIZATION_AUDIO_INVALID: audio must be an object");
  } else {
    requireNonEmptyString(value.audio.path, "DIARIZATION_AUDIO_PATH_REQUIRED", errors);
    if (!SUPPORTED_AUDIO_EXTENSIONS.includes(value.audio.format as SupportedAudioExtension)) {
      errors.push("DIARIZATION_AUDIO_FORMAT_UNSUPPORTED: audio.format must be .wav, .mp3, or .m4a");
    }
    if (typeof value.audio.sha256 !== "string" || !SHA256_PATTERN.test(value.audio.sha256)) {
      errors.push("DIARIZATION_AUDIO_SHA256_INVALID: audio.sha256 must be 64 lowercase hex characters");
    }
  }

  validateModel(value.model, errors);
  validateAuth(value.auth, errors);
  validateOptions(value.options, errors);
}

function validateProgressEvent(value: Record<string, unknown>, errors: string[]): void {
  requireNonEmptyString(value.requestId, "DIARIZATION_REQUEST_ID_REQUIRED", errors);

  if (!STAGES.includes(value.stage as DiarizationStage)) {
    errors.push("DIARIZATION_PROGRESS_STAGE_INVALID: stage is unsupported");
  }
  if (!isNonNegativeNumber(value.processedMs)) {
    errors.push("DIARIZATION_PROCESSED_MS_INVALID: processedMs must be non-negative");
  }
  if (!isPositiveNumber(value.totalMs)) {
    errors.push("DIARIZATION_TOTAL_MS_INVALID: totalMs must be positive");
  }
  if (typeof value.processedMs === "number" && typeof value.totalMs === "number" && value.processedMs > value.totalMs) {
    errors.push("DIARIZATION_PROGRESS_RANGE_INVALID: processedMs cannot exceed totalMs");
  }
  if (typeof value.progress !== "number" || !Number.isFinite(value.progress) || value.progress < 0 || value.progress > 1) {
    errors.push("DIARIZATION_PROGRESS_INVALID: progress must be between 0 and 1");
  }
  requireNonEmptyString(value.message, "DIARIZATION_PROGRESS_MESSAGE_REQUIRED", errors);
}

function validateResult(value: Record<string, unknown>, errors: string[]): void {
  requireNonEmptyString(value.requestId, "DIARIZATION_REQUEST_ID_REQUIRED", errors);
  requireNonEmptyString(value.modelVersion, "DIARIZATION_MODEL_VERSION_REQUIRED", errors);

  if (!isRecord(value.parameters)) {
    errors.push("DIARIZATION_RESULT_PARAMETERS_INVALID: parameters must be an object");
  } else {
    validateOptions(value.parameters, errors, "DIARIZATION_RESULT_OPTION_INVALID");
  }

  if (!isRecord(value.model)) {
    errors.push("DIARIZATION_RESULT_MODEL_INVALID: model must be an object");
  } else {
    requireNonEmptyString(value.model.id, "DIARIZATION_MODEL_ID_REQUIRED", errors);
    requireNonEmptyString(value.model.version, "DIARIZATION_MODEL_VERSION_REQUIRED", errors);
    if (isNonEmptyString(value.modelVersion) && isNonEmptyString(value.model.version) && value.modelVersion !== value.model.version) {
      errors.push("DIARIZATION_MODEL_VERSION_MISMATCH: modelVersion must equal model.version");
    }
    if (!isRecord(value.model.parameters)) {
      errors.push("DIARIZATION_MODEL_PARAMETERS_INVALID: model.parameters must be an object");
    } else {
      validateOptions(value.model.parameters, errors, "DIARIZATION_MODEL_OPTION_INVALID");
      if (isRecord(value.parameters) && !areDiarizationOptionsEqual(value.parameters, value.model.parameters)) {
        errors.push("DIARIZATION_MODEL_PARAMETERS_MISMATCH: parameters must equal model.parameters");
      }
    }
  }

  validateAuth(value.auth, errors);
  validateSpeakers(value.speakers, errors);
  validateSegments(value.segments, value.speakers, errors);
}

function validateError(value: Record<string, unknown>, errors: string[]): void {
  requireNonEmptyString(value.requestId, "DIARIZATION_REQUEST_ID_REQUIRED", errors);
  if (!ERROR_CODES.includes(value.code as DiarizationErrorCode)) {
    errors.push("DIARIZATION_ERROR_CODE_INVALID: code is unsupported");
  }
  requireNonEmptyString(value.message, "DIARIZATION_ERROR_MESSAGE_REQUIRED", errors);
  if (typeof value.recoverable !== "boolean") {
    errors.push("DIARIZATION_ERROR_RECOVERABLE_INVALID: recoverable must be boolean");
  }
}

function validateModel(value: unknown, errors: string[]): void {
  if (!isRecord(value)) {
    errors.push("DIARIZATION_MODEL_INVALID: model must be an object");
    return;
  }

  requireNonEmptyString(value.id, "DIARIZATION_MODEL_ID_REQUIRED", errors);
  requireNonEmptyString(value.version, "DIARIZATION_MODEL_VERSION_REQUIRED", errors);
  requireNonEmptyString(value.path, "DIARIZATION_MODEL_PATH_REQUIRED", errors);
}

function validateAuth(value: unknown, errors: string[]): void {
  if (!isRecord(value)) {
    errors.push("DIARIZATION_AUTH_INVALID: auth must be an object");
    return;
  }

  if (value.provider !== "huggingface") {
    errors.push("DIARIZATION_AUTH_PROVIDER_INVALID: auth.provider is unsupported");
  }
  if (!AUTH_STATES.includes(value.state as DiarizationAuthState)) {
    errors.push("DIARIZATION_AUTH_STATE_INVALID: auth.state is unsupported");
  }
}

function validateOptions(value: unknown, errors: string[], codePrefix = "DIARIZATION_OPTIONS_INVALID"): void {
  if (!isRecord(value)) {
    errors.push(`${codePrefix}: options must be an object`);
    return;
  }

  if (!isPositiveInteger(value.minSpeakers)) {
    errors.push("DIARIZATION_MIN_SPEAKERS_INVALID: options.minSpeakers must be a positive integer");
  }
  if (!isPositiveInteger(value.maxSpeakers)) {
    errors.push("DIARIZATION_MAX_SPEAKERS_INVALID: options.maxSpeakers must be a positive integer");
  }
  if (
    typeof value.minSpeakers === "number" &&
    typeof value.maxSpeakers === "number" &&
    Number.isInteger(value.minSpeakers) &&
    Number.isInteger(value.maxSpeakers) &&
    value.maxSpeakers < value.minSpeakers
  ) {
    errors.push("DIARIZATION_SPEAKER_RANGE_INVALID: options.maxSpeakers cannot be less than minSpeakers");
  }
  if (!isNonNegativeNumber(value.collarMs)) {
    errors.push("DIARIZATION_COLLAR_MS_INVALID: options.collarMs must be non-negative");
  }
  if (typeof value.overlapDetection !== "boolean") {
    errors.push("DIARIZATION_OVERLAP_DETECTION_INVALID: options.overlapDetection must be boolean");
  }
}

function validateSpeakers(value: unknown, errors: string[]): void {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push("DIARIZATION_SPEAKERS_REQUIRED: speakers must be a non-empty array");
    return;
  }

  value.forEach((speaker, index) => {
    if (!isRecord(speaker)) {
      errors.push(`DIARIZATION_SPEAKER_INVALID: speakers[${index}] must be an object`);
      return;
    }
    requireNonEmptyString(speaker.id, `DIARIZATION_SPEAKER_ID_REQUIRED: speakers[${index}].id is required`, errors);
    requireNonEmptyString(speaker.label, `DIARIZATION_SPEAKER_LABEL_REQUIRED: speakers[${index}].label is required`, errors);
  });
}

function validateSegments(value: unknown, speakers: unknown, errors: string[]): void {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push("DIARIZATION_SEGMENTS_REQUIRED: segments must be a non-empty array");
    return;
  }

  const speakerIds = new Set(
    Array.isArray(speakers)
      ? speakers
          .filter((speaker): speaker is Record<string, unknown> => isRecord(speaker))
          .map((speaker) => speaker.id)
          .filter((speakerId): speakerId is string => isNonEmptyString(speakerId))
      : []
  );

  value.forEach((segment, index) => {
    if (!isRecord(segment)) {
      errors.push(`DIARIZATION_SEGMENT_INVALID: segments[${index}] must be an object`);
      return;
    }
    requireNonEmptyString(segment.id, `DIARIZATION_SEGMENT_ID_REQUIRED: segments[${index}].id is required`, errors);
    requireNonEmptyString(segment.speakerId, `DIARIZATION_SEGMENT_SPEAKER_ID_REQUIRED: segments[${index}].speakerId is required`, errors);
    if (!isNonNegativeNumber(segment.startMs)) {
      errors.push(`DIARIZATION_SEGMENT_START_INVALID: segments[${index}].startMs must be non-negative`);
    }
    if (
      typeof segment.endMs !== "number" ||
      !Number.isFinite(segment.endMs) ||
      typeof segment.startMs !== "number" ||
      segment.endMs <= segment.startMs
    ) {
      errors.push(`DIARIZATION_SEGMENT_END_INVALID: segments[${index}].endMs must be greater than startMs`);
    }
    if (!isConfidence(segment.confidence)) {
      errors.push(`DIARIZATION_SEGMENT_CONFIDENCE_INVALID: segments[${index}].confidence must be between 0 and 1`);
    }
    if (typeof segment.overlap !== "boolean") {
      errors.push(`DIARIZATION_SEGMENT_OVERLAP_INVALID: segments[${index}].overlap must be boolean`);
    }
    if (isNonEmptyString(segment.speakerId) && speakerIds.size > 0 && !speakerIds.has(segment.speakerId)) {
      errors.push(`DIARIZATION_SEGMENT_SPEAKER_UNKNOWN: segments[${index}].speakerId must reference a known speaker`);
    }
  });
}

function assertValidEnvelope(envelope: DiarizationCliEnvelope): void {
  const result = validateDiarizationCliEnvelope(envelope);
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

function isPositiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value > 0;
}

function isNonNegativeNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function isConfidence(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 1;
}

function areDiarizationOptionsEqual(left: Record<string, unknown>, right: Record<string, unknown>): boolean {
  return (
    left.minSpeakers === right.minSpeakers &&
    left.maxSpeakers === right.maxSpeakers &&
    left.collarMs === right.collarMs &&
    left.overlapDetection === right.overlapDetection
  );
}
