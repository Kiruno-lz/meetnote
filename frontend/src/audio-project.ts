export const SUPPORTED_AUDIO_EXTENSIONS = [".wav", ".mp3", ".m4a"] as const;

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
  readonly sourceSegmentIds: readonly string[];
  readonly speakerId: string;
  readonly text: string;
}

export interface MeetingProject {
  readonly id: string;
  readonly title: string;
  readonly audioPath: string;
  readonly audioHash: string;
  readonly durationSeconds: number;
  readonly format: SupportedAudioExtension;
  readonly createdAt: string;
  readonly transcript: readonly TranscriptSegment[];
  readonly speakerSegments: readonly SpeakerSegment[];
  readonly speakers: readonly SpeakerProfile[];
  readonly corrections: readonly SpeakerCorrection[];
  readonly deverbalizedSegments: readonly DeverbalizedSegment[];
  readonly editedMarkdown: string | null;
}

export function getSupportedAudioExtension(path: string): SupportedAudioExtension | null {
  const normalized = path.trim().toLowerCase();
  return SUPPORTED_AUDIO_EXTENSIONS.find((extension) => normalized.endsWith(extension)) ?? null;
}

export function createMeetingProject(input: AudioImportInput): MeetingProject {
  const format = getSupportedAudioExtension(input.path);
  if (!format) {
    throw new Error(`Unsupported audio format: ${input.path}`);
  }

  const fallbackTitle = input.path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") || "Untitled meeting";

  return {
    id: `audio-${input.hash}`,
    title: input.title?.trim() || fallbackTitle,
    audioPath: input.path,
    audioHash: input.hash,
    durationSeconds: input.durationSeconds,
    format,
    createdAt: input.createdAt,
    transcript: [],
    speakerSegments: [],
    speakers: [],
    corrections: [],
    deverbalizedSegments: [],
    editedMarkdown: null
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
