import type { MeetingProject, SpeakerCorrection, SpeakerProfile, SpeakerSegment, TranscriptSegment } from "./audio-project";

export interface SpeakerMergedTranscriptSegment {
  readonly id: string;
  readonly text: string;
  readonly language: TranscriptSegment["language"];
  readonly speakerId: string;
  readonly startMs: number;
  readonly endMs: number;
  readonly confidence?: number;
}

export interface SpeakerDiarizationSegment {
  readonly id: string;
  readonly speakerId: string;
  readonly startMs: number;
  readonly endMs: number;
  readonly confidence?: number;
}

export interface SpeakerMatchOptions {
  readonly minOverlapMs?: number;
  readonly minOverlapRatio?: number;
}

export type SpeakerCorrectionCommand =
  | {
      readonly id: string;
      readonly kind: "rename-speaker";
      readonly speakerId: string;
      readonly label: string;
      readonly createdAt: string;
    }
  | {
      readonly id: string;
      readonly kind: "merge-speakers";
      readonly sourceSpeakerId: string;
      readonly targetSpeakerId: string;
      readonly createdAt: string;
    }
  | {
      readonly id: string;
      readonly kind: "split-segment";
      readonly segmentId: string;
      readonly splitAtMs: number;
      readonly createdAt: string;
    }
  | {
      readonly id: string;
      readonly kind: "assign-range";
      readonly rangeStartMs: number;
      readonly rangeEndMs: number;
      readonly speakerId: string;
      readonly createdAt: string;
    }
  | {
      readonly id: string;
      readonly kind: "bulk-replace-speaker";
      readonly sourceSpeakerId: string;
      readonly targetSpeakerId: string;
      readonly createdAt: string;
    };

export type SpeakerCorrectionRecordKind = SpeakerCorrectionCommand["kind"] | "legacy-correction";

export interface SpeakerManualCorrectionRecord extends SpeakerCorrection {
  readonly commandId: string;
  readonly kind: SpeakerCorrectionRecordKind;
  readonly beforeSegments: readonly SpeakerSegment[];
  readonly afterSegments: readonly SpeakerSegment[];
}

export interface SpeakerCorrectionReplayState {
  readonly speakers: readonly SpeakerProfile[];
  readonly speakerSegments: readonly SpeakerSegment[];
  readonly corrections: readonly SpeakerCorrection[];
}

export interface SpeakerCorrectionReplayResult {
  readonly speakers: SpeakerProfile[];
  readonly speakerSegments: SpeakerSegment[];
  readonly corrections: SpeakerManualCorrectionRecord[];
}

export interface SpeakerDiarizationConflict {
  readonly segmentId: string;
  readonly startMs: number;
  readonly endMs: number;
  readonly currentSpeakerId: string;
  readonly generatedSpeakerId: string | null;
  readonly reason: "manual-edit-would-change";
}

export type SpeakerDiarizationRerunResolution =
  | {
      readonly status: "applied";
      readonly speakerSegments: SpeakerSegment[];
    }
  | {
      readonly status: "conflict";
      readonly speakerSegments: SpeakerSegment[];
      readonly generatedSegments: SpeakerSegment[];
      readonly conflicts: SpeakerDiarizationConflict[];
    };

export type ProjectSpeakerDiarizationRerunResolution =
  | {
      readonly status: "applied";
      readonly project: MeetingProject;
    }
  | {
      readonly status: "conflict";
      readonly project: MeetingProject;
      readonly generatedSegments: SpeakerSegment[];
      readonly conflicts: SpeakerDiarizationConflict[];
    };

export const UNKNOWN_SPEAKER_ID = "unknown";
export const UNKNOWN_SPEAKER_PROFILE: SpeakerProfile = {
  id: UNKNOWN_SPEAKER_ID,
  label: "Unknown speaker"
};

