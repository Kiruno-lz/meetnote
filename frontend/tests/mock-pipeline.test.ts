import { describe, expect, test } from "bun:test";
import { runMockPipeline } from "../src/mock-pipeline";
import type { MockPipelineInput } from "../src/mock-pipeline";
import type { SpeakerCorrectionCommand } from "../src/speaker";

// Inline fixture data derived from tests/fixtures/simple-meeting.json
// Single speaker "Alice" talking for the full meeting duration

const MOCK_AUDIO_HASH = "a1b2c3d4e5f6a7b8";
const MOCK_AUDIO_PATH = "meeting-2024-05-15-simple.wav";
const MOCK_DURATION_SECONDS = 480;
const MOCK_TITLE = "Q2产品规划讨论";

const MOCK_TRANSCRIPT_SEGMENTS = [
  { id: "asr-seg-0001", startMs: 0, endMs: 2500, text: "今天我们先看一下Q2产品规划。", language: "zh" as const },
  { id: "asr-seg-0002", startMs: 2600, endMs: 5200, text: "我们需要完成三个核心功能。", language: "zh" as const },
  { id: "asr-seg-0003", startMs: 5300, endMs: 8100, text: "这些功能已经在backlog里面了。", language: "zh" as const },
  { id: "asr-seg-0004", startMs: 8200, endMs: 11500, text: "第一个，优化用户登录流程。", language: "zh" as const }
];

const MOCK_SPEAKER_PROFILES = [
  { id: "speaker-alice", label: "Alice" }
];

const MOCK_SPEAKER_SEGMENTS = [
  { id: "dia-seg-0001", speakerId: "speaker-alice", startMs: 0, endMs: 46500, confidence: 0.98 }
];

function buildBaseInput(): MockPipelineInput {
  return {
    audioPath: MOCK_AUDIO_PATH,
    audioHash: MOCK_AUDIO_HASH,
    durationSeconds: MOCK_DURATION_SECONDS,
    title: MOCK_TITLE,
    mockTranscriptSegments: MOCK_TRANSCRIPT_SEGMENTS,
    mockSpeakerSegments: MOCK_SPEAKER_SEGMENTS,
    mockSpeakerProfiles: MOCK_SPEAKER_PROFILES
  };
}

describe("runMockPipeline", () => {
  test("runs end-to-end with fixture data and returns non-empty markdownContent", () => {
    const result = runMockPipeline(buildBaseInput());

    expect(result.markdownContent).toBeTruthy();
    expect(result.markdownContent.length).toBeGreaterThan(0);
    expect(result.project).toBeDefined();
    expect(result.speakerProfiles).toBeDefined();
  });

  test("markdownContent includes speaker name from profiles", () => {
    const result = runMockPipeline(buildBaseInput());

    // Alice should appear in the markdown content as a speaker label
    expect(result.markdownContent).toContain("Alice");
  });

  test("returns correct project title", () => {
    const result = runMockPipeline(buildBaseInput());

    expect(result.project.title).toBe(MOCK_TITLE);
  });

  test("returns speaker profiles matching input", () => {
    const result = runMockPipeline(buildBaseInput());

    const aliceProfile = result.speakerProfiles.find((p) => p.id === "speaker-alice");
    expect(aliceProfile).toBeDefined();
    expect(aliceProfile?.label).toBe("Alice");
  });

  test("works with empty speakerCorrectionCommands", () => {
    const input: MockPipelineInput = {
      ...buildBaseInput(),
      speakerCorrectionCommands: []
    };

    const result = runMockPipeline(input);

    expect(result.markdownContent).toBeTruthy();
    expect(result.markdownContent).toContain("Alice");
  });

  test("works without speakerCorrectionCommands (omitted)", () => {
    const { speakerCorrectionCommands: _omitted, ...baseWithoutCommands } = buildBaseInput();
    const input: MockPipelineInput = baseWithoutCommands;

    const result = runMockPipeline(input);

    expect(result.markdownContent).toBeTruthy();
    expect(result.markdownContent).toContain("Alice");
  });

  test("speaker correction rename is reflected in markdownContent", () => {
    const renameCommand: SpeakerCorrectionCommand = {
      id: "cmd-rename-001",
      kind: "rename-speaker",
      speakerId: "speaker-alice",
      label: "产品经理",
      createdAt: "2024-01-01T00:01:00.000Z"
    };

    const inputWithCorrection: MockPipelineInput = {
      ...buildBaseInput(),
      speakerCorrectionCommands: [renameCommand]
    };

    const result = runMockPipeline(inputWithCorrection);

    // After rename, markdown should contain the new label
    expect(result.markdownContent).toContain("产品经理");
    // And should no longer show "Alice" (replaced by rename)
    expect(result.markdownContent).not.toContain("**Alice**");
  });

  test("project has all transcript segments attached", () => {
    const result = runMockPipeline(buildBaseInput());

    expect(result.project.transcript.length).toBe(MOCK_TRANSCRIPT_SEGMENTS.length);
    expect(result.project.originalTranscript.length).toBe(MOCK_TRANSCRIPT_SEGMENTS.length);
  });

  test("project has speaker segments attached", () => {
    const result = runMockPipeline(buildBaseInput());

    expect(result.project.speakerSegments.length).toBeGreaterThan(0);
  });

  test("markdownContent includes YAML frontmatter with title", () => {
    const result = runMockPipeline(buildBaseInput());

    expect(result.markdownContent).toContain("---");
    expect(result.markdownContent).toContain(MOCK_TITLE);
  });

  test("works without explicit mockSpeakerProfiles (profiles inferred from segments)", () => {
    const inputWithoutProfiles: MockPipelineInput = {
      audioPath: MOCK_AUDIO_PATH,
      audioHash: MOCK_AUDIO_HASH,
      durationSeconds: MOCK_DURATION_SECONDS,
      title: MOCK_TITLE,
      mockTranscriptSegments: MOCK_TRANSCRIPT_SEGMENTS,
      mockSpeakerSegments: MOCK_SPEAKER_SEGMENTS
      // no mockSpeakerProfiles
    };

    const result = runMockPipeline(inputWithoutProfiles);

    expect(result.markdownContent).toBeTruthy();
    expect(result.speakerProfiles.length).toBeGreaterThan(0);
  });
});
