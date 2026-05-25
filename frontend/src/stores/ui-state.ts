import type { AppSection } from "../components/app-shell.ts";
import type { SpeakerProfile } from "../types/audio-project.ts";

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
  deverPreviewBackTo?: AppSection;
  modelCachePath: string;
  /** Real audio file path from Tauri native dialog (session-only, not persisted) */
  pendingAudioPath?: string;
  /** Whether to use real ASR instead of mock pipeline (session-only, not persisted) */
  useRealAsr?: boolean;
  /** The active model ID selected in model management (persisted) */
  activeModelId?: string;
  /** Transcription target language (default "zh") */
  transcriptionLanguage: string;
  /** Transcription compute device ("auto" | "cpu" | "mps", default "auto") */
  transcriptionDevice: string;
  /** Directory where generated Markdown files are saved (default "~/Documents/meetnote") */
  markdownLibraryPath: string;
}

export function createInitialState(): AppState {
  return {
    meetings: [],
    activeMeetingId: null,
    activeSection: "project-list",
    pipelineRunning: false,
    pipelineStage: "",
    modelCachePath: "~/.meetnote/models",
    transcriptionLanguage: "zh",
    transcriptionDevice: "auto",
    markdownLibraryPath: "~/Documents/meetnote",
  };
}

export function loadState(): AppState {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return createInitialState();
    const parsed = JSON.parse(raw) as Partial<AppState>;
    const base: AppState = {
      meetings: Array.isArray(parsed.meetings) ? parsed.meetings : [],
      activeMeetingId: parsed.activeMeetingId ?? null,
      activeSection: (parsed.activeSection as AppSection) ?? "project-list",
      pipelineRunning: false,
      pipelineStage: "",
      modelCachePath:
        typeof parsed.modelCachePath === "string" && parsed.modelCachePath.trim()
          ? parsed.modelCachePath
          : "~/.meetnote/models",
      transcriptionLanguage:
        typeof parsed.transcriptionLanguage === "string" && parsed.transcriptionLanguage.trim()
          ? parsed.transcriptionLanguage
          : "zh",
      transcriptionDevice:
        typeof parsed.transcriptionDevice === "string" && parsed.transcriptionDevice.trim()
          ? parsed.transcriptionDevice
          : "auto",
      markdownLibraryPath:
        typeof parsed.markdownLibraryPath === "string" && parsed.markdownLibraryPath.trim()
          ? parsed.markdownLibraryPath
          : "~/Documents/meetnote",
    };
    if (parsed.deverPreviewBackTo) {
      base.deverPreviewBackTo = parsed.deverPreviewBackTo as AppSection;
    }
    if (typeof parsed.activeModelId === "string" && parsed.activeModelId.trim()) {
      base.activeModelId = parsed.activeModelId;
    }
    return base;
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
