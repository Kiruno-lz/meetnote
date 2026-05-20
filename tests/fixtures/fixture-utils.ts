import { writeFileSync, mkdirSync } from "fs";
import { resolve } from "path";

// Helper types and utilities for fixture generation
export interface FixtureData {
  audioMetadata: {
    filename: string;
    durationMs: number;
    audioHash: string;
    uploadedAt: string;
  };
  asrResult: {
    requestId: string;
    model: { id: string; version: string };
    segments: Array<{
      id: string;
      startMs: number;
      endMs: number;
      text: string;
      language: "zh" | "en" | "mixed";
      confidence: number;
    }>;
  };
  diarizationResult: {
    requestId: string;
    model: { id: string; version: string };
    speakers: Array<{ id: string; label: string }>;
    segments: Array<{
      id: string;
      speakerId: string;
      startMs: number;
      endMs: number;
      confidence: number;
      overlap: boolean;
    }>;
  };
  speakerCorrections?: {
    timestamp: string;
    corrections: Array<{
      originalLabel: string;
      newLabel: string;
      speakerId: string;
    }>;
  };
  deverbalizationOutput: {
    requestId: string;
    provider: "ollama" | "custom";
    model: { id: string; version: string };
    paragraphs: Array<{
      id: string;
      sourceSegmentIds: string[];
      speakerId: string;
      text: string;
    }>;
  };
  expectedMarkdownFrontmatter: {
    title: string;
    date: string;
    audioHash: string;
    speakers: string[];
    modelVersion: string;
    llmProvider: "ollama" | "custom";
  };
}

export function saveFixture(name: string, data: FixtureData): void {
  const dir = resolve(__dirname, "../fixtures");
  mkdirSync(dir, { recursive: true });
  const path = resolve(dir, `${name}.json`);
  writeFileSync(path, JSON.stringify(data, null, 2), "utf-8");
  console.log(`Saved fixture: ${path}`);
}

// Utility functions for common fixture generation tasks

export function generateAsrSegmentId(index: number): string {
  return `asr-seg-${String(index).padStart(4, "0")}`;
}

export function generateDiarizationSegmentId(index: number): string {
  return `dia-seg-${String(index).padStart(4, "0")}`;
}

export function generateParagraphId(index: number): string {
  return `para-${String(index).padStart(4, "0")}`;
}

export function generateSpeakerId(label: string): string {
  return `speaker-${label.toLowerCase().replace(/\s+/g, "-")}`;
}

export function generateAudioHash(): string {
  // Generate random 16-char hex string (sha256 prefix simulation)
  return Array.from({ length: 16 })
    .map(() => Math.floor(Math.random() * 16).toString(16))
    .join("");
}

export function generateRequestId(prefix: string): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).substring(7)}`;
}

export function formatTimestamp(date: Date = new Date()): string {
  return date.toISOString();
}

export function createTimeRange(startMs: number, endMs: number, segments: number): number[] {
  const times: number[] = [startMs];
  const segmentDuration = (endMs - startMs) / segments;

  for (let i = 1; i <= segments; i++) {
    times.push(startMs + Math.floor(segmentDuration * i));
  }

  return times;
}

export function validateFixture(data: FixtureData): string[] {
  const errors: string[] = [];

  // Validate audio hash format
  if (!/^[0-9a-f]{16}$/.test(data.audioMetadata.audioHash)) {
    errors.push(
      `Invalid audioHash format: "${data.audioMetadata.audioHash}" (must be 16 lowercase hex chars)`
    );
  }

  // Validate ASR segments
  const asrSegments = data.asrResult.segments;
  if (asrSegments.length === 0) {
    errors.push("ASR segments must not be empty");
  }

  asrSegments.forEach((seg, i) => {
    if (seg.endMs <= seg.startMs) {
      errors.push(`ASR segment ${i} has invalid time range: ${seg.startMs}-${seg.endMs}`);
    }
    if (seg.confidence < 0 || seg.confidence > 1) {
      errors.push(`ASR segment ${i} has invalid confidence: ${seg.confidence}`);
    }
  });

  // Validate diarization segments
  const diaSegments = data.diarizationResult.segments;
  const speakerIds = new Set(data.diarizationResult.speakers.map(s => s.id));

  diaSegments.forEach((seg, i) => {
    if (!speakerIds.has(seg.speakerId)) {
      errors.push(`Diarization segment ${i} references unknown speaker: ${seg.speakerId}`);
    }
    if (seg.confidence < 0 || seg.confidence > 1) {
      errors.push(`Diarization segment ${i} has invalid confidence: ${seg.confidence}`);
    }
  });

  // Validate paragraphs
  const allAsrSegmentIds = new Set(asrSegments.map(s => s.id));
  const allSpeakerIds = new Set(data.diarizationResult.speakers.map(s => s.id));

  data.deverbalizationOutput.paragraphs.forEach((para, i) => {
    para.sourceSegmentIds.forEach(segId => {
      if (!allAsrSegmentIds.has(segId)) {
        errors.push(`Paragraph ${i} references unknown ASR segment: ${segId}`);
      }
    });
    if (!allSpeakerIds.has(para.speakerId)) {
      errors.push(`Paragraph ${i} references unknown speaker: ${para.speakerId}`);
    }
  });

  // Validate frontmatter
  const frontmatterSpeakers = new Set(data.expectedMarkdownFrontmatter.speakers);
  data.diarizationResult.speakers.forEach(speaker => {
    if (!frontmatterSpeakers.has(speaker.label)) {
      errors.push(`Speaker label not in frontmatter speakers: ${speaker.label}`);
    }
  });

  return errors;
}
