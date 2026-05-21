import type { AppState } from "./ui-state.ts";
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
      <div class="card meeting-library-item" style="margin-bottom:12px;">
        <div class="library-item-header" style="cursor:pointer;display:flex;justify-content:space-between;align-items:center;" data-id="${escapeHtml(m.id)}">
          <div>
            <div class="meeting-title">${escapeHtml(m.title)}</div>
            <div class="meeting-date">${escapeHtml(formatDate(m.createdAt))}</div>
          </div>
          <span class="expand-icon" data-id="${escapeHtml(m.id)}" style="font-size:18px;color:#888;">▼</span>
        </div>
        <div class="library-markdown-content" id="md-${escapeHtml(m.id)}" style="display:none;margin-top:12px;padding-top:12px;border-top:1px solid #333;">
          <pre style="white-space:pre-wrap;font-size:12px;color:#bbb;font-family:'JetBrains Mono','Fira Code',monospace;overflow-x:auto;">${escapeHtml(m.markdownContent)}</pre>
        </div>
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

  // Toggle expand/collapse
  root.querySelectorAll<HTMLElement>(".library-item-header").forEach((header) => {
    header.addEventListener("click", () => {
      const id = header.dataset["id"];
      if (!id) return;
      const content = root.querySelector<HTMLElement>(`#md-${CSS.escape(id)}`);
      const icon = root.querySelector<HTMLElement>(`.expand-icon[data-id="${CSS.escape(id)}"]`);
      if (content) {
        const isHidden = content.style.display === "none";
        content.style.display = isHidden ? "block" : "none";
        if (icon) icon.textContent = isHidden ? "▲" : "▼";
      }
    });
  });

  root.querySelector("#btn-home")?.addEventListener("click", () => navigate("project-list"));
}
