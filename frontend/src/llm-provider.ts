import {
  createDeverbalizationOutput,
  createLlmDeverbalizationRequest,
  createLlmProviderError,
  type DeverbalizationOutput,
  type LlmDeverbalizationRequest,
  type LlmProviderEnvelope,
  type LlmProvider
} from "./llm-schema";
import type { CorrectedSpeakerLlmInput } from "./speaker-output";

export interface LlmRequestSegmentTiming {
  readonly sourceSegmentId: string;
  readonly startMs: number;
  readonly endMs: number;
}

export interface CreateLlmRequestFromCorrectedSpeakerInputOptions {
  readonly requestId: string;
  readonly input: CorrectedSpeakerLlmInput;
  readonly segmentTimings: readonly LlmRequestSegmentTiming[];
}

export interface DeterministicMockLlmProviderOptions {
  readonly request: LlmDeverbalizationRequest;
  readonly privacyConfirmed?: boolean;
  readonly onPromptRendered?: (prompt: string) => void;
}

const MOCK_PROVIDER_MODEL = {
  id: "meetnote-mock-deverbalizer",
  version: "deterministic-v1",
  parameters: {
    temperature: 0,
    maxOutputTokens: 4096
  }
} as const;

export function createLlmRequestFromCorrectedSpeakerInput(
  options: CreateLlmRequestFromCorrectedSpeakerInputOptions
): LlmDeverbalizationRequest {
  const timingBySourceSegmentId = new Map(options.segmentTimings.map((timing) => [timing.sourceSegmentId, timing]));
  const transcriptSegments = options.input.segments.flatMap((segment, index) =>
    requireSourceSegmentIds(segment.sourceSegmentIds, index).map((sourceSegmentId, sourceIndex, sourceSegmentIds) => {
      const timing = requireSegmentTiming(timingBySourceSegmentId, sourceSegmentId, index, sourceIndex);

      return {
        id: sourceSegmentId,
        startMs: timing.startMs,
        endMs: timing.endMs,
        speakerId: segment.speakerId,
        speakerLabel: segment.speakerLabel,
        text: sliceSegmentText(segment.text, sourceIndex, sourceSegmentIds.length)
      };
    })
  );

  return createLlmDeverbalizationRequest({
    requestId: options.requestId,
    provider: options.input.provider,
    transcript: {
      originalText: options.input.text,
      segments: transcriptSegments
    },
    paragraphBoundaries: options.input.segments.map((segment, index) => ({
      id: formatParagraphId(index + 1),
      speakerId: segment.speakerId,
      speakerLabel: segment.speakerLabel,
      sourceSegmentIds: requireSourceSegmentIds(segment.sourceSegmentIds, index)
    }))
  });
}

export function renderDeverbalizationPrompt(request: LlmDeverbalizationRequest): string {
  const transcriptLines = request.transcript.segments
    .map(
      (segment) =>
        `[${formatTimestamp(segment.startMs)} - ${formatTimestamp(segment.endMs)}] id=${segment.id} speakerId=${segment.speakerId} speakerLabel=${segment.speakerLabel}\n${segment.speakerLabel}: ${segment.text}`
    )
    .join("\n\n");
  const paragraphBoundaries = request.paragraphBoundaries
    .map(
      (boundary) =>
        `Paragraph ${boundary.id}: sourceSegmentIds=${boundary.sourceSegmentIds.join(", ")} speakerId=${boundary.speakerId} speakerLabel=${boundary.speakerLabel}`
    )
    .join("\n");

  return [
    "Rewrite the transcript into faithful paragraphs.",
    "Preserve traceability to the original transcript segments and speaker labels.",
    "Do not create summaries, action items, conclusions, expansions, or inferred content.",
    "",
    "Original transcript text",
    request.transcript.originalText,
    "",
    "Transcript segments",
    transcriptLines,
    "",
    "Paragraph boundaries",
    paragraphBoundaries
  ].join("\n");
}

