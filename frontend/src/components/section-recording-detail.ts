import type { AppState } from "../stores/ui-state.ts";
import { saveState } from "../stores/ui-state.ts";
import type { MeetingProject } from "../types/audio-project.ts";
import { navigate } from "../main.ts";
import { updateMeetingTitle, deleteMeeting, isTitleValid } from "../utils/meeting-management.ts";

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function msToTime(ms: number): string {
  const totalSecs = Math.floor(ms / 1000);
  const mins = Math.floor(totalSecs / 60);
  const secs = totalSecs % 60;
  return `${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
}

const SPEAKER_COLORS = ["#4a9eff", "#ff7043", "#66bb6a", "#ab47bc"];

function injectDetailStyles(): void {
  if (document.getElementById("recording-detail-styles")) return;
  const style = document.createElement("style");
  style.id = "recording-detail-styles";
  style.textContent = `
    .dropdown-menu {
      position: absolute;
      right: 0;
      top: calc(100% + 4px);
      background: #252525;
      border: 1px solid #333;
      border-radius: 8px;
      padding: 4px;
      min-width: 120px;
      z-index: 200;
      box-shadow: 0 4px 16px rgba(0,0,0,0.4);
    }
    .dropdown-item {
      display: block;
      width: 100%;
      text-align: left;
      padding: 7px 12px;
      border-radius: 5px;
      border: none;
      background: transparent;
      color: #ccc;
      font-size: 13px;
      cursor: pointer;
      transition: background 120ms ease-out;
    }
    .dropdown-item:hover { background: #2e2e2e; color: #e0e0e0; }
    .dropdown-item.danger:hover { background: rgba(198,40,40,0.12); color: #ef5350; }
  `;
  document.head.appendChild(style);
}

function getSpeakerColor(speakerId: string, profiles: { id: string; label: string }[]): string {
  const idx = profiles.findIndex((p) => p.id === speakerId);
  return SPEAKER_COLORS[((idx >= 0 ? idx : 0) % SPEAKER_COLORS.length)] ?? SPEAKER_COLORS[0] ?? "#4a9eff";
}

export function renderRecordingDetail(root: HTMLElement, state: AppState): void {
  injectDetailStyles();
  const _rawMeeting = state.meetings.find((m) => m.id === state.activeMeetingId);

  if (!_rawMeeting) {
    root.innerHTML = `
      <div class="section">
        <div class="empty-state">未找到会议记录</div>
        <button class="btn btn-secondary" id="btn-back">返回首页</button>
      </div>
    `;
    root.querySelector("#btn-back")?.addEventListener("click", () => navigate("project-list"));
    return;
  }

  const meeting = _rawMeeting;

  let project: MeetingProject;
  try {
    project = JSON.parse(meeting.projectSnapshot) as MeetingProject;
  } catch {
    root.innerHTML = `<div class="section"><div class="empty-state">数据解析失败</div></div>`;
    return;
  }

  const profiles = meeting.speakerProfiles;
  const speakerMap = new Map(profiles.map((p) => [p.id, p.label]));

  // Build speaker assignment map: transcriptSegId -> speakerId
  const segSpeakerMap = new Map<string, string>();
  for (const ss of project.speakerSegments) {
    // Match transcript segments that overlap with this speaker segment
    for (const ts of project.transcript) {
      const overlap = Math.min(ts.endMs, ss.endMs) - Math.max(ts.startMs, ss.startMs);
      if (overlap > 0) {
        segSpeakerMap.set(ts.id, ss.speakerId);
      }
    }
  }

  const segmentsHtml = project.transcript
    .map((seg) => {
      const speakerId = segSpeakerMap.get(seg.id) ?? "unknown";
      const speakerLabel = speakerMap.get(speakerId) ?? speakerId;
      const color = getSpeakerColor(speakerId, profiles);
      return `
        <div class="transcript-segment">
          <div class="segment-meta">
            <span class="speaker-tag" style="background:${escapeHtml(color)}20;color:${escapeHtml(color)};border:1px solid ${escapeHtml(color)}40;">
              ${escapeHtml(speakerLabel)}
            </span>
            <span class="segment-time">${escapeHtml(msToTime(seg.startMs))} – ${escapeHtml(msToTime(seg.endMs))}</span>
          </div>
          <div class="segment-text">${escapeHtml(seg.text)}</div>
        </div>`;
    })
    .join("");

  root.innerHTML = `
    <div class="section">
      <div class="section-header">
        <button class="btn btn-secondary btn-sm" id="btn-back">← 返回</button>
        <h2 id="detail-title">${escapeHtml(meeting.title)}</h2>
        <div class="title-actions" style="position:relative;margin-left:auto;">
          <button class="icon-btn" id="btn-title-menu" title="更多操作" aria-label="更多操作" style="color:#666;font-size:18px;letter-spacing:1px;font-weight:700;">···</button>
          <div id="title-dropdown" class="dropdown-menu" style="display:none;">
            <button class="dropdown-item" id="menu-rename">重命名</button>
            <button class="dropdown-item danger" id="menu-delete">删除</button>
          </div>
        </div>
      </div>
      <div class="transcript-list">
        ${segmentsHtml || '<div class="empty-state">无转录内容</div>'}
      </div>
      <div class="action-bar">
        <button class="btn btn-secondary" id="btn-speaker-editor">修正 Speaker</button>
        <button class="btn btn-primary" id="btn-deverb">一键去口语化</button>
      </div>
    </div>
  `;

  root.querySelector("#btn-back")?.addEventListener("click", () => navigate("project-list"));

  // ··· menu toggle
  const menuBtn = root.querySelector<HTMLButtonElement>("#btn-title-menu");
  const dropdown = root.querySelector<HTMLElement>("#title-dropdown");
  menuBtn?.addEventListener("click", (e) => {
    e.stopPropagation();
    if (dropdown) dropdown.style.display = dropdown.style.display === "none" ? "block" : "none";
  });
  const closeDropdown = () => {
    if (dropdown) dropdown.style.display = "none";
  };
  document.addEventListener("click", closeDropdown);

  // Rename action
  function enterRenameMode(): void {
    const titleEl = root.querySelector<HTMLElement>("#detail-title");
    if (!titleEl) return;
    const input = document.createElement("input");
    input.type = "text";
    input.value = meeting.title;
    input.style.cssText =
      "font-size:inherit;font-weight:inherit;background:#1a1a1a;border:1px solid #4a9eff;border-radius:4px;color:#fff;padding:2px 6px;outline:none;width:100%;max-width:400px;";
    titleEl.replaceWith(input);
    input.focus();
    input.select();

    let saved = false;
    function saveTitle(): void {
      if (saved) return;
      saved = true;
      const newTitle = input.value.trim();
      const h2 = document.createElement("h2");
      h2.id = "detail-title";
      if (newTitle && isTitleValid(newTitle) && newTitle !== meeting.title) {
        meeting.title = newTitle;
        state.meetings = updateMeetingTitle(state.meetings, meeting.id, newTitle);
        saveState(state);
      }
      h2.textContent = meeting.title;
      input.replaceWith(h2);
    }

    input.addEventListener("blur", saveTitle);
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        input.blur(); // triggers blur → saveTitle
      }
      if (e.key === "Escape") {
        saved = true; // prevent blur from saving
        const h2 = document.createElement("h2");
        h2.id = "detail-title";
        h2.textContent = meeting.title;
        input.replaceWith(h2);
      }
    });
  }

  // Delete action
  function handleDelete(): void {
    if (confirm(`确认删除「${meeting.title}」吗？`)) {
      state.meetings = deleteMeeting(state.meetings, meeting.id);
      state.activeMeetingId = null;
      saveState(state);
      navigate("project-list");
    }
  }

  root.querySelector("#menu-rename")?.addEventListener("click", (e) => {
    e.stopPropagation();
    closeDropdown();
    enterRenameMode();
  });

  root.querySelector("#menu-delete")?.addEventListener("click", (e) => {
    e.stopPropagation();
    closeDropdown();
    handleDelete();
  });

  root.querySelector("#btn-speaker-editor")?.addEventListener("click", () => {
    saveState(state);
    navigate("speaker-editor");
  });

  root.querySelector("#btn-deverb")?.addEventListener("click", () => {
    saveState(state);
    navigate("deverbalization-preview");
  });
}
