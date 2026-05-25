import { describe, expect, test } from "bun:test";
import { APP_SECTIONS, createAppShellState } from "../src/components/app-shell";

describe("app shell skeleton", () => {
  test("defines the MVP workflow sections", () => {
    expect(APP_SECTIONS).toEqual([
      "project-list",
      "recording-detail",
      "transcription-progress",
      "speaker-editor",
      "deverbalization-preview",
      "markdown-library",
      "model-management",
      "settings"
    ]);
  });

  test("keeps model-dependent work explicit", () => {
    const state = createAppShellState();
    const llm = state.capabilities.find((capability) => capability.id === "llm-deverbalization");

    expect(llm?.mvp).toBe(true);
    expect(llm?.requiresModel).toBe(true);
  });
});
