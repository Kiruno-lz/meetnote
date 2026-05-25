import { describe, expect, test } from "bun:test";
import {
  UNKNOWN_SPEAKER_PROFILE,
  assignSpeakerForRange,
  applySpeakerCorrectionCommands,
  bulkReplaceSpeaker,
  ensureUnknownSpeakerProfile,
  mergeSpeakers,
  mergeTranscriptWithDiarization,
  renameSpeaker,
  resolveDiarizationRerun,
  resolveProjectDiarizationRerun,
  splitSpeakerSegment
} from "../src/utils/speaker";
import {
  attachSpeakerState,
  attachTranscript,
  createMeetingProjectManifest,
  createMeetingProject,
  restoreMeetingProjectManifest,
  validateMeetingProjectManifest
} from "../src/types/audio-project";
import { createMockDiarizationResult } from "../src/types/diarization-schema";

describe("speaker corrections", () => {
  test("renames a speaker without mutating other speakers", () => {
    const speakers = [
      { id: "s1", label: "Speaker 1" },
      { id: "s2", label: "Speaker 2" }
    ];

    expect(renameSpeaker(speakers, "s1", "产品经理")).toEqual([
      { id: "s1", label: "产品经理" },
      { id: "s2", label: "Speaker 2" }
    ]);
  });

  test("merges one speaker into another and records the manual correction", () => {
    const result = mergeSpeakers(
      [
        { id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 },
        { id: "seg-2", speakerId: "s2", startMs: 1000, endMs: 2000 }
      ],
      [],
      "s2",
      "s1",
      "2026-05-17T09:00:00.000Z"
    );

    expect(result.segments[1]?.speakerId).toBe("s1");
    expect(result.corrections[0]?.reason).toBe("manual-merge");
  });

  test("assigns a speaker to overlapping timeline segments", () => {
    const updated = assignSpeakerForRange(
      [
        { id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 },
        { id: "seg-2", speakerId: "s2", startMs: 1000, endMs: 2000 }
      ],
      900,
      1500,
      "s3"
    );

    expect(updated.map((segment) => segment.speakerId)).toEqual(["s3", "s3"]);
  });

  test("splits a speaker segment at a deterministic timestamp", () => {
    const input = [{ id: "seg-1", speakerId: "s1", startMs: 0, endMs: 2000, confidence: 0.91 }];

    const updated = splitSpeakerSegment(input, "seg-1", 750);

    expect(updated).toEqual([
      { id: "seg-1-split-750-a", speakerId: "s1", startMs: 0, endMs: 750, confidence: 0.91 },
      { id: "seg-1-split-750-b", speakerId: "s1", startMs: 750, endMs: 2000, confidence: 0.91 }
    ]);
    expect(input).toEqual([{ id: "seg-1", speakerId: "s1", startMs: 0, endMs: 2000, confidence: 0.91 }]);
  });

  test("bulk replaces all segments from one speaker without mutating input", () => {
    const input = [
      { id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 },
      { id: "seg-2", speakerId: "s2", startMs: 1000, endMs: 2000 },
      { id: "seg-3", speakerId: "s1", startMs: 2000, endMs: 3000 }
    ];

    const updated = bulkReplaceSpeaker(input, "s1", "s3");

    expect(updated).toEqual([
      { id: "seg-1", speakerId: "s3", startMs: 0, endMs: 1000 },
      { id: "seg-2", speakerId: "s2", startMs: 1000, endMs: 2000 },
      { id: "seg-3", speakerId: "s3", startMs: 2000, endMs: 3000 }
    ]);
    expect(input.map((segment) => segment.speakerId)).toEqual(["s1", "s2", "s1"]);
  });

  test("replays manual speaker correction commands into speaker state and correction records", () => {
    const result = applySpeakerCorrectionCommands(
      {
        speakers: [
          { id: "s1", label: "Speaker 1" },
          { id: "s2", label: "Speaker 2" },
          { id: "s3", label: "Speaker 3" }
        ],
        speakerSegments: [
          { id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 },
          { id: "seg-2", speakerId: "s2", startMs: 1000, endMs: 3000 },
          { id: "seg-3", speakerId: "s3", startMs: 3000, endMs: 4000 }
        ],
        corrections: []
      },
      [
        { id: "cmd-1", kind: "rename-speaker", speakerId: "s1", label: "主持人", createdAt: "2026-05-20T01:00:00.000Z" },
        { id: "cmd-2", kind: "merge-speakers", sourceSpeakerId: "s2", targetSpeakerId: "s1", createdAt: "2026-05-20T01:01:00.000Z" },
        { id: "cmd-3", kind: "split-segment", segmentId: "seg-2", splitAtMs: 1800, createdAt: "2026-05-20T01:02:00.000Z" },
        { id: "cmd-4", kind: "assign-range", rangeStartMs: 1800, rangeEndMs: 2200, speakerId: "s3", createdAt: "2026-05-20T01:03:00.000Z" },
        { id: "cmd-5", kind: "bulk-replace-speaker", sourceSpeakerId: "s3", targetSpeakerId: "s1", createdAt: "2026-05-20T01:04:00.000Z" }
      ]
    );

    expect(result.speakers).toEqual([
      { id: "s1", label: "主持人" },
      { id: "s3", label: "Speaker 3" }
    ]);
    expect(result.speakerSegments).toEqual([
      { id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 },
      { id: "seg-2-split-1800-a", speakerId: "s1", startMs: 1000, endMs: 1800 },
      { id: "seg-2-split-1800-b", speakerId: "s1", startMs: 1800, endMs: 3000 },
      { id: "seg-3", speakerId: "s1", startMs: 3000, endMs: 4000 }
    ]);
    expect(result.corrections.map((correction) => correction.id)).toEqual(["cmd-1", "cmd-2", "cmd-3", "cmd-4", "cmd-5"]);
    expect(result.corrections[3]?.afterSegments).toEqual([
      { id: "seg-2-split-1800-b", speakerId: "s3", startMs: 1800, endMs: 3000 }
    ]);
  });

  test("persists merge correction snapshots after the source speaker is removed", () => {
    const project = attachTranscript(
      createMeetingProject({
        path: "/meetings/merge-speakers.wav",
        hash: "merge-speakers-hash",
        durationSeconds: 120,
        createdAt: "2026-05-20T09:00:00.000Z"
      }),
      [
        { id: "asr-1", startMs: 0, endMs: 1000, text: "先看目标", language: "zh" },
        { id: "asr-2", startMs: 1000, endMs: 2000, text: "我补充风险", language: "zh" }
      ]
    );

    const manualState = applySpeakerCorrectionCommands(
      {
        speakers: [
          { id: "speaker-host", label: "主持人" },
          { id: "speaker-dev", label: "研发" }
        ],
        speakerSegments: [
          { id: "spk-1", speakerId: "speaker-host", startMs: 0, endMs: 1000 },
          { id: "spk-2", speakerId: "speaker-dev", startMs: 1000, endMs: 2000 }
        ],
        corrections: []
      },
      [
        {
          id: "cmd-merge-dev-host",
          kind: "merge-speakers",
          sourceSpeakerId: "speaker-dev",
          targetSpeakerId: "speaker-host",
          createdAt: "2026-05-20T09:01:00.000Z"
        }
      ]
    );

    const withSpeakerState = attachSpeakerState(
      project,
      manualState.speakers,
      manualState.speakerSegments,
      manualState.corrections
    );
    const manifest = createMeetingProjectManifest(withSpeakerState);
    const restored = restoreMeetingProjectManifest(JSON.parse(JSON.stringify(manifest)) as unknown);

    expect(manualState.speakers).toEqual([{ id: "speaker-host", label: "主持人" }]);
    expect(manualState.corrections[0]?.beforeSegments).toEqual([
      { id: "spk-2", speakerId: "speaker-dev", startMs: 1000, endMs: 2000 }
    ]);
    expect(validateMeetingProjectManifest(withSpeakerState)).toEqual({ ok: true, errors: [] });
    expect(manifest).toEqual(withSpeakerState);
    expect(restored).toEqual(withSpeakerState);
  });

  test("keeps unknown speaker profile project-safe while replaying manual corrections", () => {
    const result = applySpeakerCorrectionCommands(
      {
        speakers: [{ id: "s1", label: "Speaker 1" }],
        speakerSegments: [{ id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 }],
        corrections: []
      },
      [{ id: "cmd-unknown", kind: "assign-range", rangeStartMs: 0, rangeEndMs: 1000, speakerId: "unknown", createdAt: "2026-05-20T01:00:00.000Z" }]
    );

    expect(result.speakers).toEqual([{ id: "s1", label: "Speaker 1" }, UNKNOWN_SPEAKER_PROFILE]);
    expect(result.speakerSegments).toEqual([{ id: "seg-1", speakerId: "unknown", startMs: 0, endMs: 1000 }]);
  });

  test("returns a rerun diarization conflict before replacing manually corrected segments", () => {
    const manualState = applySpeakerCorrectionCommands(
      {
        speakers: [
          { id: "s1", label: "Speaker 1" },
          { id: "s2", label: "Speaker 2" }
        ],
        speakerSegments: [
          { id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 },
          { id: "seg-2", speakerId: "s2", startMs: 1000, endMs: 2000 }
        ],
        corrections: []
      },
      [{ id: "cmd-range", kind: "assign-range", rangeStartMs: 0, rangeEndMs: 1000, speakerId: "s2", createdAt: "2026-05-20T01:00:00.000Z" }]
    );

    const resolution = resolveDiarizationRerun(manualState.speakerSegments, [
      { id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 },
      { id: "seg-2", speakerId: "s2", startMs: 1000, endMs: 2000 }
    ], manualState.corrections);

    expect(resolution).toEqual({
      status: "conflict",
      speakerSegments: manualState.speakerSegments,
      generatedSegments: [
        { id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 },
        { id: "seg-2", speakerId: "s2", startMs: 1000, endMs: 2000 }
      ],
      conflicts: [
        {
          segmentId: "seg-1",
          startMs: 0,
          endMs: 1000,
          currentSpeakerId: "s2",
          generatedSpeakerId: "s1",
          reason: "manual-edit-would-change"
        }
      ]
    });
  });

  test("keeps project state unchanged when rerun diarization conflicts with manual corrections", () => {
    const project = attachSpeakerState(
      createMeetingProject({
        path: "/meetings/rerun.wav",
        hash: "rerun-hash",
        durationSeconds: 120,
        createdAt: "2026-05-20T09:00:00.000Z"
      }),
      [
        { id: "s1", label: "Speaker 1" },
        { id: "s2", label: "Speaker 2" }
      ],
      [{ id: "seg-1", speakerId: "s2", startMs: 0, endMs: 1000 }],
      [
        {
          id: "cmd-range",
          commandId: "cmd-range",
          kind: "assign-range",
          targetSpeakerId: "s2",
          replacementSpeakerId: "s2",
          reason: "manual-range-assign",
          createdAt: "2026-05-20T09:01:00.000Z",
          beforeSegments: [{ id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 }],
          afterSegments: [{ id: "seg-1", speakerId: "s2", startMs: 0, endMs: 1000 }]
        }
      ]
    );

    const resolution = resolveProjectDiarizationRerun(project, [{ id: "seg-1", speakerId: "s1", startMs: 0, endMs: 1000 }]);

    expect(resolution.status).toBe("conflict");
    expect(resolution.project).toEqual(project);
    if (resolution.status === "conflict") {
      expect(resolution.conflicts).toEqual([
        {
          segmentId: "seg-1",
          startMs: 0,
          endMs: 1000,
          currentSpeakerId: "s2",
          generatedSpeakerId: "s1",
          reason: "manual-edit-would-change"
        }
      ]);
    }
  });

  test("maps exactly aligned ASR segments to matching diarization speakers", () => {
    const merged = mergeTranscriptWithDiarization(
      [
        { id: "asr-1", startMs: 0, endMs: 1000, text: "先说第一部分", language: "zh" },
        { id: "asr-2", startMs: 1000, endMs: 2000, text: "再说第二部分", language: "zh" }
      ],
      [
        { id: "dia-1", speakerId: "speaker-a", startMs: 0, endMs: 1000, confidence: 0.98 },
        { id: "dia-2", speakerId: "speaker-b", startMs: 1000, endMs: 2000, confidence: 0.97 }
      ]
    );

    expect(merged).toEqual([
      {
        id: "asr-1",
        text: "先说第一部分",
        language: "zh",
        speakerId: "speaker-a",
        startMs: 0,
        endMs: 1000,
        confidence: 0.98
      },
      {
        id: "asr-2",
        text: "再说第二部分",
        language: "zh",
        speakerId: "speaker-b",
        startMs: 1000,
        endMs: 2000,
        confidence: 0.97
      }
    ]);
  });

  test("attaches mock diarization speakers and merged ASR transcript to project state", () => {
    const project = attachTranscript(
      createMeetingProject({
        path: "/meetings/mock.wav",
        hash: "mock-diarization-hash",
        durationSeconds: 6,
        createdAt: "2026-05-18T09:00:00.000Z"
      }),
      [
        { id: "asr-1", startMs: 0, endMs: 2400, text: "今天先确认目标", language: "zh" },
        { id: "asr-2", startMs: 2600, endMs: 5000, text: "然后讨论实现方案", language: "zh" }
      ]
    );
    const diarization = createMockDiarizationResult({
      requestId: "diarization-request-acceptance",
      model: {
        id: "pyannote-speaker-diarization-3.1",
        version: "3.1",
        parameters: {
          minSpeakers: 1,
          maxSpeakers: 2,
          collarMs: 250,
          overlapDetection: true
        }
      },
      auth: { provider: "huggingface", state: "configured" }
    });

    const merged = mergeTranscriptWithDiarization(project.transcript, diarization.segments);
    const withSpeakerState = attachSpeakerState(project, diarization.speakers, merged, []);

    expect(merged.map((segment) => segment.speakerId)).toEqual(["speaker-1", "speaker-2"]);
    expect(withSpeakerState.speakers).toEqual(diarization.speakers);
    expect(withSpeakerState.speakerSegments).toEqual(merged);
    expect(validateMeetingProjectManifest(withSpeakerState)).toEqual({ ok: true, errors: [] });
    expect(restoreMeetingProjectManifest(JSON.parse(JSON.stringify(withSpeakerState)) as unknown)).toEqual(withSpeakerState);
  });

  test("prefers the best overlapping diarization segment when ASR boundaries drift", () => {
    const merged = mergeTranscriptWithDiarization(
      [{ id: "asr-1", startMs: 950, endMs: 2050, text: "边界漂移的句子", language: "zh" }],
      [
        { id: "dia-1", speakerId: "speaker-a", startMs: 0, endMs: 1000, confidence: 0.82 },
        { id: "dia-2", speakerId: "speaker-b", startMs: 1000, endMs: 2100, confidence: 0.88 }
      ]
    );

    expect(merged).toEqual([
      {
        id: "asr-1",
        text: "边界漂移的句子",
        language: "zh",
        speakerId: "speaker-b",
        startMs: 950,
        endMs: 2050,
        confidence: 0.88
      }
    ]);
  });

  test("assigns the unknown speaker when no diarization segment covers the ASR segment", () => {
    const merged = mergeTranscriptWithDiarization(
      [{ id: "asr-1", startMs: 3000, endMs: 3600, text: "静音后的一句", language: "zh" }],
      [
        { id: "dia-1", speakerId: "speaker-a", startMs: 0, endMs: 1000, confidence: 0.7 },
        { id: "dia-2", speakerId: "speaker-b", startMs: 1200, endMs: 1800, confidence: 0.8 }
      ]
    );

    expect(merged).toEqual([
      {
        id: "asr-1",
        text: "静音后的一句",
        language: "zh",
        speakerId: "unknown",
        startMs: 3000,
        endMs: 3600
      }
    ]);
  });

  test("keeps unknown speaker output valid when attached to a project manifest", () => {
    const project = attachTranscript(
      createMeetingProject({
        path: "/meetings/silence.wav",
        hash: "silence-hash",
        durationSeconds: 10,
        createdAt: "2026-05-17T09:00:00.000Z"
      }),
      [{ id: "asr-1", startMs: 3000, endMs: 3600, text: "静音后的一句", language: "zh" }]
    );
    const merged = mergeTranscriptWithDiarization(project.transcript, []);
    const withSpeakers = attachSpeakerState(project, ensureUnknownSpeakerProfile([], merged), merged, []);

    expect(withSpeakers.speakers).toEqual([UNKNOWN_SPEAKER_PROFILE]);
    expect(validateMeetingProjectManifest(withSpeakers)).toEqual({ ok: true, errors: [] });
    expect(restoreMeetingProjectManifest(JSON.parse(JSON.stringify(withSpeakers)) as unknown)).toEqual(withSpeakers);
  });

  test("does not assign a speaker from accidental one millisecond boundary drift", () => {
    const merged = mergeTranscriptWithDiarization(
      [{ id: "asr-1", startMs: 999, endMs: 1800, text: "边界漂移不应误标", language: "zh" }],
      [{ id: "dia-1", speakerId: "speaker-a", startMs: 0, endMs: 1000, confidence: 0.99 }]
    );

    expect(merged).toEqual([
      {
        id: "asr-1",
        text: "边界漂移不应误标",
        language: "zh",
        speakerId: "unknown",
        startMs: 999,
        endMs: 1800
      }
    ]);
  });

  test("allows explicit overlap thresholds for short ASR segments", () => {
    const merged = mergeTranscriptWithDiarization(
      [{ id: "asr-1", startMs: 980, endMs: 1050, text: "短句", language: "zh" }],
      [{ id: "dia-1", speakerId: "speaker-a", startMs: 900, endMs: 1000, confidence: 0.8 }],
      { minOverlapMs: 15, minOverlapRatio: 0.2 }
    );

    expect(merged[0]?.speakerId).toBe("speaker-a");
    expect(merged[0]?.confidence).toBe(0.8);
  });

  test("resolves overlapping diarization segments deterministically", () => {
    const merged = mergeTranscriptWithDiarization(
      [{ id: "asr-1", startMs: 1000, endMs: 1500, text: "两个说话人重叠", language: "zh" }],
      [
        { id: "dia-1", speakerId: "speaker-a", startMs: 900, endMs: 1600, confidence: 0.4 },
        { id: "dia-2", speakerId: "speaker-b", startMs: 1000, endMs: 1700, confidence: 0.9 }
      ]
    );

    expect(merged).toEqual([
      {
        id: "asr-1",
        text: "两个说话人重叠",
        language: "zh",
        speakerId: "speaker-b",
        startMs: 1000,
        endMs: 1500,
        confidence: 0.9
      }
    ]);
  });
});
