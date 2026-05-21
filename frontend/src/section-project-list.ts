import type { AppState, MeetingRecord } from "./ui-state.ts";
import { saveState } from "./ui-state.ts";
import { runMockPipeline } from "./mock-pipeline.ts";
import { DEMO_TRANSCRIPT_SEGMENTS, DEMO_SPEAKER_SEGMENTS, DEMO_SPEAKER_PROFILES } from "./mock-fixtures.ts";
import { validateAudioFile, computeFileHash, extractDefaultTitle } from "./audio-file-import.ts";
import { updateMeetingTitle, deleteMeeting, isTitleValid } from "./meeting-management.ts";
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

async function getAudioDuration(file: File): Promise<number> {
  return new Promise((resolve) => {
    const audio = new Audio();
    const url = URL.createObjectURL(file);
    audio.onloadedmetadata = () => {
      URL.revokeObjectURL(url);
      resolve(isFinite(audio.duration) ? audio.duration : 0);
    };
    audio.onerror = () => {
      URL.revokeObjectURL(url);
      resolve(0);
    };
    audio.src = url;
  });
}

function injectStyles(): void {
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
  `;
  document.head.appendChild(style);
}

export function renderProjectList(root: HTMLElement, state: AppState): void {
  injectStyles();

  const hasItems = state.meetings.length > 0;

  const listHtml = hasItems
    ? state.meetings
        .map(
          (m) => `
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
      </div>`
        )
        .join("")
    : `<div class="empty-state">暂无会议记录<br><span style="font-size:13px;color:#888;">点击下方按钮导入演示数据</span></div>`;

  root.innerHTML = `
    <div class="section">
      <div class="section-header">
        <h2>会议项目</h2>
      </div>
      <div id="meeting-list">${listHtml}</div>
      <div id="import-error" style="color:#c62828; font-size:13px; min-height:20px; margin-top:8px;"></div>
      <div style="margin-top:16px; display:flex; gap:12px; flex-wrap:wrap;">
        <button class="btn btn-primary" id="btn-import-audio">导入音频文件</button>
        <button class="btn btn-secondary" id="btn-import-mock">导入 Mock 音频</button>
      </div>
      <input type="file" id="audio-file-input" accept="audio/wav,audio/mpeg,audio/mp4,audio/x-m4a,.wav,.mp3,.m4a" style="display:none;" />
    </div>
  `;

  // Attach meeting item body click events (navigate)
  root.querySelectorAll<HTMLElement>(".meeting-item-body").forEach((el) => {
    el.addEventListener("click", () => {
      const id = el.dataset["id"];
      if (id) {
        state.activeMeetingId = id;
        state.activeSection = "recording-detail";
        saveState(state);
        navigate("recording-detail");
      }
    });
  });

  // Attach edit button events
  root.querySelectorAll<HTMLButtonElement>(".btn-edit").forEach((btn) => {
    btn.addEventListener("click", (event) => {
      event.stopPropagation();
      const id = btn.dataset["id"];
      if (!id) return;
      const meeting = state.meetings.find((m) => m.id === id);
      if (!meeting) return;

      const card = root.querySelector<HTMLElement>(`.meeting-item[data-id="${CSS.escape(id)}"]`);
      if (!card) return;
      const body = card.querySelector<HTMLElement>(".meeting-item-body");
      if (!body) return;
      const actionsDiv = card.querySelector<HTMLElement>(".meeting-item-actions");
      const editBtn = actionsDiv?.querySelector<HTMLButtonElement>(".btn-edit");

      // Switch to editing state
      card.classList.add("editing");
      if (editBtn) {
        editBtn.className = "icon-btn confirm btn-confirm-edit";
        editBtn.title = "确认";
        editBtn.setAttribute("aria-label", "确认");
        editBtn.innerHTML = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 7l4 4 6-6"/></svg>`;
      }

      // Replace body content with inline editor
      body.innerHTML = `
        <input type="text" class="edit-title-input" value="${escapeHtml(meeting.title)}" style="flex:1; padding:4px 8px; font-size:14px; border:1px solid #aaa; border-radius:4px; width:100%;" />
      `;

      const input = body.querySelector<HTMLInputElement>(".edit-title-input");
      if (input) {
        input.focus();
        input.select();
      }

      function restoreEditBtn(): void {
        card!.classList.remove("editing");
        const confirmBtn = actionsDiv?.querySelector<HTMLButtonElement>(".btn-confirm-edit");
        if (confirmBtn) {
          confirmBtn.className = "icon-btn btn-edit";
          confirmBtn.title = "重命名";
          confirmBtn.setAttribute("aria-label", "重命名");
          confirmBtn.innerHTML = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M10 2l2 2-7 7H3V9l7-7z"/></svg>`;
          confirmBtn.dataset["id"] = id;
        }
      }

      function submitEdit(): void {
        if (!input) return;
        const newTitle = input.value;
        if (!isTitleValid(newTitle)) {
          input.classList.remove("shake");
          // Force reflow to restart animation
          void input.offsetWidth;
          input.classList.add("shake");
          return;
        }
        state.meetings = updateMeetingTitle(state.meetings, id!, newTitle);
        saveState(state);
        renderProjectList(root, state);
      }

      input?.addEventListener("keydown", (e) => {
        if (e.key === "Enter") submitEdit();
        if (e.key === "Escape") {
          restoreEditBtn();
          if (body) body.innerHTML = `
            <div class="meeting-title">${escapeHtml(meeting.title)}</div>
            <div class="meeting-date">${escapeHtml(formatDate(meeting.createdAt))}</div>
          `;
        }
      });

      // Attach confirm click on the swapped button
      if (editBtn) {
        editBtn.onclick = (e) => {
          e.stopPropagation();
          submitEdit();
        };
      }
    });
  });

  // Attach delete button events
  root.querySelectorAll<HTMLButtonElement>(".btn-delete").forEach((btn) => {
    btn.addEventListener("click", (event) => {
      event.stopPropagation();
      const id = btn.dataset["id"];
      if (!id) return;
      const meeting = state.meetings.find((m) => m.id === id);
      if (!meeting) return;

      if (confirm(`确认删除「${meeting.title}」吗？`)) {
        state.meetings = deleteMeeting(state.meetings, id);
        saveState(state);
        renderProjectList(root, state);
      }
    });
  });

  // Attach real audio import button
  const importAudioBtn = root.querySelector<HTMLButtonElement>("#btn-import-audio");
  const fileInput = root.querySelector<HTMLInputElement>("#audio-file-input");
  const errorDiv = root.querySelector<HTMLDivElement>("#import-error");

  if (importAudioBtn && fileInput) {
    importAudioBtn.addEventListener("click", () => {
      fileInput.value = "";
      fileInput.click();
    });

    fileInput.addEventListener("change", () => {
      const file = fileInput.files?.[0];
      if (!file) return;
      void handleRealAudioImport(file, state, root, importAudioBtn, errorDiv);
    });
  }

  // Attach mock import button
  const mockImportBtn = root.querySelector<HTMLButtonElement>("#btn-import-mock");
  if (mockImportBtn) {
    mockImportBtn.addEventListener("click", () => {
      startMockImport(state);
    });
  }
}

