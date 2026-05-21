import { describe, test, expect } from "bun:test";
import type { MeetingRecord } from "../src/ui-state.ts";
import {
  isTitleValid,
  updateMeetingTitle,
  deleteMeeting,
  sortMeetingsByDate,
} from "../src/meeting-management.ts";

function makeMeeting(
  id: string,
  title: string,
  createdAt = "2024-01-01T00:00:00.000Z"
): MeetingRecord {
  return {
    id,
    title,
    createdAt,
    markdownContent: "",
    speakerProfiles: [],
    projectSnapshot: "{}",
  };
}

// ---------------------------------------------------------------------------
// isTitleValid
// ---------------------------------------------------------------------------
describe("isTitleValid", () => {
  test("returns true for a normal Chinese title", () => {
    expect(isTitleValid("会议")).toBe(true);
  });

  test("returns true for a title that is valid after trimming", () => {
    expect(isTitleValid("  Q2产品  ")).toBe(true);
  });

  test("returns false for an empty string", () => {
    expect(isTitleValid("")).toBe(false);
  });

  test("returns false for a whitespace-only string", () => {
    expect(isTitleValid("   ")).toBe(false);
  });

  test("returns true for a 100-character string", () => {
    expect(isTitleValid("a".repeat(100))).toBe(true);
  });

  test("returns false for a 101-character string", () => {
    expect(isTitleValid("a".repeat(101))).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// updateMeetingTitle
// ---------------------------------------------------------------------------
describe("updateMeetingTitle", () => {
  test("correctly updates the matching meeting title", () => {
    const meetings = [makeMeeting("1", "Old Title"), makeMeeting("2", "Other")];
    const result = updateMeetingTitle(meetings, "1", "New Title");
    expect(result.find((m) => m.id === "1")?.title).toBe("New Title");
  });

  test("does not mutate the input array", () => {
    const meetings = [makeMeeting("1", "Old Title")];
    const original = meetings[0]!;
    updateMeetingTitle(meetings, "1", "New Title");
    expect(meetings[0]).toBe(original);
    expect(meetings[0]!.title).toBe("Old Title");
  });

  test("does not modify unmatched meetings", () => {
    const meetings = [makeMeeting("1", "Alpha"), makeMeeting("2", "Beta")];
    const result = updateMeetingTitle(meetings, "1", "Updated");
    expect(result.find((m) => m.id === "2")?.title).toBe("Beta");
  });

  test("returns a copy of the original array when id does not exist", () => {
    const meetings = [makeMeeting("1", "Alpha")];
    const result = updateMeetingTitle(meetings, "999", "Whatever");
    expect(result).not.toBe(meetings);
    expect(result).toEqual(meetings);
  });

  test("returns a copy of the original array when newTitle is invalid (empty string)", () => {
    const meetings = [makeMeeting("1", "Alpha")];
    const result = updateMeetingTitle(meetings, "1", "");
    expect(result).not.toBe(meetings);
    expect(result[0]!.title).toBe("Alpha");
  });
});

// ---------------------------------------------------------------------------
// deleteMeeting
// ---------------------------------------------------------------------------
describe("deleteMeeting", () => {
  test("removes the matching meeting", () => {
    const meetings = [makeMeeting("1", "Alpha"), makeMeeting("2", "Beta")];
    const result = deleteMeeting(meetings, "1");
    expect(result.find((m) => m.id === "1")).toBeUndefined();
  });

  test("returns an array with length reduced by 1", () => {
    const meetings = [makeMeeting("1", "Alpha"), makeMeeting("2", "Beta")];
    const result = deleteMeeting(meetings, "1");
    expect(result.length).toBe(1);
  });

  test("does not mutate the input array", () => {
    const meetings = [makeMeeting("1", "Alpha"), makeMeeting("2", "Beta")];
    deleteMeeting(meetings, "1");
    expect(meetings.length).toBe(2);
  });

  test("does not affect other meetings", () => {
    const meetings = [makeMeeting("1", "Alpha"), makeMeeting("2", "Beta")];
    const result = deleteMeeting(meetings, "1");
    expect(result[0]!.id).toBe("2");
    expect(result[0]!.title).toBe("Beta");
  });

  test("returns same-length array when id does not exist", () => {
    const meetings = [makeMeeting("1", "Alpha"), makeMeeting("2", "Beta")];
    const result = deleteMeeting(meetings, "999");
    expect(result.length).toBe(2);
  });
});

// ---------------------------------------------------------------------------
// sortMeetingsByDate
// ---------------------------------------------------------------------------
describe("sortMeetingsByDate", () => {
  test("sorts meetings by createdAt in descending order (newest first)", () => {
    const meetings = [
      makeMeeting("1", "Oldest", "2023-01-01T00:00:00.000Z"),
      makeMeeting("2", "Newest", "2025-06-01T00:00:00.000Z"),
      makeMeeting("3", "Middle", "2024-03-15T00:00:00.000Z"),
    ];
    const result = sortMeetingsByDate(meetings);
    expect(result[0]!.id).toBe("2");
    expect(result[1]!.id).toBe("3");
    expect(result[2]!.id).toBe("1");
  });

  test("does not mutate the input array", () => {
    const meetings = [
      makeMeeting("1", "Oldest", "2023-01-01T00:00:00.000Z"),
      makeMeeting("2", "Newest", "2025-06-01T00:00:00.000Z"),
    ];
    sortMeetingsByDate(meetings);
    expect(meetings[0]!.id).toBe("1");
    expect(meetings[1]!.id).toBe("2");
  });
});
