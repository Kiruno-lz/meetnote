export const SUPPORTED_AUDIO_EXTENSIONS = [".wav", ".mp3", ".m4a"] as const;
export const PROJECT_MANIFEST_SCHEMA_VERSION = 1;

export type SupportedAudioExtension = (typeof SUPPORTED_AUDIO_EXTENSIONS)[number];

export interface AudioImportInput {
  readonly path: string;
  readonly hash: string;
  readonly durationSeconds: number;
  readonly createdAt: string;
  readonly title?: string;
}

export interface TranscriptSegment {
  readonly id: string;
  readonly startMs: number;
  readonly endMs: number;
  readonly text: string;
  readonly language: "zh" | "en" | "mixed" | "unknown";
}

export interface SpeakerSegment {
  readonly id: string;
  readonly speakerId: string;
  readonly startMs: number;
  readonly endMs: number;
  readonly confidence?: number;
}

export interface SpeakerProfile {
  readonly id: string;
  readonly label: string;
}

export interface SpeakerCorrection {
  readonly id: string;
  readonly targetSpeakerId: string;
  readonly replacementSpeakerId: string;
  readonly reason: string;
  readonly createdAt: string;
}

export interface DeverbalizedSegment {
  readonly id: string;
  readonly sourceSegmentIds: readonly string[];
  readonly speakerId: string;
  readonly text: string;
  readonly createdAt: string;
  readonly provider: string;
}

export type EditHistoryKind = "title-change" | "speaker-correction" | "llm-deverbalized" | "markdown-edit";

export interface EditHistoryEntry {
  readonly id: string;
  readonly kind: EditHistoryKind;
  readonly summary: string;
  readonly createdAt: string;
  readonly actor: "user" | "system";
}

export interface MeetingProject {
  readonly schemaVersion: typeof PROJECT_MANIFEST_SCHEMA_VERSION;
  readonly id: string;
  readonly title: string;
  readonly audioPath: string;
  readonly audioHash: string;
  readonly durationSeconds: number;
  readonly format: SupportedAudioExtension;
  readonly createdAt: string;
  readonly originalTranscript: readonly TranscriptSegment[];
  readonly transcript: readonly TranscriptSegment[];
  readonly speakerSegments: readonly SpeakerSegment[];
  readonly speakers: readonly SpeakerProfile[];
  readonly corrections: readonly SpeakerCorrection[];
  readonly deverbalizedSegments: readonly DeverbalizedSegment[];
  readonly editedMarkdown: string | null;
  readonly editHistory: readonly EditHistoryEntry[];
}

export type AudioImportPlan =
  | {
      readonly action: "reuse-existing";
      readonly projectId: string;
      readonly reason: "duplicate-audio-hash";
    }
  | {
      readonly action: "create-new";
      readonly projectId: string;
      readonly reason: "new-audio-hash";
    };

export interface ManifestValidationResult {
  readonly ok: boolean;
  readonly errors: readonly string[];
}

export function getSupportedAudioExtension(path: string): SupportedAudioExtension | null {
  const normalized = path.trim().toLowerCase();
  return SUPPORTED_AUDIO_EXTENSIONS.find((extension) => normalized.endsWith(extension)) ?? null;
}

export function planAudioImport(
  existingProjects: readonly Pick<MeetingProject, "id" | "audioHash" | "createdAt" | "audioPath" | "title">[],
  input: AudioImportInput
): AudioImportPlan {
  assertValidAudioImportInput(input);
  const existing = resolveCanonicalExistingProject(existingProjects, input.hash);
  if (existing) {
    return {
      action: "reuse-existing",
      projectId: existing.id,
      reason: "duplicate-audio-hash"
    };
  }

  return {
    action: "create-new",
    projectId: createProjectId(input.hash),
    reason: "new-audio-hash"
  };
}

