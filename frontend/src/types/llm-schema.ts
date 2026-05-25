export const LLM_PROVIDER_SCHEMA_VERSION = "meetnote.llm-provider.v1" as const;

export type LlmProvider = "ollama" | "custom";
export type LlmProviderErrorCode =
  | "timeout"
  | "token_insufficient"
  | "privacy_confirmation_required"
  | "user_cancelled"
  | "schema_validation_failed";

export interface ValidationResult {
  readonly ok: boolean;
  readonly errors: readonly string[];
}

export interface LlmSourceTranscriptSegment {
  readonly id: string;
  readonly startMs: number;
  readonly endMs: number;
  readonly speakerId: string;
  readonly speakerLabel: string;
  readonly text: string;
}

export interface LlmParagraphBoundary {
  readonly id: string;
  readonly speakerId: string;
  readonly speakerLabel: string;
  readonly sourceSegmentIds: readonly string[];
}

export interface LlmDeverbalizationRequestInput {
  readonly requestId: string;
  readonly provider: LlmProvider;
  readonly transcript: {
    readonly originalText: string;
    readonly segments: readonly LlmSourceTranscriptSegment[];
  };
  readonly paragraphBoundaries: readonly LlmParagraphBoundary[];
}

export interface LlmDeverbalizationRequest extends LlmDeverbalizationRequestInput {
  readonly kind: "llm.deverbalization.request";
  readonly schemaVersion: typeof LLM_PROVIDER_SCHEMA_VERSION;
}

export interface LlmProviderModel {
  readonly id: string;
  readonly version: string;
  readonly parameters: {
    readonly temperature: number;
    readonly maxOutputTokens: number;
  };
}

export interface DeverbalizedParagraph {
  readonly id: string;
  readonly sourceSegmentIds: readonly string[];
  readonly speakerId: string;
  readonly text: string;
}

export interface DeverbalizationOutputInput {
  readonly requestId: string;
  readonly provider: LlmProvider;
  readonly model: LlmProviderModel;
  readonly paragraphs: readonly DeverbalizedParagraph[];
}

export interface DeverbalizationOutput extends DeverbalizationOutputInput {
  readonly kind: "llm.deverbalization.result";
  readonly schemaVersion: typeof LLM_PROVIDER_SCHEMA_VERSION;
  readonly modelVersion: string;
  readonly parameters: LlmProviderModel["parameters"];
  readonly summary?: never;
  readonly actionItems?: never;
  readonly actions?: never;
  readonly todos?: never;
  readonly conclusions?: never;
}

export interface LlmProviderErrorInput {
  readonly requestId: string;
  readonly provider: LlmProvider;
  readonly code: LlmProviderErrorCode;
  readonly message: string;
  readonly recoverable: boolean;
}

export interface LlmProviderError extends LlmProviderErrorInput {
  readonly kind: "llm.deverbalization.error";
  readonly schemaVersion: typeof LLM_PROVIDER_SCHEMA_VERSION;
}

export type LlmProviderEnvelope = LlmDeverbalizationRequest | DeverbalizationOutput | LlmProviderError;

const PROVIDERS: readonly LlmProvider[] = ["ollama", "custom"] as const;
const ERROR_CODES: readonly LlmProviderErrorCode[] = [
  "timeout",
  "token_insufficient",
  "privacy_confirmation_required",
  "user_cancelled",
  "schema_validation_failed"
] as const;
const LEGACY_DEVERBALIZATION_SCHEMA_VERSION = "meetnote.deverbalization.v1" as const;
const FORBIDDEN_MVP_KEYS = ["summary", "actionItems", "actions", "todos", "conclusions"] as const;

export function createLlmDeverbalizationRequest(input: LlmDeverbalizationRequestInput): LlmDeverbalizationRequest {
  const request: LlmDeverbalizationRequest = {
    kind: "llm.deverbalization.request",
    schemaVersion: LLM_PROVIDER_SCHEMA_VERSION,
    requestId: input.requestId,
    provider: input.provider,
    transcript: {
      originalText: input.transcript.originalText,
      segments: input.transcript.segments.map((segment) => ({ ...segment }))
    },
    paragraphBoundaries: input.paragraphBoundaries.map((boundary) => ({
      ...boundary,
      sourceSegmentIds: [...boundary.sourceSegmentIds]
    }))
  };
  assertValidEnvelope(request);
  return request;
}

