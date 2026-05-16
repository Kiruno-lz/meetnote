import { describe, expect, test } from "bun:test";
import { assignSpeakerForRange, mergeSpeakers, renameSpeaker } from "../src/speaker";

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
});