const DEFAULT_SPEAKER_MATCH_OPTIONS: Required<SpeakerMatchOptions> = {
  minOverlapMs: 50,
  minOverlapRatio: 0.1
};

export function renameSpeaker(speakers: readonly SpeakerProfile[], speakerId: string, label: string): SpeakerProfile[] {
  const normalized = label.trim();
  if (!normalized) {
    throw new Error("Speaker label cannot be empty");
  }

  return speakers.map((speaker) => (speaker.id === speakerId ? { ...speaker, label: normalized } : speaker));
}

export function mergeSpeakers(
  segments: readonly SpeakerSegment[],
  corrections: readonly SpeakerCorrection[],
  sourceSpeakerId: string,
  targetSpeakerId: string,
  createdAt: string
): { readonly segments: SpeakerSegment[]; readonly corrections: SpeakerCorrection[] } {
  if (sourceSpeakerId === targetSpeakerId) {
    return { segments: [...segments], corrections: [...corrections] };
  }

  return {
    segments: segments.map((segment) =>
      segment.speakerId === sourceSpeakerId ? { ...segment, speakerId: targetSpeakerId } : segment
    ),
    corrections: [
      ...corrections,
      {
        id: `correction-${createdAt}-${sourceSpeakerId}-to-${targetSpeakerId}`,
        targetSpeakerId: sourceSpeakerId,
        replacementSpeakerId: targetSpeakerId,
        reason: "manual-merge",
        createdAt
      }
    ]
  };
}

export function assignSpeakerForRange(
  segments: readonly SpeakerSegment[],
  rangeStartMs: number,
  rangeEndMs: number,
  speakerId: string
): SpeakerSegment[] {
  if (rangeEndMs <= rangeStartMs) {
    throw new Error("Speaker edit range must have positive duration");
  }

  return segments.map((segment) => {
    const overlaps = segment.startMs < rangeEndMs && segment.endMs > rangeStartMs;
    return overlaps ? { ...segment, speakerId } : segment;
  });
}

export function splitSpeakerSegment(
  segments: readonly SpeakerSegment[],
  segmentId: string,
  splitAtMs: number
): SpeakerSegment[] {
  const segmentToSplit = segments.find((segment) => segment.id === segmentId);
  if (segmentToSplit === undefined) {
    throw new Error(`Speaker segment not found: ${segmentId}`);
  }

  if (splitAtMs <= segmentToSplit.startMs || splitAtMs >= segmentToSplit.endMs) {
    throw new Error("Speaker split timestamp must be inside the segment");
  }

  return segments.flatMap((segment) => {
    if (segment.id !== segmentId) {
      return [{ ...segment }];
    }

    return [
      { ...segment, id: `${segment.id}-split-${splitAtMs}-a`, endMs: splitAtMs },
      { ...segment, id: `${segment.id}-split-${splitAtMs}-b`, startMs: splitAtMs }
    ];
  });
}

export function bulkReplaceSpeaker(
  segments: readonly SpeakerSegment[],
  sourceSpeakerId: string,
  targetSpeakerId: string
): SpeakerSegment[] {
  if (sourceSpeakerId === targetSpeakerId) {
    return segments.map((segment) => ({ ...segment }));
  }

  return segments.map((segment) =>
    segment.speakerId === sourceSpeakerId ? { ...segment, speakerId: targetSpeakerId } : { ...segment }
  );
}