export function createDeverbalizationOutput(input: DeverbalizationOutputInput): DeverbalizationOutput {
  const output: DeverbalizationOutput = {
    kind: "llm.deverbalization.result",
    schemaVersion: LLM_PROVIDER_SCHEMA_VERSION,
    requestId: input.requestId,
    provider: input.provider,
    model: {
      id: input.model.id,
      version: input.model.version,
      parameters: { ...input.model.parameters }
    },
    modelVersion: input.model.version,
    parameters: { ...input.model.parameters },
    paragraphs: input.paragraphs.map((paragraph) => ({
      ...paragraph,
      sourceSegmentIds: [...paragraph.sourceSegmentIds]
    }))
  };
  assertValidEnvelope(output);
  return output;
}

export function createLlmProviderError(input: LlmProviderErrorInput): LlmProviderError {
  const error: LlmProviderError = {
    kind: "llm.deverbalization.error",
    schemaVersion: LLM_PROVIDER_SCHEMA_VERSION,
    requestId: input.requestId,
    provider: input.provider,
    code: input.code,
    message: input.message,
    recoverable: input.recoverable
  };
  assertValidEnvelope(error);
  return error;
}

export function validateLlmProviderEnvelope(value: unknown): ValidationResult {
  const errors: string[] = [];

  if (!isRecord(value)) {
    return { ok: false, errors: ["LLM_ENVELOPE_INVALID: envelope must be an object"] };
  }

  if (value.schemaVersion !== LLM_PROVIDER_SCHEMA_VERSION) {
    errors.push("LLM_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.llm-provider.v1");
  }

  switch (value.kind) {
    case "llm.deverbalization.request":
      validateRequest(value, errors);
      break;
    case "llm.deverbalization.result":
      validateResult(value, errors);
      break;
    case "llm.deverbalization.error":
      validateError(value, errors);
      break;
    default:
      errors.push("LLM_KIND_INVALID: kind is unsupported");
      validateCommonEnvelopeFields(value, errors);
      if ("paragraphs" in value) {
        validateParagraphs(value.paragraphs, errors);
      }
  }

  return { ok: errors.length === 0, errors };
}

export function validateDeverbalizationOutput(value: unknown): ValidationResult {
  const errors: string[] = [];

  if (!isRecord(value)) {
    return { ok: false, errors: ["LLM_ENVELOPE_INVALID: envelope must be an object"] };
  }

  if (isLegacyDeverbalizationOutput(value)) {
    validateLegacyDeverbalizationOutput(value, errors);
    return { ok: errors.length === 0, errors };
  }

  if (value.schemaVersion !== LLM_PROVIDER_SCHEMA_VERSION) {
    errors.push("LLM_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.llm-provider.v1");
  }
  if (value.kind !== "llm.deverbalization.result") {
    errors.push("LLM_KIND_INVALID: kind must be llm.deverbalization.result");
  }
  validateResult(value, errors);

  return { ok: errors.length === 0, errors };
}

function validateRequest(value: Record<string, unknown>, errors: string[]): void {
  validateCommonEnvelopeFields(value, errors);
  validateTranscript(value.transcript, errors);
  validateParagraphBoundaries(value.paragraphBoundaries, value.transcript, errors);
}

