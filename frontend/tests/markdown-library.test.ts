import { describe, expect, test } from "bun:test";
import { createMeetingProject } from "../src/audio-project";
import { createMarkdownLibraryRecord, rebuildLibraryIndex } from "../src/markdown-library";

describe("markdown file library", () => {
  test("creates a pure file record without database dependencies", () => {
    const project = createMeetingProject({
      title: "中文周会",
      path: "/audio/weekly.mp3",
      hash: "weekly-hash",
      durationSeconds: 360,
      createdAt: "2026-05-17T09:00:00.000Z"
    });

    const record = createMarkdownLibraryRecord(project);

    expect(record.root).toBe("meetings/2026/05/中文周会");
    expect(record.files.map((file) => file.path)).toEqual([
      "meetings/2026/05/中文周会/manifest.json",
      "meetings/2026/05/中文周会/meeting.md"
    ]);
    expect(record.files[1]?.contents).toContain("schema: meetnote.markdown.v1");
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