export function applySpeakerCorrectionCommands(
  state: SpeakerCorrectionReplayState,
  commands: readonly SpeakerCorrectionCommand[]
): SpeakerCorrectionReplayResult {
  let speakers = state.speakers.map((speaker) => ({ ...speaker }));
  let speakerSegments = state.speakerSegments.map((segment) => ({ ...segment }));
  const corrections = state.corrections.map(normalizeCorrectionRecord);

  for (const command of commands) {
    if (command.kind === "rename-speaker") {
      speakers = renameSpeaker(speakers, command.speakerId, command.label);
      corrections.push(
        createCorrectionRecord(command, command.speakerId, command.speakerId, "manual-rename", [], [])
      );
      continue;
    }

    if (command.kind === "merge-speakers") {
      const beforeSegments = speakerSegments.filter((segment) => segment.speakerId === command.sourceSpeakerId);
      speakerSegments = bulkReplaceSpeaker(speakerSegments, command.sourceSpeakerId, command.targetSpeakerId);
      speakers = speakers
        .filter((speaker) => speaker.id !== command.sourceSpeakerId)
        .map((speaker) => ({ ...speaker }));
      corrections.push(
        createCorrectionRecord(
          command,
          command.sourceSpeakerId,
          command.targetSpeakerId,
          "manual-merge",
          beforeSegments,
          getSegmentsById(speakerSegments, beforeSegments)
        )
      );
      continue;
    }

    if (command.kind === "split-segment") {
      const beforeSegment = findSpeakerSegmentOrThrow(speakerSegments, command.segmentId);
      speakerSegments = splitSpeakerSegment(speakerSegments, command.segmentId, command.splitAtMs);
      corrections.push(
        createCorrectionRecord(
          command,
          beforeSegment.speakerId,
          beforeSegment.speakerId,
          "manual-split",
          [beforeSegment],
          speakerSegments.filter((segment) => segment.id.startsWith(`${command.segmentId}-split-${command.splitAtMs}-`))
        )
      );
      continue;
    }

    if (command.kind === "assign-range") {
      const beforeSegments = speakerSegments.filter((segment) =>
        rangesOverlap(segment.startMs, segment.endMs, command.rangeStartMs, command.rangeEndMs)
      );
      speakerSegments = assignSpeakerForRange(
        speakerSegments,
        command.rangeStartMs,
        command.rangeEndMs,
        command.speakerId
      ).map((segment) => ({ ...segment }));
      corrections.push(
        createCorrectionRecord(
          command,
          command.speakerId,
          command.speakerId,
          "manual-range-assign",
          beforeSegments,
          getSegmentsById(speakerSegments, beforeSegments)
        )
      );
      speakers = ensureUnknownSpeakerProfile(speakers, speakerSegments);
      continue;
    }

    const beforeSegments = speakerSegments.filter((segment) => segment.speakerId === command.sourceSpeakerId);
    speakerSegments = bulkReplaceSpeaker(speakerSegments, command.sourceSpeakerId, command.targetSpeakerId);
    corrections.push(
      createCorrectionRecord(
        command,
        command.sourceSpeakerId,
        command.targetSpeakerId,
        "manual-bulk-replace",
        beforeSegments,
        getSegmentsById(speakerSegments, beforeSegments)
      )
    );
  }

  return {
    speakers: ensureUnknownSpeakerProfile(speakers, speakerSegments),
    speakerSegments,
    corrections
  };
}

export function resolveDiarizationRerun(
  currentSegments: readonly SpeakerSegment[],
  generatedSegments: readonly SpeakerSegment[],
  corrections: readonly SpeakerCorrection[]
): SpeakerDiarizationRerunResolution {
  const manuallyEditedSegmentIds = new Set(
    corrections.flatMap((correction) => normalizeCorrectionRecord(correction).afterSegments.map((segment) => segment.id))
  );
  const generatedById = new Map(generatedSegments.map((segment) => [segment.id, segment]));
  const conflicts: SpeakerDiarizationConflict[] = [];

  for (const currentSegment of currentSegments) {
    if (!manuallyEditedSegmentIds.has(currentSegment.id)) {
      continue;
    }

    const generatedSegment = generatedById.get(currentSegment.id);
    if (generatedSegment !== undefined && segmentsAreEquivalent(currentSegment, generatedSegment)) {
      continue;
    }

    conflicts.push({
      segmentId: currentSegment.id,
      startMs: currentSegment.startMs,
      endMs: currentSegment.endMs,
      currentSpeakerId: currentSegment.speakerId,
      generatedSpeakerId: generatedSegment?.speakerId ?? null,
      reason: "manual-edit-would-change"
    });
  }

  if (conflicts.length > 0) {
    return {
      status: "conflict",
      speakerSegments: currentSegments.map((segment) => ({ ...segment })),
      generatedSegments: generatedSegments.map((segment) => ({ ...segment })),
      conflicts
    };
  }

  return {
    status: "applied",
    speakerSegments: generatedSegments.map((segment) => ({ ...segment }))
  };
}