function validateResult(value: Record<string, unknown>, errors: string[]): void {
  validateCommonEnvelopeFields(value, errors);
  validateForbiddenMvpFields(value, "", errors);

  requireNonEmptyString(value.modelVersion, "LLM_MODEL_VERSION_REQUIRED: modelVersion is required", errors);

  if (!isRecord(value.parameters)) {
    errors.push("LLM_RESULT_PARAMETERS_INVALID: parameters must be an object");
  } else {
    validateModelParameters(value.parameters, "parameters", errors);
  }

  if (!isRecord(value.model)) {
    errors.push("LLM_MODEL_INVALID: model must be an object");
  } else {
    requireNonEmptyString(value.model.id, "LLM_MODEL_ID_REQUIRED: model.id is required", errors);
    requireNonEmptyString(value.model.version, "LLM_MODEL_VERSION_REQUIRED: model.version is required", errors);
    if (isNonEmptyString(value.modelVersion) && isNonEmptyString(value.model.version) && value.modelVersion !== value.model.version) {
      errors.push("LLM_MODEL_VERSION_MISMATCH: modelVersion must equal model.version");
    }
    if (!isRecord(value.model.parameters)) {
      errors.push("LLM_MODEL_PARAMETERS_INVALID: model.parameters must be an object");
    } else {
      validateModelParameters(value.model.parameters, "model.parameters", errors);
      if (isRecord(value.parameters) && !areModelParametersEqual(value.parameters, value.model.parameters)) {
        errors.push("LLM_MODEL_PARAMETERS_MISMATCH: parameters must equal model.parameters");
      }
    }
  }

  validateParagraphs(value.paragraphs, errors);
}

function validateError(value: Record<string, unknown>, errors: string[]): void {
  validateCommonEnvelopeFields(value, errors);
  if (!ERROR_CODES.includes(value.code as LlmProviderErrorCode)) {
    errors.push("LLM_ERROR_CODE_INVALID: code is unsupported");
  }
  requireNonEmptyString(value.message, "LLM_ERROR_MESSAGE_REQUIRED: message is required", errors);
  if (typeof value.recoverable !== "boolean") {
    errors.push("LLM_ERROR_RECOVERABLE_INVALID: recoverable must be boolean");
  }
}

function validateCommonEnvelopeFields(value: Record<string, unknown>, errors: string[]): void {
  requireNonEmptyString(value.requestId, "LLM_REQUEST_ID_REQUIRED: requestId is required", errors);
  if (!PROVIDERS.includes(value.provider as LlmProvider)) {
    errors.push("LLM_PROVIDER_INVALID: provider must be ollama or custom");
  }
}

function validateTranscript(value: unknown, errors: string[]): void {
  if (!isRecord(value)) {
    errors.push("LLM_TRANSCRIPT_INVALID: transcript must be an object");
    return;
  }

  requireNonEmptyString(value.originalText, "LLM_TRANSCRIPT_TEXT_REQUIRED: transcript.originalText is required", errors);
  validateSourceSegments(value.segments, errors);
}

function validateSourceSegments(value: unknown, errors: string[]): void {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push("LLM_TRANSCRIPT_SEGMENTS_REQUIRED: transcript.segments must be a non-empty array");
    return;
  }

  value.forEach((segment, index) => {
    if (!isRecord(segment)) {
      errors.push(`LLM_TRANSCRIPT_SEGMENT_INVALID: transcript.segments[${index}] must be an object`);
      return;
    }
    requireNonEmptyString(segment.id, `LLM_TRANSCRIPT_SEGMENT_ID_REQUIRED: transcript.segments[${index}].id is required`, errors);
    if (!isNonNegativeNumber(segment.startMs)) {
      errors.push(`LLM_TRANSCRIPT_SEGMENT_START_INVALID: transcript.segments[${index}].startMs must be non-negative`);
    }
    if (
      typeof segment.endMs !== "number" ||
      !Number.isFinite(segment.endMs) ||
      typeof segment.startMs !== "number" ||
      segment.endMs <= segment.startMs
    ) {
      errors.push(`LLM_TRANSCRIPT_SEGMENT_END_INVALID: transcript.segments[${index}].endMs must be greater than startMs`);
    }
    requireNonEmptyString(
      segment.speakerId,
      `LLM_TRANSCRIPT_SEGMENT_SPEAKER_REQUIRED: transcript.segments[${index}].speakerId is required`,
      errors
    );
    requireNonEmptyString(
      segment.speakerLabel,
      `LLM_TRANSCRIPT_SEGMENT_SPEAKER_LABEL_REQUIRED: transcript.segments[${index}].speakerLabel is required`,
      errors
    );
    requireNonEmptyString(segment.text, `LLM_TRANSCRIPT_SEGMENT_TEXT_REQUIRED: transcript.segments[${index}].text is required`, errors);
  });
}

