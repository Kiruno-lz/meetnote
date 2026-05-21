import type { MeetingRecord } from "./ui-state.ts";

/**
 * Validates a meeting title.
 * Returns true if the trimmed title is between 1 and 100 characters inclusive.
 */
export function isTitleValid(title: string): boolean {
  const trimmed = title.trim();
  return trimmed.length >= 1 && trimmed.length <= 100;
}

/**
 * Returns a new array with the matching meeting's title updated.
 * If the id does not exist or newTitle is invalid, the original array is returned as-is.
 */
export function updateMeetingTitle(
  meetings: readonly MeetingRecord[],
  id: string,
  newTitle: string
): MeetingRecord[] {
  if (!isTitleValid(newTitle)) {
    return [...meetings];
  }
  const index = meetings.findIndex((m) => m.id === id);
  if (index === -1) {
    return [...meetings];
  }
  const result = [...meetings];
  const existing = result[index] as MeetingRecord;
  result[index] = { ...existing, title: newTitle };
  return result;
}

/**
 * Returns a new array with the matching meeting removed.
 * If the id does not exist, the original array is returned as-is (shallow copy).
 */
export function deleteMeeting(
  meetings: readonly MeetingRecord[],
  id: string
): MeetingRecord[] {
  return meetings.filter((m) => m.id !== id);
}

/**
 * Returns a new array sorted by createdAt in descending order (newest first).
 * The original array is not modified.
 */
export function sortMeetingsByDate(
  meetings: readonly MeetingRecord[]
): MeetingRecord[] {
  return [...meetings].sort((a, b) => {
    if (b.createdAt < a.createdAt) return -1;
    if (b.createdAt > a.createdAt) return 1;
    return 0;
  });
}