export function resolveProjectDiarizationRerun(
  project: MeetingProject,
  generatedSegments: readonly SpeakerSegment[]
): ProjectSpeakerDiarizationRerunResolution {
  const resolution = resolveDiarizationRerun(project.speakerSegments, generatedSegments, project.corrections);

  if (resolution.status === "conflict") {
    return {
      status: "conflict",
      project,
      generatedSegments: resolution.generatedSegments,
      conflicts: resolution.conflicts
    };
  }

  return {
    status: "applied",
    project: {
      ...project,
      speakerSegments: resolution.speakerSegments
    }
  };
}

export function mergeTranscriptWithDiarization(
  transcript: readonly TranscriptSegment[],
  diarizationSegments: readonly SpeakerDiarizationSegment[],
  options: SpeakerMatchOptions = {}
): SpeakerMergedTranscriptSegment[] {
  const matchOptions = normalizeMatchOptions(options);
  return transcript.map((segment) => {
    const match = findBestDiarizationMatch(segment, diarizationSegments, matchOptions);
    return {
      id: segment.id,
      text: segment.text,
      language: segment.language,
      speakerId: match?.speakerId ?? UNKNOWN_SPEAKER_ID,
      startMs: segment.startMs,
      endMs: segment.endMs,
      ...(match?.confidence === undefined ? {} : { confidence: match.confidence })
    };
  });
}

export function ensureUnknownSpeakerProfile(
  speakers: readonly SpeakerProfile[],
  speakerSegments: readonly Pick<SpeakerSegment, "speakerId">[]
): SpeakerProfile[] {
  const hasUnknownSegment = speakerSegments.some((segment) => segment.speakerId === UNKNOWN_SPEAKER_ID);
  const hasUnknownProfile = speakers.some((speaker) => speaker.id === UNKNOWN_SPEAKER_ID);

  if (!hasUnknownSegment || hasUnknownProfile) {
    return speakers.map((speaker) => ({ ...speaker }));
  }

  return [...speakers.map((speaker) => ({ ...speaker })), { ...UNKNOWN_SPEAKER_PROFILE }];
}

function findBestDiarizationMatch(
  transcriptSegment: TranscriptSegment,
  diarizationSegments: readonly SpeakerDiarizationSegment[],
  options: Required<SpeakerMatchOptions>
): SpeakerDiarizationSegment | null {
  let bestSegment: SpeakerDiarizationSegment | null = null;
  let bestOverlap = 0;
  const transcriptDuration = transcriptSegment.endMs - transcriptSegment.startMs;

  for (const candidate of diarizationSegments) {
    const overlap = getOverlapMs(transcriptSegment.startMs, transcriptSegment.endMs, candidate.startMs, candidate.endMs);
    const overlapRatio = transcriptDuration > 0 ? overlap / transcriptDuration : 0;
    if (overlap < options.minOverlapMs || overlapRatio < options.minOverlapRatio) {
      continue;
    }

    if (overlap > bestOverlap) {
      bestSegment = candidate;
      bestOverlap = overlap;
      continue;
    }

    if (overlap === bestOverlap && bestSegment !== null && candidate.confidence !== undefined && bestSegment.confidence !== undefined) {
      if (candidate.confidence > bestSegment.confidence) {
        bestSegment = candidate;
      }
      continue;
    }

    if (overlap === bestOverlap && bestSegment !== null && bestSegment.confidence === undefined && candidate.confidence !== undefined) {
      bestSegment = candidate;
      continue;
    }

    if (overlap === bestOverlap && bestSegment !== null && candidate.startMs < bestSegment.startMs) {
      bestSegment = candidate;
    }
  }

  return bestSegment;
}

