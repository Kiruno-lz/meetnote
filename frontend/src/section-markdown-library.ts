import type { AppState } from "./ui-state.ts";
import { saveState } from "./ui-state.ts";
import { navigate } from "./main.ts";

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function formatDate(isoString: string): string {
  try {
    const d = new Date(isoString);
    return d.toLocaleDateString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit" });
  } catch {
    return isoString;
  }
}

export function renderMarkdownLibrary(root: HTMLElement, state: AppState): void {
  const hasItems = state.meetings.length > 0;

  const listHtml = hasItems
    ? state.meetings
        .map(
          (m) => `
      <div class="card meeting-library-item" style="margin-bottom:12px;cursor:pointer;" data-id="${escapeHtml(m.id)}">
        <div class="meeting-title">${escapeHtml(m.title)}</div>
        <div class="meeting-date">${escapeHtml(formatDate(m.createdAt))}</div>
      </div>`
        )
        .join("")
    : `<div class="empty-state">尚无保存的 Markdown</div>`;

  root.innerHTML = `
    <div class="section">
      <div class="section-header">
        <h2>Markdown 库</h2>
      </div>
      <div id="library-list">${listHtml}</div>
      <div style="margin-top:24px;">
        <button class="btn btn-secondary" id="btn-home">返回首页</button>
      </div>
    </div>
  `;

  // Click card → navigate to deverbalization-preview
  root.querySelectorAll<HTMLElement>(".meeting-library-item").forEach((card) => {
    card.addEventListener("click", () => {
      const id = card.dataset["id"];
      if (!id) return;
      state.activeMeetingId = id;
      state.activeSection = "deverbalization-preview";
      state.deverPreviewBackTo = "markdown-library";
      saveState(state);
      navigate("deverbalization-preview");
    });
  });

  root.querySelector("#btn-home")?.addEventListener("click", () => navigate("project-list"));
}