export function createMeetingProject(input: AudioImportInput): MeetingProject {
  const { format, normalizedHash } = assertValidAudioImportInput(input);

  const fallbackTitle = input.path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") || "Untitled meeting";

  return {
    schemaVersion: PROJECT_MANIFEST_SCHEMA_VERSION,
    id: createProjectId(normalizedHash),
    title: input.title?.trim() || fallbackTitle,
    audioPath: input.path,
    audioHash: normalizedHash,
    durationSeconds: input.durationSeconds,
    format,
    createdAt: input.createdAt,
    originalTranscript: [],
    transcript: [],
    speakerSegments: [],
    speakers: [],
    corrections: [],
    deverbalizedSegments: [],
    editedMarkdown: null,
    editHistory: []
  };
}

export function createMeetingProjectManifest(project: MeetingProject): MeetingProject {
  return restoreMeetingProjectManifest(JSON.parse(JSON.stringify(project)) as unknown);
}

export function restoreMeetingProjectManifest(input: unknown): MeetingProject {
  const result = validateMeetingProjectManifest(input);
  if (!result.ok) {
    throw new Error(`Invalid meeting project manifest: ${result.errors.join("; ")}`);
  }

  const manifest = input as MeetingProject;
  return {
    ...manifest,
    originalTranscript: cloneTranscript(manifest.originalTranscript),
    transcript: cloneTranscript(manifest.transcript),
    speakerSegments: manifest.speakerSegments.map((segment) => ({ ...segment })),
    speakers: manifest.speakers.map((speaker) => ({ ...speaker })),
    corrections: manifest.corrections.map((correction) => ({ ...correction })),
    deverbalizedSegments: manifest.deverbalizedSegments.map((segment) => ({
      ...segment,
      sourceSegmentIds: [...segment.sourceSegmentIds]
    })),
    editHistory: manifest.editHistory.map((entry) => ({ ...entry }))
  };
}

export function resolveImportedMeetingProject(
  existingProjects: readonly MeetingProject[],
  input: AudioImportInput
): MeetingProject {
  const existing = resolveCanonicalExistingProject(existingProjects, input.hash);
  return existing ?? createMeetingProject(input);
}

export function attachTranscript(
  project: MeetingProject,
  transcript: readonly TranscriptSegment[]
): MeetingProject {
  const nextTranscript = cloneTranscript(transcript);
  const originalTranscript = project.originalTranscript.length === 0 ? cloneTranscript(transcript) : cloneTranscript(project.originalTranscript);
  return {
    ...project,
    originalTranscript,
    transcript: nextTranscript
  };
}

export function attachSpeakerState(
  project: MeetingProject,
  speakers: readonly SpeakerProfile[],
  speakerSegments: readonly SpeakerSegment[],
  corrections: readonly SpeakerCorrection[]
): MeetingProject {
  return {
    ...project,
    speakers: [...speakers],
    speakerSegments: [...speakerSegments],
    corrections: [...corrections]
  };
}

export function attachDeverbalizedState(
  project: MeetingProject,
  deverbalizedSegments: readonly DeverbalizedSegment[]
): MeetingProject {
  return {
    ...project,
    deverbalizedSegments: deverbalizedSegments.map((segment) => ({
      ...segment,
      sourceSegmentIds: [...segment.sourceSegmentIds]
    }))
  };
}

export function attachEditedMarkdown(project: MeetingProject, editedMarkdown: string): MeetingProject {
  return {
    ...project,
    editedMarkdown
  };
}

export function addEditHistoryEntry(project: MeetingProject, entry: EditHistoryEntry): MeetingProject {
  return {
    ...project,
    editHistory: [...project.editHistory, { ...entry }]
  };
}

function resolveCanonicalExistingProject<T extends Pick<MeetingProject, "id" | "audioHash" | "createdAt" | "audioPath" | "title">>(
  existingProjects: readonly T[],
  audioHash: string
): T | null {
  const candidates = existingProjects.filter((project) => project.audioHash === audioHash.trim());
  if (candidates.length === 0) {
    return null;
  }

  return [...candidates].sort(compareCanonicalProject)[0] ?? null;
}

