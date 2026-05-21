import type { AppSection } from "./app-shell.ts";
import type { SpeakerProfile } from "./audio-project.ts";

export const STORAGE_KEY = "meetnote-mvp-state";

export interface MeetingRecord {
  id: string;
  title: string;
  createdAt: string;
  markdownContent: string;
  speakerProfiles: SpeakerProfile[];
  // MeetingProject serialized as JSON string
  projectSnapshot: string;
}

export interface AppState {
  meetings: MeetingRecord[];
  activeMeetingId: string | null;
  activeSection: AppSection;
  pipelineRunning: boolean;
  pipelineStage: string;
}

export function createInitialState(): AppState {
  return {
    meetings: [],
    activeMeetingId: null,
    activeSection: "project-list",
    pipelineRunning: false,
    pipelineStage: ""
  };
}

export function loadState(): AppState {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return createInitialState();
    const parsed = JSON.parse(raw) as Partial<AppState>;
    return {
      meetings: Array.isArray(parsed.meetings) ? parsed.meetings : [],
      activeMeetingId: parsed.activeMeetingId ?? null,
      activeSection: (parsed.activeSection as AppSection) ?? "project-list",
      pipelineRunning: false,
      pipelineStage: ""
    };
  } catch {
    return createInitialState();
  }
}

export function saveState(state: AppState): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // localStorage not available, ignore
  }
}