async function handleRealAudioImport(
  file: File,
  state: AppState,
  root: HTMLElement,
  importBtn: HTMLButtonElement,
  errorDiv: HTMLDivElement | null
): Promise<void> {
  // Clear previous errors
  if (errorDiv) errorDiv.textContent = "";

  // Validate
  const validation = validateAudioFile(file.name, file.size);
  if (!validation.ok) {
    showImportError(errorDiv, validation.errors.join("；"));
    return;
  }

  // Show loading state
  const originalText = importBtn.textContent ?? "导入音频文件";
  importBtn.disabled = true;
  importBtn.textContent = "正在分析文件...";

  try {
    const buffer = await file.arrayBuffer();
    const hash = await computeFileHash(buffer);
    const durationSeconds = await getAudioDuration(file);
    const title = extractDefaultTitle(file.name);

    const result = runMockPipeline({
      audioPath: file.name,
      audioHash: hash,
      durationSeconds,
      title,
      mockTranscriptSegments: DEMO_TRANSCRIPT_SEGMENTS,
      mockSpeakerSegments: DEMO_SPEAKER_SEGMENTS,
      mockSpeakerProfiles: DEMO_SPEAKER_PROFILES
    });

    const record: MeetingRecord = {
      id: result.project.id,
      title: result.project.title,
      createdAt: result.project.createdAt,
      markdownContent: result.markdownContent,
      speakerProfiles: result.speakerProfiles,
      projectSnapshot: JSON.stringify(result.project)
    };

    // Upsert: avoid duplicates by id (same file re-imported updates in place)
    const existing = state.meetings.findIndex((m) => m.id === record.id);
    if (existing >= 0) {
      state.meetings[existing] = record;
    } else {
      state.meetings.unshift(record);
    }

    state.activeMeetingId = record.id;
    state.pipelineRunning = true;
    state.pipelineStage = "queued";
    saveState(state);

    navigate("transcription-progress");
  } catch (err) {
    importBtn.disabled = false;
    importBtn.textContent = originalText;
    const message = err instanceof Error ? err.message : "导入失败，请重试";
    showImportError(errorDiv, message);
  }
}

function showImportError(errorDiv: HTMLDivElement | null, message: string): void {
  if (!errorDiv) return;
  errorDiv.textContent = message;
  setTimeout(() => {
    errorDiv.textContent = "";
  }, 3000);
}

function startMockImport(state: AppState): void {
  const uniqueHash = `demo${Date.now()}`;
  const result = runMockPipeline({
    audioPath: `/demo/mock-meeting-${Date.now()}.wav`,
    audioHash: uniqueHash,
    durationSeconds: 36,
    title: `演示会议 ${new Date().toLocaleString("zh-CN")}`,
    mockTranscriptSegments: DEMO_TRANSCRIPT_SEGMENTS,
    mockSpeakerSegments: DEMO_SPEAKER_SEGMENTS,
    mockSpeakerProfiles: DEMO_SPEAKER_PROFILES
  });

  const record: MeetingRecord = {
    id: result.project.id,
    title: result.project.title,
    createdAt: result.project.createdAt,
    markdownContent: result.markdownContent,
    speakerProfiles: result.speakerProfiles,
    projectSnapshot: JSON.stringify(result.project)
  };

  // Avoid duplicates by id
  const existing = state.meetings.findIndex((m) => m.id === record.id);
  if (existing >= 0) {
    state.meetings[existing] = record;
  } else {
    state.meetings.unshift(record);
  }

  state.activeMeetingId = record.id;
  state.pipelineRunning = true;
  state.pipelineStage = "queued";
  saveState(state);

  navigate("transcription-progress");
}
