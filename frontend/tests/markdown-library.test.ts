import { describe, expect, test } from "bun:test";
import {
  attachEditedMarkdown,
  attachSpeakerState,
  attachTranscript,
  createMeetingProject,
  restoreMeetingProjectManifest
} from "../src/types/audio-project";
import { createMarkdownLibraryRecord, rebuildLibraryIndex } from "../src/utils/markdown-library";

describe("markdown file library", () => {
  test("creates a pure file record without database dependencies", () => {
    const project = attachEditedMarkdown(
      attachSpeakerState(
        attachTranscript(
          createMeetingProject({
            title: "中文周会",
            path: "/audio/weekly.mp3",
            hash: "weekly-hash",
            durationSeconds: 360,
            createdAt: "2026-05-17T09:00:00.000Z"
          }),
          [{ id: "seg-1", startMs: 0, endMs: 1000, text: "大家好", language: "zh" }]
        ),
        [{ id: "speaker-1", label: "主持人" }],
        [{ id: "spk-1", speakerId: "speaker-1", startMs: 0, endMs: 1000 }],
        []
      ),
      "# 中文周会"
    );

    const record = createMarkdownLibraryRecord(project);
    const manifestFile = record.files.find((file) => file.path.endsWith("/manifest.json"));

    expect(record.root).toBe("meetings/2026/05/中文周会");
    expect(record.files.map((file) => file.path)).toEqual([
      "meetings/2026/05/中文周会/manifest.json",
      "meetings/2026/05/中文周会/meeting.md"
    ]);
    expect(restoreMeetingProjectManifest(JSON.parse(manifestFile?.contents ?? "") as unknown)).toEqual(project);
    expect(record.files[1]?.contents).toContain("schema: meetnote.markdown.v1");
  });

  test("projects corrected speaker text into markdown when no edited markdown exists", () => {
    const project = attachSpeakerState(
      attachTranscript(
        createMeetingProject({
          title: "Speaker history",
          path: "/audio/speaker-history.wav",
          hash: "speaker-history-hash",
          durationSeconds: 180,
          createdAt: "2026-05-20T09:00:00.000Z"
        }),
        [
          { id: "asr-1", startMs: 0, endMs: 1000, text: "今天先确认目标", language: "zh" },
          { id: "asr-2", startMs: 1200, endMs: 2200, text: "我来补充进度", language: "zh" }
        ]
      ),
      [
        { id: "speaker-a", label: "主持人" },
        { id: "speaker-b", label: "研发" }
      ],
      [
        { id: "spk-1", speakerId: "speaker-a", startMs: 0, endMs: 1100 },
        { id: "spk-2", speakerId: "speaker-b", startMs: 1200, endMs: 2200 }
      ],
      [
        {
          id: "cmd-1",
          commandId: "cmd-1",
          kind: "assign-range",
          targetSpeakerId: "speaker-b",
          replacementSpeakerId: "speaker-b",
          reason: "manual-range-assign",
          createdAt: "2026-05-20T09:01:00.000Z",
          beforeSegments: [{ id: "spk-2", speakerId: "speaker-a", startMs: 1200, endMs: 2200 }],
          afterSegments: [{ id: "spk-2", speakerId: "speaker-b", startMs: 1200, endMs: 2200 }]
        }
      ]
    );

    const record = createMarkdownLibraryRecord(project);

    expect(record.files[1]?.contents).toContain("**主持人:** 今天先确认目标");
    expect(record.files[1]?.contents).toContain("**研发:** 我来补充进度");
    expect(record.files[1]?.contents).not.toContain("_No edited meeting text yet._");
  });

  test("rebuilds an index from file records", () => {
    const first = createMarkdownLibraryRecord(
      createMeetingProject({
        title: "A",
        path: "/audio/a.wav",
        hash: "a",
        durationSeconds: 1,
        createdAt: "2026-05-17T09:00:00.000Z"
      })
    );
    const second = createMarkdownLibraryRecord(
      createMeetingProject({
        title: "B",
        path: "/audio/b.m4a",
        hash: "b",
        durationSeconds: 1,
        createdAt: "2026-05-17T09:00:00.000Z"
      })
    );

    expect(rebuildLibraryIndex([second, first]).projectIds).toEqual(["audio-a", "audio-b"]);
  });
});