function validateParagraphBoundaries(value: unknown, transcript: unknown, errors: string[]): void {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push("LLM_PARAGRAPH_BOUNDARIES_REQUIRED: paragraphBoundaries must be a non-empty array");
    return;
  }

  const segments = isRecord(transcript) && Array.isArray(transcript.segments) ? transcript.segments : [];
  const segmentById = new Map(
    segments
      .filter((segment): segment is Record<string, unknown> => isRecord(segment) && isNonEmptyString(segment.id))
      .map((segment) => [segment.id as string, segment])
  );

  value.forEach((boundary, index) => {
    if (!isRecord(boundary)) {
      errors.push(`LLM_PARAGRAPH_BOUNDARY_INVALID: paragraphBoundaries[${index}] must be an object`);
      return;
    }
    requireNonEmptyString(boundary.id, `LLM_PARAGRAPH_BOUNDARY_ID_REQUIRED: paragraphBoundaries[${index}].id is required`, errors);
    requireNonEmptyString(
      boundary.speakerId,
      `LLM_PARAGRAPH_BOUNDARY_SPEAKER_REQUIRED: paragraphBoundaries[${index}].speakerId is required`,
      errors
    );
    requireNonEmptyString(
      boundary.speakerLabel,
      `LLM_PARAGRAPH_BOUNDARY_SPEAKER_LABEL_REQUIRED: paragraphBoundaries[${index}].speakerLabel is required`,
      errors
    );
    validateSourceSegmentIds(
      boundary.sourceSegmentIds,
      `paragraphBoundaries[${index}].sourceSegmentIds`,
      "LLM_PARAGRAPH_BOUNDARY_TRACE_REQUIRED",
      errors
    );
    if (Array.isArray(boundary.sourceSegmentIds)) {
      boundary.sourceSegmentIds.forEach((sourceSegmentId, sourceSegmentIndex) => {
        if (!isNonEmptyString(sourceSegmentId)) {
          return;
        }
        const segment = segmentById.get(sourceSegmentId);
        if (segment === undefined && segmentById.size > 0) {
          errors.push(
            `LLM_PARAGRAPH_BOUNDARY_SOURCE_UNKNOWN: paragraphBoundaries[${index}].sourceSegmentIds[${sourceSegmentIndex}] must reference transcript.segments`
          );
        }
        if (segment !== undefined && isNonEmptyString(boundary.speakerId) && segment.speakerId !== boundary.speakerId) {
          errors.push(
            `LLM_PARAGRAPH_BOUNDARY_SPEAKER_MISMATCH: paragraphBoundaries[${index}].speakerId must match referenced transcript segment speakerId`
          );
        }
      });
    }
  });
}

function validateParagraphs(value: unknown, errors: string[]): void {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push("LLM_PARAGRAPHS_REQUIRED: paragraphs must be a non-empty array");
    return;
  }

  value.forEach((paragraph, index) => {
    if (!isRecord(paragraph)) {
      errors.push(`LLM_PARAGRAPH_INVALID: paragraphs[${index}] must be an object`);
      return;
    }
    validateForbiddenMvpFields(paragraph, `paragraphs[${index}].`, errors);
    requireNonEmptyString(paragraph.id, `LLM_PARAGRAPH_ID_REQUIRED: paragraphs[${index}].id is required`, errors);
    validateSourceSegmentIds(
      paragraph.sourceSegmentIds,
      `paragraphs[${index}].sourceSegmentIds`,
      "LLM_PARAGRAPH_TRACE_REQUIRED",
      errors
    );
    requireNonEmptyString(paragraph.speakerId, `LLM_PARAGRAPH_SPEAKER_REQUIRED: paragraphs[${index}].speakerId is required`, errors);
    requireNonEmptyString(paragraph.text, `LLM_PARAGRAPH_TEXT_REQUIRED: paragraphs[${index}].text is required`, errors);
  });
}

