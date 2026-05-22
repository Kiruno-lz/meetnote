// Mock pipeline: AudioImport → MockASR → MockDiarization → SpeakerMerge → SpeakerCorrectionReplay → MockLLM → MarkdownGenerate
import {
  createMeetingProject,
  attachTranscript,
  attachSpeakerState,
  type MeetingProject,
  type SpeakerProfile,
  type SpeakerSegment,
  type TranscriptSegment
} from "./audio-project";
import { mergeTranscriptWithDiarization, applySpeakerCorrectionCommands, type SpeakerCorrectionCommand } from "./speaker";
import { projectMeetingCorrectedSpeakerOutput, createCorrectedSpeakerLlmInput, renderCorrectedSpeakerLlmInput } from "./speaker-output";
import { createLlmRequestFromCorrectedSpeakerInput, runDeterministicMockLlmProvider } from "./llm-provider";
import { generateMarkdownContent, type GenerateMarkdownOptions } from "./markdown-library-generator";
import { createMarkdownFrontmatter, MARKDOWN_LIBRARY_SCHEMA_VERSION } from "./markdown-library-schema";
import type { DeverbalizationOutput } from "./llm-schema";

export interface MockPipelineInput {
  audioPath: string;
  audioHash: string;
  durationSeconds: number;
  title: string;
  mockTranscriptSegments: TranscriptSegment[];
  mockSpeakerSegments: SpeakerSegment[];
  mockSpeakerProfiles?: SpeakerProfile[];
  speakerCorrectionCommands?: SpeakerCorrectionCommand[];
}

export interface MockPipelineResult {
  project: MeetingProject;
  markdownContent: string;
  speakerProfiles: SpeakerProfile[];
}

export function runMockPipeline(input: MockPipelineInput): MockPipelineResult {
  const createdAt = "2024-01-01T00:00:00.000Z";

  // Step 1: Create project (AudioImport)
  const baseProject = createMeetingProject({
    path: input.audioPath,
    hash: input.audioHash,
    durationSeconds: input.durationSeconds,
    createdAt,
    title: input.title
  });

  // Step 2: Attach transcript segments (MockASR)
  const projectWithTranscript = attachTranscript(baseProject, input.mockTranscriptSegments);

  // Step 3: Attach speaker state (MockDiarization)
  // Merge transcript with diarization to derive correct speaker assignments
  const mergedSegments = mergeTranscriptWithDiarization(
    input.mockTranscriptSegments,
    input.mockSpeakerSegments
  );

  // Build initial speaker profiles from provided profiles or infer from segments
  const initialSpeakerProfiles: SpeakerProfile[] =
    input.mockSpeakerProfiles && input.mockSpeakerProfiles.length > 0
      ? input.mockSpeakerProfiles
      : deriveUniqueProfiles(input.mockSpeakerSegments);

  // Convert merged transcript segments to speaker segments for the project
  const derivedSpeakerSegments: SpeakerSegment[] = mergedSegments.map((seg) => ({
    id: seg.id,
    speakerId: seg.speakerId,
    startMs: seg.startMs,
    endMs: seg.endMs,
    ...(seg.confidence !== undefined ? { confidence: seg.confidence } : {})
  }));

  const projectWithSpeakers = attachSpeakerState(
    projectWithTranscript,
    initialSpeakerProfiles,
    derivedSpeakerSegments,
    []
  );

  // Step 4: Apply speaker correction commands (SpeakerCorrectionReplay)
  let finalProject = projectWithSpeakers;
  const commands = input.speakerCorrectionCommands ?? [];

  if (commands.length > 0) {
    const correctionResult = applySpeakerCorrectionCommands(
      {
        speakers: projectWithSpeakers.speakers,
        speakerSegments: projectWithSpeakers.speakerSegments,
        corrections: projectWithSpeakers.corrections
      },
      commands
    );

    finalProject = attachSpeakerState(
      projectWithSpeakers,
      correctionResult.speakers,
      correctionResult.speakerSegments,
      correctionResult.corrections
    );
  }

  // Step 5: Project corrected speaker output
  const correctedOutput = projectMeetingCorrectedSpeakerOutput(finalProject);

  // Step 6: Create LLM input
  const llmInput = createCorrectedSpeakerLlmInput("ollama", correctedOutput);

  // Build segment timings from transcript segments for LLM request
  const segmentTimings = input.mockTranscriptSegments.map((seg) => ({
    sourceSegmentId: seg.id,
    startMs: seg.startMs,
    endMs: seg.endMs
  }));

  // Step 7: Create LLM request
  const llmRequest = createLlmRequestFromCorrectedSpeakerInput({
    requestId: `mock-request-${input.audioHash}`,
    input: llmInput,
    segmentTimings
  });

  // Step 8: Run deterministic mock LLM provider
  const llmEnvelope = runDeterministicMockLlmProvider({
    request: llmRequest,
    privacyConfirmed: true
  });

  if (llmEnvelope.kind !== "llm.deverbalization.result") {
    throw new Error(`Mock LLM pipeline failed: expected deverbalization result, got ${llmEnvelope.kind}`);
  }

  const deverbalizationOutput = llmEnvelope as DeverbalizationOutput;

  // Step 9: Generate markdown content
  const speakerLabels = finalProject.speakers
    .filter((s) => s.id !== "unknown")
    .map((s) => s.label);

  // Ensure at least one speaker label for frontmatter validation
  const frontmatterSpeakers = speakerLabels.length > 0 ? speakerLabels : ["Unknown speaker"];

  const frontmatter = createMarkdownFrontmatter({
    title: input.title,
    date: createdAt.split("T")[0] ?? "2024-01-01",
    audioHash: input.audioHash,
    speakers: frontmatterSpeakers,
    modelVersion: deverbalizationOutput.modelVersion,
    llmProvider: "ollama",
    schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
    generatedAt: createdAt
  });

  const originalTranscriptText = renderCorrectedSpeakerLlmInput(correctedOutput);

  const markdownOptions: GenerateMarkdownOptions = {
    frontmatter,
    deverbalizationOutput,
    originalTranscript: originalTranscriptText,
    speakerCorrections: []
  };

  const markdownContent = generateMarkdownContent(markdownOptions);

  return {
    project: finalProject,
    markdownContent,
    speakerProfiles: [...finalProject.speakers]
  };
}

function deriveUniqueProfiles(speakerSegments: readonly SpeakerSegment[]): SpeakerProfile[] {
  const seen = new Set<string>();
  const profiles: SpeakerProfile[] = [];

  for (const seg of speakerSegments) {
    if (!seen.has(seg.speakerId)) {
      seen.add(seg.speakerId);
      profiles.push({
        id: seg.speakerId,
        label: seg.speakerId
      });
    }
  }

  return profiles;
}
