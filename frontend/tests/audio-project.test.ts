import { describe, expect, test } from "bun:test";
import { attachSpeakerState, createMeetingProject, getSupportedAudioExtension } from "../src/audio-project";

describe("audio project model", () => {
  test("accepts only the supported MVP audio formats", () => {
    expect(getSupportedAudioExtension("/tmp/meeting.wav")).toBe(".wav");
    expect(getSupportedAudioExtension("/tmp/meeting.MP3")).toBe(".mp3");
    expect(getSupportedAudioExtension("/tmp/meeting.m4a")).toBe(".m4a");
    expect(getSupportedAudioExtension("/tmp/meeting.flac")).toBeNull();
  });

  test("creates deterministic project state from an audio import", () => {
    const project = createMeetingProject({
      path: "/meetings/周会.m4a",
      hash: "abc123",
      durationSeconds: 1800,
      createdAt: "2026-05-17T09:00:00.000Z"
    });

    expect(project.id).toBe("audio-abc123");
    expect(project.title).toBe("周会");
    expect(project.format).toBe(".m4a");
    expect(project.editedMarkdown).toBeNull();
  });

  test("persists speaker corrections in project state", () => {
    const project = createMeetingProject({
      path: "/meetings/weekly.wav",
      hash: "weekly",
      durationSeconds: 120,
      createdAt: "2026-05-17T09:00:00.000Z"
    });

    const updated = attachSpeakerState(
      project,
      [{ id: "speaker-a", label: "主持人" }],
      [{ id: "seg-1", speakerId: "speaker-a", startMs: 0, endMs: 1000 }],
      [
        {
          id: "correction-1",
          targetSpeakerId: "speaker-b",
          replacementSpeakerId: "speaker-a",
          reason: "manual-merge",
          createdAt: "2026-05-17T09:01:00.000Z"
        }
      ]
    );

    expect(updated.corrections).toHaveLength(1);
    expect(updated.speakers[0]?.label).toBe("主持人");
  });
});