function normalizeMatchOptions(options: SpeakerMatchOptions): Required<SpeakerMatchOptions> {
  return {
    minOverlapMs: options.minOverlapMs ?? DEFAULT_SPEAKER_MATCH_OPTIONS.minOverlapMs,
    minOverlapRatio: options.minOverlapRatio ?? DEFAULT_SPEAKER_MATCH_OPTIONS.minOverlapRatio
  };
}

function normalizeCorrectionRecord(correction: SpeakerCorrection): SpeakerManualCorrectionRecord {
  if (isManualCorrectionRecord(correction)) {
    return cloneCorrectionRecord(correction);
  }

  return {
    ...correction,
    commandId: correction.id,
    kind: "legacy-correction",
    beforeSegments: [],
    afterSegments: []
  };
}

function createCorrectionRecord(
  command: SpeakerCorrectionCommand,
  targetSpeakerId: string,
  replacementSpeakerId: string,
  reason: string,
  beforeSegments: readonly SpeakerSegment[],
  afterSegments: readonly SpeakerSegment[]
): SpeakerManualCorrectionRecord {
  return {
    id: command.id,
    commandId: command.id,
    kind: command.kind,
    targetSpeakerId,
    replacementSpeakerId,
    reason,
    createdAt: command.createdAt,
    beforeSegments: beforeSegments.map((segment) => ({ ...segment })),
    afterSegments: afterSegments.map((segment) => ({ ...segment }))
  };
}

function cloneCorrectionRecord(correction: SpeakerManualCorrectionRecord): SpeakerManualCorrectionRecord {
  return {
    ...correction,
    beforeSegments: correction.beforeSegments.map((segment) => ({ ...segment })),
    afterSegments: correction.afterSegments.map((segment) => ({ ...segment }))
  };
}

function isManualCorrectionRecord(correction: SpeakerCorrection): correction is SpeakerManualCorrectionRecord {
  const candidate = correction as Partial<SpeakerManualCorrectionRecord>;
  return Array.isArray(candidate.beforeSegments) && Array.isArray(candidate.afterSegments);
}

function getSegmentsById(
  segments: readonly SpeakerSegment[],
  sourceSegments: readonly Pick<SpeakerSegment, "id">[]
): SpeakerSegment[] {
  const sourceIds = new Set(sourceSegments.map((segment) => segment.id));
  return segments.filter((segment) => sourceIds.has(segment.id)).map((segment) => ({ ...segment }));
}

function findSpeakerSegmentOrThrow(segments: readonly SpeakerSegment[], segmentId: string): SpeakerSegment {
  const segment = segments.find((candidate) => candidate.id === segmentId);
  if (segment === undefined) {
    throw new Error(`Speaker segment not found: ${segmentId}`);
  }

  return { ...segment };
}

function rangesOverlap(
  leftStartMs: number,
  leftEndMs: number,
  rightStartMs: number,
  rightEndMs: number
): boolean {
  return leftStartMs < rightEndMs && leftEndMs > rightStartMs;
}

function segmentsAreEquivalent(left: SpeakerSegment, right: SpeakerSegment): boolean {
  return left.speakerId === right.speakerId && left.startMs === right.startMs && left.endMs === right.endMs;
}

function getOverlapMs(
  leftStartMs: number,
  leftEndMs: number,
  rightStartMs: number,
  rightEndMs: number
): number {
  return Math.max(0, Math.min(leftEndMs, rightEndMs) - Math.max(leftStartMs, rightStartMs));
}
