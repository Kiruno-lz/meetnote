export const APP_SECTIONS = [
  "project-list",
  "recording-detail",
  "transcription-progress",
  "speaker-editor",
  "deverbalization-preview",
  "markdown-library"
] as const;

export type AppSection = (typeof APP_SECTIONS)[number];

export type CapabilityId =
  | "audio-import"
  | "mock-asr"
  | "mock-diarization"
  | "speaker-correction"
  | "llm-deverbalization"
  | "markdown-library";

export interface AppCapability {
  readonly id: CapabilityId;
  readonly label: string;
  readonly mvp: boolean;
  readonly requiresModel: boolean;
}

export interface AppShellState {
  readonly sections: readonly AppSection[];
  readonly capabilities: readonly AppCapability[];
  readonly activeSection: AppSection;
}

export function createAppShellState(activeSection: AppSection = "project-list"): AppShellState {
  return {
    sections: APP_SECTIONS,
    activeSection,
    capabilities: [
      { id: "audio-import", label: "Audio import", mvp: true, requiresModel: false },
      { id: "mock-asr", label: "Mock ASR", mvp: true, requiresModel: false },
      { id: "mock-diarization", label: "Mock diarization", mvp: true, requiresModel: false },
      { id: "speaker-correction", label: "Speaker correction", mvp: true, requiresModel: false },
      { id: "llm-deverbalization", label: "LLM deverbalization", mvp: true, requiresModel: true },
      { id: "markdown-library", label: "Markdown file library", mvp: true, requiresModel: false }
    ]
  };
}
