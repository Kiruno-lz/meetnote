import type { AppSection } from "./app-shell.ts";
import { loadState, saveState, type AppState } from "./ui-state.ts";
import { renderProjectList } from "./section-project-list.ts";
import { renderTranscriptionProgress } from "./section-transcription-progress.ts";
import { renderRecordingDetail } from "./section-recording-detail.ts";
import { renderSpeakerEditor } from "./section-speaker-editor.ts";
import { renderDeverbalizationPreview } from "./section-deverbalization-preview.ts";
import { renderMarkdownLibrary } from "./section-markdown-library.ts";

// Global state
let appState: AppState = loadState();

// Global mount root
const root = document.querySelector<HTMLElement>("#app");

export function navigate(section: AppSection): void {
  appState.activeSection = section;
  saveState(appState);
  render();
}

function render(): void {
  if (!root) return;

  // Build nav
  const navItems: { id: AppSection; label: string }[] = [
    { id: "project-list", label: "会议项目" },
    { id: "markdown-library", label: "Markdown 库" }
  ];

  const navHtml = navItems
    .map(
      (item) =>
        `<button class="nav-btn${appState.activeSection === item.id ? " active" : ""}" data-section="${item.id}">${item.label}</button>`
    )
    .join("");

  root.innerHTML = `
    <div class="app-layout">
      <nav class="nav">
        <span class="nav-brand">meetnote</span>
        <div class="nav-links">${navHtml}</div>
      </nav>
      <div class="content" id="section-root"></div>
    </div>
  `;

  // Attach nav event listeners
  root.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const section = btn.dataset["section"] as AppSection | undefined;
      if (section) navigate(section);
    });
  });

  const sectionRoot = root.querySelector<HTMLElement>("#section-root");
  if (!sectionRoot) return;

  switch (appState.activeSection) {
    case "project-list":
      renderProjectList(sectionRoot, appState);
      break;
    case "transcription-progress":
      renderTranscriptionProgress(sectionRoot, appState);
      break;
    case "recording-detail":
      renderRecordingDetail(sectionRoot, appState);
      break;
    case "speaker-editor":
      renderSpeakerEditor(sectionRoot, appState);
      break;
    case "deverbalization-preview":
      renderDeverbalizationPreview(sectionRoot, appState);
      break;
    case "markdown-library":
      renderMarkdownLibrary(sectionRoot, appState);
      break;
    default:
      renderProjectList(sectionRoot, appState);
  }
}

// Initial render
render();