function compareCanonicalProject(
  left: Pick<MeetingProject, "createdAt" | "id" | "audioPath" | "title">,
  right: Pick<MeetingProject, "createdAt" | "id" | "audioPath" | "title">
): number {
  return (
    left.createdAt.localeCompare(right.createdAt) ||
    left.id.localeCompare(right.id) ||
    left.audioPath.localeCompare(right.audioPath) ||
    left.title.localeCompare(right.title)
  );
}

export function validateMeetingProjectManifest(input: unknown): ManifestValidationResult {
  const errors: string[] = [];

  if (!isRecord(input)) {
    return { ok: false, errors: ["manifest must be an object"] };
  }

  if (input.schemaVersion !== PROJECT_MANIFEST_SCHEMA_VERSION) {
    errors.push("schemaVersion must be 1");
  }

  const audioHash = input.audioHash;
  if (!isNonEmptyString(audioHash)) {
    errors.push("audioHash is required");
  }

  if (!isNonEmptyString(input.id)) {
    errors.push("id is required");
  } else if (isNonEmptyString(audioHash) && input.id !== createProjectId(audioHash)) {
    errors.push("id must be derived from audioHash");
  }

  if (!isNonEmptyString(input.title)) {
    errors.push("title is required");
  }
  if (!isNonEmptyString(input.audioPath)) {
    errors.push("audioPath is required");
  }
  if (!SUPPORTED_AUDIO_EXTENSIONS.includes(input.format as SupportedAudioExtension)) {
    errors.push("format must be a supported audio extension");
  }
  if (!isPositiveNumber(input.durationSeconds)) {
    errors.push("durationSeconds must be positive");
  }
  if (!isNonEmptyString(input.createdAt)) {
    errors.push("createdAt is required");
  }

  validateTranscriptArray(input.originalTranscript, "originalTranscript", errors);
  validateTranscriptArray(input.transcript, "transcript", errors);
  validateSpeakerSegments(input.speakerSegments, errors);
  validateSpeakers(input.speakers, errors);
  validateCorrections(input.corrections, errors);
  validateDeverbalizedSegments(input.deverbalizedSegments, errors);
  validateEditHistory(input.editHistory, errors);
  validateProjectReferences(input, errors);

  if (input.editedMarkdown !== null && typeof input.editedMarkdown !== "string") {
    errors.push("editedMarkdown must be a string or null");
  }

  return {
    ok: errors.length === 0,
    errors
  };
}

function assertValidAudioImportInput(input: AudioImportInput): {
  readonly format: SupportedAudioExtension;
  readonly normalizedHash: string;
} {
  const format = getSupportedAudioExtension(input.path);
  if (!format) {
    throw new Error(`Unsupported audio format: ${input.path}`);
  }

  const normalizedHash = input.hash.trim();
  if (!normalizedHash) {
    throw new Error("Audio hash is required");
  }
  if (!isPositiveNumber(input.durationSeconds)) {
    throw new Error("Audio duration must be positive");
  }
  if (!input.createdAt.trim()) {
    throw new Error("Audio createdAt is required");
  }

  return { format, normalizedHash };
}

function createProjectId(audioHash: string): string {
  return `audio-${audioHash.trim()}`;
}

function cloneTranscript(transcript: readonly TranscriptSegment[]): TranscriptSegment[] {
  return transcript.map((segment) => ({ ...segment }));
}

function validateTranscriptArray(input: unknown, fieldName: string, errors: string[]): void {
  if (!Array.isArray(input)) {
    errors.push(`${fieldName} must be an array`);
    return;
  }

  input.forEach((segment, index) => {
    if (!isRecord(segment)) {
      errors.push(`${fieldName}[${index}] must be an object`);
      return;
    }
    if (!isNonEmptyString(segment.id)) {
      errors.push(`${fieldName}[${index}].id is required`);
    }
    if (typeof segment.startMs !== "number" || segment.startMs < 0) {
      errors.push(`${fieldName}[${index}].startMs must be non-negative`);
    }
    if (typeof segment.endMs !== "number" || segment.endMs <= Number(segment.startMs)) {
      errors.push(`${fieldName}[${index}].endMs must be greater than startMs`);
    }
    if (!isNonEmptyString(segment.text)) {
      errors.push(`${fieldName}[${index}].text is required`);
    }
    if (!["zh", "en", "mixed", "unknown"].includes(String(segment.language))) {
      errors.push(`${fieldName}[${index}].language is invalid`);
    }
  });
}