export function runDeterministicMockLlmProvider(
  options: DeterministicMockLlmProviderOptions
): LlmProviderEnvelope {
  if (options.request.provider === "custom" && options.privacyConfirmed !== true) {
    return createLlmProviderError({
      requestId: options.request.requestId,
      provider: options.request.provider,
      code: "privacy_confirmation_required",
      message: "Confirm custom provider privacy handling before sending transcript text.",
      recoverable: true
    });
  }

  const prompt = renderDeverbalizationPrompt(options.request);
  options.onPromptRendered?.(prompt);

  return buildDeterministicMockResult(options.request);
}

function buildDeterministicMockResult(request: LlmDeverbalizationRequest): DeverbalizationOutput {
  return createDeverbalizationOutput({
    requestId: request.requestId,
    provider: request.provider,
    model: MOCK_PROVIDER_MODEL,
    paragraphs: request.paragraphBoundaries.map((boundary) => {
      const sourceSegments = boundary.sourceSegmentIds
        .map((sourceSegmentId) => request.transcript.segments.find((segment) => segment.id === sourceSegmentId))
        .filter((segment): segment is LlmDeverbalizationRequest["transcript"]["segments"][number] => segment !== undefined);

      return {
        id: boundary.id,
        sourceSegmentIds: [...boundary.sourceSegmentIds],
        speakerId: boundary.speakerId,
        text: normalizeParagraphText(sourceSegments.map((segment) => segment.text).join("\n"))
      };
    })
  });
}

function normalizeParagraphText(text: string): string {
  return text
    .split("\n")
    .map((line) =>
      line
        .replace(/^(嗯+|啊+|呃+|那个+|然后+|就是+|其实+|所以+|顺便+)\s*/u, "")
        .trim()
    )
    .filter((line) => line.length > 0)
    .join("\n");
}

function requireSourceSegmentIds(sourceSegmentIds: readonly string[], segmentIndex: number): readonly string[] {
  if (sourceSegmentIds.length === 0) {
    throw new Error(`LLM_SOURCE_SEGMENT_IDS_REQUIRED: input.segments[${segmentIndex}].sourceSegmentIds must be a non-empty array`);
  }

  return [...sourceSegmentIds];
}

function requireSegmentTiming(
  timingBySourceSegmentId: ReadonlyMap<string, LlmRequestSegmentTiming>,
  sourceSegmentId: string,
  segmentIndex: number,
  sourceSegmentIndex: number
): LlmRequestSegmentTiming {
  const timing = timingBySourceSegmentId.get(sourceSegmentId);
  if (timing === undefined) {
    throw new Error(
      `LLM_SOURCE_SEGMENT_TIMING_REQUIRED: segmentTimings must include input.segments[${segmentIndex}].sourceSegmentIds[${sourceSegmentIndex}]`
    );
  }

  return timing;
}

function sliceSegmentText(text: string, sourceIndex: number, sourceCount: number): string {
  const lines = text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);

  if (lines.length === sourceCount) {
    return lines[sourceIndex] ?? text;
  }

  return text;
}

function formatParagraphId(index: number): string {
  return `paragraph-${String(index).padStart(3, "0")}`;
}

function formatTimestamp(totalMs: number): string {
  const safeTotalMs = Math.max(0, Math.floor(totalMs));
  const hours = Math.floor(safeTotalMs / 3_600_000);
  const minutes = Math.floor((safeTotalMs % 3_600_000) / 60_000);
  const seconds = Math.floor((safeTotalMs % 60_000) / 1000);
  const milliseconds = safeTotalMs % 1000;
  const minuteTimestamp = `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}.${String(milliseconds).padStart(3, "0")}`;
  return hours === 0 ? minuteTimestamp : `${String(hours).padStart(2, "0")}:${minuteTimestamp}`;
}

export const MOCK_LLM_PROVIDER_MODEL = MOCK_PROVIDER_MODEL;
export type { LlmProvider };
