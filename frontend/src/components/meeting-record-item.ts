import type { MeetingRecord } from "../stores/ui-state.ts";

// Shared HTML escape + date format (remove duplicates from section files)
export function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export function formatDate(isoString: string): string {
  try {
    const d = new Date(isoString);
    return d.toLocaleDateString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit" });
  } catch {
    return isoString;
  }
}

// RecordActions controls which action UI is rendered on a meeting card:
// - false (default): no action buttons, simple clickable card
// - "inline": inline edit + delete icon buttons (used by project-list)
// - "menu": a ··· button opening a dropdown with rename + delete (used by markdown-library)
export type RecordActions = false | "inline" | "menu";

// Inject shared card styles once (idempotent)
export function injectMeetingItemStyles(): void {
  if (document.getElementById("meeting-list-styles")) return;
  const style = document.createElement("style");
  style.id = "meeting-list-styles";
  style.textContent = `
    .meeting-item { display: flex; align-items: center; gap: 0; }
    .meeting-item-actions {
      display: flex;
      align-items: center;
      gap: 4px;
      opacity: 0;
      transition: opacity 200ms ease-out;
      flex-shrink: 0;
    }
    .meeting-item:hover .meeting-item-actions { opacity: 1; }
    .meeting-item.editing .meeting-item-actions { opacity: 1; }
    .icon-btn {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 28px;
      height: 28px;
      border-radius: 6px;
      border: 1px solid transparent;
      background: transparent;
      cursor: pointer;
      color: #888;
      transition: background 150ms ease-out, color 150ms ease-out, border-color 150ms ease-out;
      flex-shrink: 0;
    }
    .icon-btn:hover { background: #2a2a2a; border-color: #3a3a3a; color: #e0e0e0; }
    .icon-btn:active { background: #333; }
    .icon-btn:focus-visible { outline: 2px solid #4a9eff; outline-offset: 1px; }
    .icon-btn.danger:hover { background: rgba(198, 40, 40, 0.15); border-color: rgba(198, 40, 40, 0.3); color: #ef5350; }
    .icon-btn.confirm { color: #66bb6a; }
    .icon-btn.confirm:hover { background: rgba(102, 187, 106, 0.15); border-color: rgba(102, 187, 106, 0.3); color: #66bb6a; }
    @keyframes shake {
      0%, 100% { transform: translateX(0); }
      20%, 60% { transform: translateX(-4px); }
      40%, 80% { transform: translateX(4px); }
    }
    .shake { animation: shake 0.35s ease; }
    .dropdown-menu {
      position: absolute;
      top: calc(100% + 4px);
      right: 0;
      background: #1e1e1e;
      border: 1px solid #3a3a3a;
      border-radius: 8px;
      box-shadow: 0 4px 16px rgba(0,0,0,0.4);
      min-width: 120px;
      z-index: 100;
      overflow: hidden;
    }
    .dropdown-item {
      display: block;
      width: 100%;
      padding: 8px 14px;
      background: transparent;
      border: none;
      color: #e0e0e0;
      font-size: 13px;
      text-align: left;
      cursor: pointer;
      transition: background 150ms ease-out;
    }
    .dropdown-item:hover { background: #2a2a2a; }
    .dropdown-item.danger { color: #ef5350; }
    .dropdown-item.danger:hover { background: rgba(198, 40, 40, 0.15); }
  `;
  document.head.appendChild(style);
}

// Render a single meeting record card as an HTML string.
// showActions="inline" adds edit+delete icon buttons (used by project list).
// showActions="menu" adds a ··· dropdown button (used by markdown library).
// showActions=false (default) renders a simple clickable card.
export function renderMeetingItemHtml(m: MeetingRecord, showActions: RecordActions = false): string {
  if (showActions === "inline") {
    return `
      <div class="card meeting-item" data-id="${escapeHtml(m.id)}">
        <div class="meeting-item-body" style="flex:1; cursor:pointer;" data-id="${escapeHtml(m.id)}">
          <div class="meeting-title">${escapeHtml(m.title)}</div>
          <div class="meeting-date">${escapeHtml(formatDate(m.createdAt))}</div>
        </div>
        <div class="meeting-item-actions">
          <button class="icon-btn btn-edit" data-id="${escapeHtml(m.id)}" title="重命名" aria-label="重命名">
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M10 2l2 2-7 7H3V9l7-7z"/>
            </svg>
          </button>
          <button class="icon-btn danger btn-delete" data-id="${escapeHtml(m.id)}" title="删除" aria-label="删除">
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M2 4h10M5 4V2h4v2M3 4l1 8h6l1-8"/>
            </svg>
          </button>
        </div>
      </div>`;
  }

  if (showActions === "menu") {
    return `
      <div class="card meeting-item" data-id="${escapeHtml(m.id)}">
        <div class="meeting-item-body" style="flex:1; cursor:pointer;" data-id="${escapeHtml(m.id)}">
          <div class="meeting-title">${escapeHtml(m.title)}</div>
          <div class="meeting-date">${escapeHtml(formatDate(m.createdAt))}</div>
        </div>
        <div class="meeting-item-actions">
          <div class="meeting-item-menu-wrap" style="position:relative;">
            <button class="icon-btn btn-menu" data-id="${escapeHtml(m.id)}" title="更多操作" aria-label="更多操作"
                    style="font-size:16px;letter-spacing:1px;font-weight:700;">···</button>
            <div class="dropdown-menu" style="display:none;">
              <button class="dropdown-item btn-menu-rename" data-id="${escapeHtml(m.id)}">重命名</button>
              <button class="dropdown-item danger btn-menu-delete" data-id="${escapeHtml(m.id)}">删除</button>
            </div>
          </div>
        </div>
      </div>`;
  }

  return `
    <div class="card meeting-item" style="margin-bottom:12px;cursor:pointer;" data-id="${escapeHtml(m.id)}">
      <div class="meeting-item-body" data-id="${escapeHtml(m.id)}">
        <div class="meeting-title">${escapeHtml(m.title)}</div>
        <div class="meeting-date">${escapeHtml(formatDate(m.createdAt))}</div>
      </div>
    </div>`;
}
