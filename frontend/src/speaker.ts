import type { SpeakerCorrection, SpeakerProfile, SpeakerSegment } from "./audio-project";

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