function isRecord(input: unknown): input is Record<string, unknown> {
  return typeof input === "object" && input !== null && !Array.isArray(input);
}

function isNonEmptyString(input: unknown): input is string {
  return typeof input === "string" && input.trim().length > 0;
}

function isPositiveNumber(input: unknown): input is number {
  return typeof input === "number" && Number.isFinite(input) && input > 0;
}

function validateSpeakerSegments(input: unknown, errors: string[]): void {
  if (!Array.isArray(input)) {
    errors.push("speakerSegments must be an array");
    return;
  }

  input.forEach((segment, index) => {
    if (!isRecord(segment)) {
      errors.push(`speakerSegments[${index}] must be an object`);
      return;
    }
    if (!isNonEmptyString(segment.id)) {
      errors.push(`speakerSegments[${index}].id is required`);
    }
    if (!isNonEmptyString(segment.speakerId)) {
      errors.push(`speakerSegments[${index}].speakerId is required`);
    }
    const startMs = segment.startMs;
    const endMs = segment.endMs;
    const hasValidStartMs = isPositiveOrZeroNumber(startMs);
    if (!hasValidStartMs) {
      errors.push(`speakerSegments[${index}].startMs must be non-negative`);
    }
    if (hasValidStartMs && typeof startMs === "number" && (!isPositiveNumber(endMs) || endMs <= startMs)) {
      errors.push(`speakerSegments[${index}].endMs must be greater than startMs`);
    }
    if (segment.confidence !== undefined && !isConfidenceValue(segment.confidence)) {
      errors.push(`speakerSegments[${index}].confidence must be between 0 and 1`);
    }
  });
}

function validateSpeakers(input: unknown, errors: string[]): void {
  if (!Array.isArray(input)) {
    errors.push("speakers must be an array");
    return;
  }

  input.forEach((speaker, index) => {
    if (!isRecord(speaker)) {
      errors.push(`speakers[${index}] must be an object`);
      return;
    }
    if (!isNonEmptyString(speaker.id)) {
      errors.push(`speakers[${index}].id is required`);
    }
    if (!isNonEmptyString(speaker.label)) {
      errors.push(`speakers[${index}].label is required`);
    }
  });
}

function validateCorrections(input: unknown, errors: string[]): void {
  if (!Array.isArray(input)) {
    errors.push("corrections must be an array");
    return;
  }

  input.forEach((correction, index) => {
    if (!isRecord(correction)) {
      errors.push(`corrections[${index}] must be an object`);
      return;
    }
    if (!isNonEmptyString(correction.id)) {
      errors.push(`corrections[${index}].id is required`);
    }
    if (!isNonEmptyString(correction.targetSpeakerId)) {
      errors.push(`corrections[${index}].targetSpeakerId is required`);
    }
    if (!isNonEmptyString(correction.replacementSpeakerId)) {
      errors.push(`corrections[${index}].replacementSpeakerId is required`);
    }
    if (!isNonEmptyString(correction.reason)) {
      errors.push(`corrections[${index}].reason is required`);
    }
    if (!isNonEmptyString(correction.createdAt)) {
      errors.push(`corrections[${index}].createdAt is required`);
    }
  });
}

