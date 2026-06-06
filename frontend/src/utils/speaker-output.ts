import type { MeetingProject, SpeakerProfile, SpeakerSegment, TranscriptSegment } from "../types/audio-project";
import { getOverlapMs } from "../utils/speaker";

export interface CorrectedSpeakerLine {
  readonly transcriptSegmentId: string;
  readonly speakerId: string;
  readonly speakerLabel: string;
  readonly startMs: number;
  readonly endMs: number;
  readonly text: string;
}

export interface CorrectedSpeakerBlock {
  readonly speakerId: string;
  readonly speakerLabel: string;
  readonly startMs: number;
  readonly endMs: number;
  readonly transcriptSegmentIds: readonly string[];
  readonly text: string;
}

export interface CorrectedSpeakerOutput {
  readonly lines: readonly CorrectedSpeakerLine[];
  readonly blocks: readonly CorrectedSpeakerBlock[];
}

export interface CorrectedSpeakerLlmInputSegment {
  readonly sourceSegmentIds: readonly string[];
  readonly speakerId: string;
  readonly speakerLabel: string;
  readonly text: string;
}

export interface CorrectedSpeakerLlmInput {
  readonly kind: "corrected-speaker-deverbalization-input";
  readonly schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1";
  readonly provider: "ollama" | "custom";
  readonly text: string;
  readonly segments: readonly CorrectedSpeakerLlmInputSegment[];
}

export function projectCorrectedSpeakerOutput(
  transcript: readonly TranscriptSegment[],
  speakerSegments: readonly SpeakerSegment[],
  speakers: readonly SpeakerProfile[]
): CorrectedSpeakerOutput {
  const speakerLabels = new Map(speakers.map((speaker) => [speaker.id, speaker.label]));
  const lines = transcript
    .map((segment) => {
      const speakerSegment = findBestSpeakerSegment(segment, speakerSegments);
      const speakerId = speakerSegment?.speakerId ?? "unknown";
      return {
        transcriptSegmentId: segment.id,
        speakerId,
        speakerLabel: speakerLabels.get(speakerId) ?? speakerId,
        startMs: segment.startMs,
        endMs: segment.endMs,
        text: segment.text
      };
    })
    .sort(compareLines);

  return {
    lines,
    blocks: groupSpeakerBlocks(lines)
  };
}

export function projectMeetingCorrectedSpeakerOutput(project: MeetingProject): CorrectedSpeakerOutput {
  return projectCorrectedSpeakerOutput(project.transcript, project.speakerSegments, project.speakers);
}

export function renderCorrectedSpeakerMarkdown(output: CorrectedSpeakerOutput): string {
  if (output.blocks.length === 0) {
    return "";
  }

  return output.blocks.map((block) => `**${block.speakerLabel}:** ${block.text}`).join("\n\n");
}

export function renderCorrectedSpeakerLlmInput(output: CorrectedSpeakerOutput): string {
  return output.lines.map((line) => `${line.speakerLabel}: ${line.text}`).join("\n");
}

export function createCorrectedSpeakerLlmInput(
  provider: CorrectedSpeakerLlmInput["provider"],
  output: CorrectedSpeakerOutput
): CorrectedSpeakerLlmInput {
  return {
    kind: "corrected-speaker-deverbalization-input",
    schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
    provider,
    text: renderCorrectedSpeakerLlmInput(output),
    segments: output.lines.map((line) => ({
      sourceSegmentIds: [line.transcriptSegmentId],
      speakerId: line.speakerId,
      speakerLabel: line.speakerLabel,
      text: line.text
    }))
  };
}

function findBestSpeakerSegment(
  transcriptSegment: TranscriptSegment,
  speakerSegments: readonly SpeakerSegment[]
): SpeakerSegment | null {
  let bestSegment: SpeakerSegment | null = null;
  let bestOverlapMs = 0;

  for (const speakerSegment of speakerSegments) {
    const overlapMs = getOverlapMs(
      transcriptSegment.startMs,
      transcriptSegment.endMs,
      speakerSegment.startMs,
      speakerSegment.endMs
    );
    if (overlapMs === 0) {
      continue;
    }
    if (overlapMs > bestOverlapMs) {
      bestSegment = speakerSegment;
      bestOverlapMs = overlapMs;
      continue;
    }
    if (overlapMs === bestOverlapMs && bestSegment !== null && speakerSegment.startMs < bestSegment.startMs) {
      bestSegment = speakerSegment;
    }
  }

  return bestSegment === null ? null : { ...bestSegment };
}

function groupSpeakerBlocks(lines: readonly CorrectedSpeakerLine[]): CorrectedSpeakerBlock[] {
  const blocks: CorrectedSpeakerBlock[] = [];

  for (const line of lines) {
    const previous = blocks[blocks.length - 1];
    if (previous !== undefined && previous.speakerId === line.speakerId) {
      blocks[blocks.length - 1] = {
        ...previous,
        endMs: line.endMs,
        transcriptSegmentIds: [...previous.transcriptSegmentIds, line.transcriptSegmentId],
        text: `${previous.text}\n${line.text}`
      };
      continue;
    }

    blocks.push({
      speakerId: line.speakerId,
      speakerLabel: line.speakerLabel,
      startMs: line.startMs,
      endMs: line.endMs,
      transcriptSegmentIds: [line.transcriptSegmentId],
      text: line.text
    });
  }

  return blocks;
}

function compareLines(left: CorrectedSpeakerLine, right: CorrectedSpeakerLine): number {
  return left.startMs - right.startMs || left.endMs - right.endMs || left.transcriptSegmentId.localeCompare(right.transcriptSegmentId);
}

