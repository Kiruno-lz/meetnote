import type { AppState, MeetingRecord } from "../stores/ui-state.ts";
import { saveState } from "../stores/ui-state.ts";
import type { MeetingProject } from "../types/audio-project.ts";
import { navigate } from "../main.ts";
import {
  createMarkdownEditor,
  isMarkdownBodyChanged,
} from "./markdown-editor-component.ts";
import { updateMeetingTitle, deleteMeeting, isTitleValid } from "../utils/meeting-management.ts";
import { injectMeetingItemStyles } from "./meeting-record-item.ts";

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
// Abort controller for document-level click listener cleanup on re-render
let _deverbClickAbort: AbortController | null = null;

export function renderDeverbalizationPreview(root: HTMLElement, state: AppState): void {
  const meetingOrUndef = state.meetings.find((m) => m.id === state.activeMeetingId);

  if (!meetingOrUndef) {
    root.innerHTML = `
      <div class="section">
        <div class="empty-state">未找到会议记录</div>
        <button class="btn btn-secondary" id="btn-back">返回</button>
      </div>
    `;
    root.querySelector("#btn-back")?.addEventListener("click", () => navigate("recording-detail"));
    return;
  }

  const meeting: MeetingRecord = meetingOrUndef;

  let project: MeetingProject;
  try {
    project = JSON.parse(meeting.projectSnapshot) as MeetingProject;
  } catch {
    root.innerHTML = `<div class="section"><div class="empty-state">数据解析失败</div></div>`;
    return;
  }

  // Track current markdown content for edits
  let currentMarkdown = meeting.markdownContent;

  const profiles = meeting.speakerProfiles;
  const speakerMap = new Map(profiles.map((p) => [p.id, p.label]));

  // Build speaker assignment map for transcript segments
  const segSpeakerMap = new Map<string, string>();
  for (const ss of project.speakerSegments) {
    for (const ts of project.transcript) {
      const overlap = Math.min(ts.endMs, ss.endMs) - Math.max(ts.startMs, ss.startMs);
      if (overlap > 0) {
        segSpeakerMap.set(ts.id, ss.speakerId);
      }
    }
  }

  // Group transcript segments by speaker (consecutive runs)
  type SpeakerBlock = { speakerId: string; texts: string[]; startMs: number; endMs: number };
  const blocks: SpeakerBlock[] = [];

  for (const seg of project.transcript) {
    const speakerId = segSpeakerMap.get(seg.id) ?? "unknown";
    const last = blocks[blocks.length - 1];
    if (last && last.speakerId === speakerId) {
      last.texts.push(seg.text);
      last.endMs = seg.endMs;
    } else {
      blocks.push({ speakerId, texts: [seg.text], startMs: seg.startMs, endMs: seg.endMs });
    }
  }

  const leftHtml = blocks
    .map((b) => {
      const label = speakerMap.get(b.speakerId) ?? b.speakerId;
      const color = SPEAKER_COLORS[profiles.findIndex((p) => p.id === b.speakerId) % SPEAKER_COLORS.length] ?? "#4a9eff";
      return `
        <div class="deverb-block" style="margin-bottom:16px;">
          <div class="segment-meta" style="margin-bottom:4px;">
            <span class="speaker-tag" style="background:${escapeHtml(color)}20;color:${escapeHtml(color)};border:1px solid ${escapeHtml(color)}40;">
              ${escapeHtml(label)}
            </span>
            <span class="segment-time">${escapeHtml(msToTime(b.startMs))} – ${escapeHtml(msToTime(b.endMs))}</span>
          </div>
          <div class="segment-text">${escapeHtml(b.texts.join(" "))}</div>
        </div>`;
    })
    .join("");

  const backTarget = state.deverPreviewBackTo ?? "recording-detail";

  injectMeetingItemStyles();

  root.innerHTML = `
    <div class="section">
      <div class="section-header">
        <button class="btn btn-secondary btn-sm" id="btn-back">← 返回</button>
        <h2 id="deverb-title">${escapeHtml(meeting.title)}</h2>
        <div class="title-actions" style="position:relative;margin-left:auto;">
          <button class="icon-btn" id="btn-title-menu" title="更多操作" aria-label="更多操作"
                  style="color:#666;font-size:18px;letter-spacing:1px;font-weight:700;">···</button>
          <div id="title-dropdown" class="dropdown-menu" style="display:none;">
            <button class="dropdown-item" id="menu-rename">重命名</button>
            <button class="dropdown-item danger" id="menu-delete">删除</button>
          </div>
        </div>
      </div>
      <div class="deverb-split">
        <div class="deverb-left">
          <h3 style="margin-bottom:12px;color:#aaa;font-size:14px;font-weight:600;text-transform:uppercase;letter-spacing:.05em;">原始转录</h3>
          ${leftHtml || '<div class="empty-state">无内容</div>'}
        </div>
        <div class="deverb-divider"></div>
        <div class="deverb-right">
          <h3 style="margin-bottom:12px;color:#aaa;font-size:14px;font-weight:600;text-transform:uppercase;letter-spacing:.05em;">去口语化 Markdown（可编辑）</h3>
          <div class="deverb-right-content" style="flex:1;display:flex;flex-direction:column;min-height:0;"></div>
        </div>
      </div>
      <div class="action-bar">
        <button class="btn btn-primary" id="btn-save-markdown">保存到 Markdown 库</button>
      </div>
    </div>
  `;

  // Inject the markdown editor into the right-side container
  const rightContent = root.querySelector<HTMLElement>(".deverb-right-content");
  if (rightContent) {
    const editor = createMarkdownEditor({
      initialContent: meeting.markdownContent,
      readonly: false,
      onchange: (content) => {
        currentMarkdown = content;
      },
    });
    rightContent.appendChild(editor);
  }

  root.querySelector("#btn-back")?.addEventListener("click", () => navigate(backTarget));

  // ··· dropdown toggle
  const menuBtn = root.querySelector<HTMLButtonElement>("#btn-title-menu");
  const dropdown = root.querySelector<HTMLElement>("#title-dropdown");
  menuBtn?.addEventListener("click", (e) => {
    e.stopPropagation();
    if (dropdown) dropdown.style.display = dropdown.style.display === "none" ? "block" : "none";
  });
  // Remove previous document click listener (module-level abort signal)
  if (typeof _deverbClickAbort !== "undefined") _deverbClickAbort?.abort();
  _deverbClickAbort = new AbortController();
  const closeDropdown = (): void => { if (dropdown) dropdown.style.display = "none"; };
  document.addEventListener("click", closeDropdown, { signal: _deverbClickAbort.signal });

  // Rename inline-edit
  function enterRenameMode(): void {
    const titleEl = root.querySelector<HTMLElement>("#deverb-title");
    if (!titleEl) return;
    const input = document.createElement("input");
    input.type = "text";
    input.value = meeting.title;
    input.style.cssText = "font-size:inherit;font-weight:inherit;background:#1a1a1a;border:1px solid #4a9eff;border-radius:4px;color:#fff;padding:2px 6px;outline:none;width:100%;max-width:400px;";
    titleEl.replaceWith(input);
    input.focus(); input.select();

    let saved = false;
    function saveTitle(): void {
      if (saved) return; saved = true;
      const newTitle = input.value.trim();
      const h2 = document.createElement("h2"); h2.id = "deverb-title";
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
      if (e.key === "Enter") input.blur();
      if (e.key === "Escape") {
        saved = true;
        const h2 = document.createElement("h2"); h2.id = "deverb-title";
        h2.textContent = meeting.title; input.replaceWith(h2);
      }
    });
  }

  // Delete action
  function handleDelete(): void {
    if (confirm(`确认删除「${meeting.title}」吗？`)) {
      state.meetings = deleteMeeting(state.meetings, meeting.id);
      state.activeMeetingId = null;
      saveState(state);
      navigate(backTarget);
    }
  }

  root.querySelector("#menu-rename")?.addEventListener("click", (e) => { e.stopPropagation(); closeDropdown(); enterRenameMode(); });
  root.querySelector("#menu-delete")?.addEventListener("click", (e) => { e.stopPropagation(); closeDropdown(); handleDelete(); });

  root.querySelector("#btn-save-markdown")?.addEventListener("click", async () => {
    if (isMarkdownBodyChanged(meeting.markdownContent, currentMarkdown)) {
      const idx = state.meetings.findIndex((m: MeetingRecord) => m.id === meeting.id);
      if (idx !== -1) {
        state.meetings[idx] = { ...state.meetings[idx]!, markdownContent: currentMarkdown };
      }
    }
    saveState(state);

    // Also write to filesystem if running in Tauri
    if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const safeTitle = meeting.title.replace(/[<>:"/\\|?*]/g, "_").trim() || "untitled";
        const dateStr = new Date().toISOString().split("T")[0] ?? "unknown-date";
        const slug = `${dateStr}-${safeTitle.toLowerCase().replace(/\s+/g, "-").replace(/-+/g, "-").slice(0, 80)}`;
        const baseDir = state.markdownLibraryPath || "~/Documents/meetnote";
        const mdPath = `${baseDir}/meetings/${dateStr}/${slug}/meeting.md`;
        await invoke<string>("write_text_file", { path: mdPath, content: currentMarkdown });
      } catch (err) {
        console.warn("Failed to save markdown to filesystem:", err);
        // Non-fatal — localStorage save already succeeded
      }
    }

    navigate("markdown-library");
  });
}