function validateLegacyDeverbalizationOutput(value: Record<string, unknown>, errors: string[]): void {
  if (value.schemaVersion !== LEGACY_DEVERBALIZATION_SCHEMA_VERSION) {
    errors.push("LLM_LEGACY_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.deverbalization.v1");
  }
  if (value.kind !== "deverbalization") {
    errors.push("LLM_LEGACY_KIND_INVALID: kind must be deverbalization");
  }
  if (!PROVIDERS.includes(value.provider as LlmProvider)) {
    errors.push("LLM_PROVIDER_INVALID: provider must be ollama or custom");
  }
  validateForbiddenMvpFields(value, "", errors);
  validateLegacySegments(value.segments, errors);
}

function validateLegacySegments(value: unknown, errors: string[]): void {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push("LLM_LEGACY_SEGMENTS_REQUIRED: segments must be a non-empty array");
    return;
  }

  value.forEach((segment, index) => {
    if (!isRecord(segment)) {
      errors.push(`LLM_LEGACY_SEGMENT_INVALID: segments[${index}] must be an object`);
      return;
    }
    validateForbiddenMvpFields(segment, `segments[${index}].`, errors);
    validateSourceSegmentIds(
      segment.sourceSegmentIds,
      `segments[${index}].sourceSegmentIds`,
      "LLM_LEGACY_SEGMENT_TRACE_REQUIRED",
      errors
    );
    requireNonEmptyString(segment.speakerId, `LLM_LEGACY_SEGMENT_SPEAKER_REQUIRED: segments[${index}].speakerId is required`, errors);
    requireNonEmptyString(segment.text, `LLM_LEGACY_SEGMENT_TEXT_REQUIRED: segments[${index}].text is required`, errors);
  });
}

function validateForbiddenMvpFields(value: Record<string, unknown>, pathPrefix: string, errors: string[]): void {
  for (const key of FORBIDDEN_MVP_KEYS) {
    if (key in value) {
      errors.push(`LLM_FORBIDDEN_FIELD: ${pathPrefix}${key} is not allowed in MVP output`);
    }
  }
}

function validateSourceSegmentIds(value: unknown, path: string, emptyCode: string, errors: string[]): void {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push(`${emptyCode}: ${path} must be a non-empty array`);
    return;
  }

  value.forEach((sourceSegmentId, index) => {
    if (!isNonEmptyString(sourceSegmentId)) {
      errors.push(`LLM_SOURCE_SEGMENT_ID_INVALID: ${path}[${index}] must be a non-empty string`);
    }
  });
}

function validateModelParameters(value: Record<string, unknown>, path: string, errors: string[]): void {
  if (typeof value.temperature !== "number" || !Number.isFinite(value.temperature) || value.temperature < 0 || value.temperature > 1) {
    errors.push(`LLM_TEMPERATURE_INVALID: ${path}.temperature must be between 0 and 1`);
  }
  if (!isPositiveInteger(value.maxOutputTokens)) {
    errors.push(`LLM_MAX_OUTPUT_TOKENS_INVALID: ${path}.maxOutputTokens must be a positive integer`);
  }
}

function assertValidEnvelope(envelope: LlmProviderEnvelope): void {
  const result = validateLlmProviderEnvelope(envelope);
  if (!result.ok) {
    throw new Error(result.errors.join("; "));
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

function isLegacyDeverbalizationOutput(value: Record<string, unknown>): boolean {
  return value.kind === "deverbalization" || value.schemaVersion === LEGACY_DEVERBALIZATION_SCHEMA_VERSION;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isNonNegativeNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value > 0;
}

function areModelParametersEqual(left: Record<string, unknown>, right: Record<string, unknown>): boolean {
  return left.temperature === right.temperature && left.maxOutputTokens === right.maxOutputTokens;
}