function validateDeverbalizedSegments(input: unknown, errors: string[]): void {
  if (!Array.isArray(input)) {
    errors.push("deverbalizedSegments must be an array");
    return;
  }

  input.forEach((segment, index) => {
    if (!isRecord(segment)) {
      errors.push(`deverbalizedSegments[${index}] must be an object`);
      return;
    }
    if (!isNonEmptyString(segment.id)) {
      errors.push(`deverbalizedSegments[${index}].id is required`);
    }
    if (!Array.isArray(segment.sourceSegmentIds) || segment.sourceSegmentIds.length === 0) {
      errors.push(`deverbalizedSegments[${index}].sourceSegmentIds must not be empty`);
    } else {
      segment.sourceSegmentIds.forEach((segmentId, segmentIdIndex) => {
        if (!isNonEmptyString(segmentId)) {
          errors.push(`deverbalizedSegments[${index}].sourceSegmentIds[${segmentIdIndex}] must be a non-empty string`);
        }
      });
    }
    if (!isNonEmptyString(segment.speakerId)) {
      errors.push(`deverbalizedSegments[${index}].speakerId is required`);
    }
    if (!isNonEmptyString(segment.text)) {
      errors.push(`deverbalizedSegments[${index}].text is required`);
    }
    if (!isNonEmptyString(segment.createdAt)) {
      errors.push(`deverbalizedSegments[${index}].createdAt is required`);
    }
    if (!isNonEmptyString(segment.provider)) {
      errors.push(`deverbalizedSegments[${index}].provider is required`);
    }
  });
}

function validateProjectReferences(input: Record<string, unknown>, errors: string[]): void {
  if (!Array.isArray(input.speakers) || !Array.isArray(input.transcript)) {
    return;
  }

  const speakerIds = new Set(
    input.speakers
      .filter(isRecord)
      .map((speaker) => speaker.id)
      .filter(isNonEmptyString)
  );
  const transcriptIds = new Set(
    input.transcript
      .filter(isRecord)
      .map((segment) => segment.id)
      .filter(isNonEmptyString)
  );

  if (Array.isArray(input.speakerSegments)) {
    input.speakerSegments.forEach((segment, index) => {
      if (isRecord(segment) && isNonEmptyString(segment.speakerId) && !speakerIds.has(segment.speakerId)) {
        errors.push(`speakerSegments[${index}].speakerId must reference a known speaker`);
      }
    });
  }

  if (Array.isArray(input.deverbalizedSegments)) {
    input.deverbalizedSegments.forEach((segment, index) => {
      if (!isRecord(segment)) {
        return;
      }

      if (isNonEmptyString(segment.speakerId) && !speakerIds.has(segment.speakerId)) {
        errors.push(`deverbalizedSegments[${index}].speakerId must reference a known speaker`);
      }

      if (Array.isArray(segment.sourceSegmentIds)) {
        segment.sourceSegmentIds.forEach((segmentId, segmentIdIndex) => {
          if (isNonEmptyString(segmentId) && !transcriptIds.has(segmentId)) {
            errors.push(`deverbalizedSegments[${index}].sourceSegmentIds[${segmentIdIndex}] must reference a known transcript segment`);
          }
        });
      }
    });
  }
}

function validateEditHistory(input: unknown, errors: string[]): void {
  if (!Array.isArray(input)) {
    errors.push("editHistory must be an array");
    return;
  }

  input.forEach((entry, index) => {
    if (!isRecord(entry)) {
      errors.push(`editHistory[${index}] must be an object`);
      return;
    }
    if (!isNonEmptyString(entry.id)) {
      errors.push(`editHistory[${index}].id is required`);
    }
    if (!["title-change", "speaker-correction", "llm-deverbalized", "markdown-edit"].includes(String(entry.kind))) {
      errors.push(`editHistory[${index}].kind is invalid`);
    }
    if (!isNonEmptyString(entry.summary)) {
      errors.push(`editHistory[${index}].summary is required`);
    }
    if (!isNonEmptyString(entry.createdAt)) {
      errors.push(`editHistory[${index}].createdAt is required`);
    }
    if (!["user", "system"].includes(String(entry.actor))) {
      errors.push(`editHistory[${index}].actor is invalid`);
    }
  });
}

function isPositiveOrZeroNumber(input: unknown): input is number {
  return typeof input === "number" && Number.isFinite(input) && input >= 0;
}

function isConfidenceValue(input: unknown): input is number {
  return typeof input === "number" && Number.isFinite(input) && input >= 0 && input <= 1;
}
