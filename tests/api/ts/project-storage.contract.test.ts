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
