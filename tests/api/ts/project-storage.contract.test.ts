import { describe, expect, test } from "bun:test";
import {
  addEditHistoryEntry,
  attachDeverbalizedState,
  attachEditedMarkdown,
  attachSpeakerState,
  attachTranscript,
  createMeetingProject,
  createMeetingProjectManifest,
  planAudioImport,
  restoreMeetingProjectManifest
} from "../../../frontend/src/audio-project";
import { applySpeakerCorrectionCommands } from "../../../frontend/src/speaker";

describe("project storage contract", () => {
  test("round-trips a project manifest through JSON serialization", () => {
    const project = addEditHistoryEntry(
      attachEditedMarkdown(
        attachSpeakerState(
          attachDeverbalizedState(
            attachTranscript(
              createMeetingProject({
                path: "/meetings/project-review.wav",
                hash: "project-review-hash",
                durationSeconds: 600,
                createdAt: "2026-05-17T09:00:00.000Z",
                title: "Project review"
              }),
              [
                {
                  id: "seg-1",
                  startMs: 0,
                  endMs: 1500,
                  text: "呃我们今天先确认里程碑",
                  language: "zh"
                }
              ]
            ),
            [
              {
                id: "clean-1",
                sourceSegmentIds: ["seg-1"],
                speakerId: "speaker-1",
                text: "我们今天先确认里程碑。",
                createdAt: "2026-05-17T09:02:00.000Z",
                provider: "mock-local"
              }
            ]
          ),
          [{ id: "speaker-1", label: "主持人" }],
          [{ id: "spk-1", speakerId: "speaker-1", startMs: 0, endMs: 1500 }],
          [
            {
              id: "correction-1",
              targetSpeakerId: "speaker-raw",
              replacementSpeakerId: "speaker-1",
              reason: "manual-rename",
              createdAt: "2026-05-17T09:02:30.000Z"
            }
          ]
        ),
        "# Project review"
      ),
      {
        id: "edit-1",
        kind: "markdown-edit",
        summary: "Tidy headline",
        createdAt: "2026-05-17T09:03:00.000Z",
        actor: "user"
      }
    );

    const restored = restoreMeetingProjectManifest(JSON.parse(JSON.stringify(project)) as unknown);

    expect(restored).toEqual(project);
    expect(restored.speakers).toEqual([{ id: "speaker-1", label: "主持人" }]);
    expect(restored.speakerSegments).toEqual([{ id: "spk-1", speakerId: "speaker-1", startMs: 0, endMs: 1500 }]);
    expect(restored.corrections).toEqual([
      {
        id: "correction-1",
        targetSpeakerId: "speaker-raw",
        replacementSpeakerId: "speaker-1",
        reason: "manual-rename",
        createdAt: "2026-05-17T09:02:30.000Z"
      }
    ]);
    expect(createMeetingProjectManifest(restored)).toEqual(project);
  });

  test("round-trips structured speaker correction records through the storage contract", () => {
    const project = attachSpeakerState(
      attachTranscript(
        createMeetingProject({
          path: "/meetings/manual-corrections.m4a",
          hash: "manual-corrections-hash",
          durationSeconds: 300,
          createdAt: "2026-05-20T09:00:00.000Z",
          title: "Manual corrections"
        }),
        [
          {
            id: "asr-1",
            startMs: 0,
            endMs: 1500,
            text: "先确认议程",
            language: "zh"
          }
        ]
      ),
      [{ id: "speaker-host", label: "主持人" }],
      [{ id: "spk-1", speakerId: "speaker-host", startMs: 0, endMs: 1500 }],
      [
        {
          id: "cmd-range-1",
          commandId: "cmd-range-1",
          kind: "assign-range",
          targetSpeakerId: "speaker-host",
          replacementSpeakerId: "speaker-host",
          reason: "manual-range-assign",
          createdAt: "2026-05-20T09:01:00.000Z",
          beforeSegments: [{ id: "spk-1", speakerId: "speaker-host", startMs: 0, endMs: 1500 }],
          afterSegments: [{ id: "spk-1", speakerId: "speaker-host", startMs: 0, endMs: 1500 }]
        }
      ]
    );

    const manifest = createMeetingProjectManifest(project);
    const restored = restoreMeetingProjectManifest(JSON.parse(JSON.stringify(manifest)) as unknown);

    expect(restored).toEqual(project);
    expect(restored.corrections).toEqual(project.corrections);
    expect(restored.corrections[0]?.afterSegments).not.toBe(project.corrections[0]?.afterSegments);
  });

  test("round-trips merge correction snapshots for removed source speakers", () => {
    const baseProject = attachTranscript(
      createMeetingProject({
        path: "/meetings/merge-storage.wav",
        hash: "merge-storage-hash",
        durationSeconds: 300,
        createdAt: "2026-05-20T09:00:00.000Z",
        title: "Merge storage"
      }),
      [
        {
          id: "asr-1",
          startMs: 0,
          endMs: 1500,
          text: "先确认目标",
          language: "zh"
        },
        {
          id: "asr-2",
          startMs: 1500,
          endMs: 3000,
          text: "我补充风险",
          language: "zh"
        }
      ]
    );
    const manualState = applySpeakerCorrectionCommands(
      {
        speakers: [
          { id: "speaker-host", label: "主持人" },
          { id: "speaker-dev", label: "研发" }
        ],
        speakerSegments: [
          { id: "spk-1", speakerId: "speaker-host", startMs: 0, endMs: 1500 },
          { id: "spk-2", speakerId: "speaker-dev", startMs: 1500, endMs: 3000 }
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
    const project = attachSpeakerState(
      baseProject,
      manualState.speakers,
      manualState.speakerSegments,
      manualState.corrections
    );

    const manifest = createMeetingProjectManifest(project);
    const restored = restoreMeetingProjectManifest(JSON.parse(JSON.stringify(manifest)) as unknown);

    expect(restored).toEqual(project);
    expect(restored.speakers).toEqual([{ id: "speaker-host", label: "主持人" }]);
    expect(restored.corrections[0]?.beforeSegments).toEqual([
      { id: "spk-2", speakerId: "speaker-dev", startMs: 1500, endMs: 3000 }
    ]);
  });

  test("keeps raw transcript stable when current transcript is replaced", () => {
    const project = attachTranscript(
      createMeetingProject({
        path: "/meetings/raw-stability.wav",
        hash: "raw-stability-hash",
        durationSeconds: 600,
        createdAt: "2026-05-17T09:00:00.000Z",
        title: "Raw stability"
      }),
      [
        {
          id: "seg-1",
          startMs: 0,
          endMs: 1500,
          text: "呃原始转录",
          language: "zh"
        }
      ]
    );

    const updated = attachTranscript(project, [
      {
        id: "seg-1-edited",
        startMs: 0,
        endMs: 1500,
        text: "编辑后的当前转录",
        language: "zh"
      }
    ]);

    const restored = restoreMeetingProjectManifest(JSON.parse(JSON.stringify(updated)) as unknown);

    expect(restored.originalTranscript).toEqual(project.originalTranscript);
    expect(restored.transcript).toEqual(updated.transcript);
  });

  test("selects a stable canonical project for duplicate audio hashes", () => {
    const existing = [
      createMeetingProject({
        path: "/archive/older-copy.wav",
        hash: "dup-hash",
        durationSeconds: 120,
        createdAt: "2026-05-16T09:00:00.000Z"
      }),
      createMeetingProject({
        path: "/archive/newer-copy.mp3",
        hash: "dup-hash",
        durationSeconds: 120,
        createdAt: "2026-05-17T09:00:00.000Z"
      })
    ];

    expect(
      planAudioImport(existing, {
        path: "/downloads/dup.m4a",
        hash: "dup-hash",
        durationSeconds: 120,
        createdAt: "2026-05-18T09:00:00.000Z"
      })
    ).toEqual({
      action: "reuse-existing",
      projectId: "audio-dup-hash",
      reason: "duplicate-audio-hash"
    });
    expect(
      planAudioImport([...existing].reverse(), {
        path: "/downloads/dup.m4a",
        hash: "dup-hash",
        durationSeconds: 120,
        createdAt: "2026-05-18T09:00:00.000Z"
      })
    ).toEqual({
      action: "reuse-existing",
      projectId: "audio-dup-hash",
      reason: "duplicate-audio-hash"
    });
  });
});
