import type { AppSection } from "./components/app-shell.ts";
import { loadState, saveState, type AppState } from "./stores/ui-state.ts";
import { renderProjectList } from "./components/section-project-list.ts";
import { renderTranscriptionProgress } from "./components/section-transcription-progress.ts";
import { renderRecordingDetail } from "./components/section-recording-detail.ts";
import { renderSpeakerEditor } from "./components/section-speaker-editor.ts";
import { renderDeverbalizationPreview } from "./components/section-deverbalization-preview.ts";
import { renderMarkdownLibrary } from "./components/section-markdown-library.ts";
import { renderModelManagement } from "./components/section-model-management.ts";
import { renderSettings } from "./components/section-settings.ts";

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
    { id: "markdown-library", label: "Markdown 库" },
    { id: "model-management", label: "模型管理" }
  ];

  const navHtml = navItems
    .map(
      (item) =>
        `<button class="nav-btn${appState.activeSection === item.id ? " active" : ""}" data-section="${item.id}">${item.label}</button>`
    )
    .join("");

  const settingsBtnHtml = `
    <button class="nav-btn nav-settings-btn${appState.activeSection === "settings" ? " active" : ""}"
            data-section="settings"
            title="设置 (⌘,)"
            aria-label="设置"
            style="font-size:16px;">⚙️</button>`;

  root.innerHTML = `
    <div class="app-layout">
      <nav class="nav">
        <span class="nav-brand">meetnote</span>
        <div class="nav-links">${navHtml}</div>
        ${settingsBtnHtml}
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
    case "model-management":
      renderModelManagement(sectionRoot, appState);
      break;
    case "settings":
      renderSettings(sectionRoot, appState);
      break;
    default:
      renderProjectList(sectionRoot, appState);
  }
}

// Keyboard shortcut: ⌘, opens settings
let settingsShortcutBound = false;

function bindSettingsShortcut(): void {
  if (settingsShortcutBound) return;
  settingsShortcutBound = true;
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === ",") {
      e.preventDefault();
      navigate("settings");
    }
  });
}

// Initial render
render();
bindSettingsShortcut();
