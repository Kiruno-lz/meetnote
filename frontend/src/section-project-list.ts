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
    .meeting-item { display: flex; align-items: center; }
    .meeting-item:hover .meeting-item-actions { opacity: 1 !important; }
    .btn-danger { background: #c62828; color: #fff; border: none; }
    .btn-danger:hover { background: #b71c1c; }
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
        <div class="meeting-item-actions" style="opacity:0; transition: opacity 0.15s; display:flex; gap:6px; align-items:center;">
          <button class="btn btn-sm btn-secondary btn-edit" data-id="${escapeHtml(m.id)}">✏️</button>
          <button class="btn btn-sm btn-danger btn-delete" data-id="${escapeHtml(m.id)}">🗑️</button>
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
      <input type="file" id="audio-file-input" accept=".wav,.mp3,.m4a" style="display:none;" />
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

      // Replace body content with inline editor
      body.innerHTML = `
        <input type="text" class="edit-title-input" value="${escapeHtml(meeting.title)}" style="flex:1; padding:4px 8px; font-size:14px; border:1px solid #aaa; border-radius:4px; width:100%;" />
        <button class="btn btn-sm btn-primary btn-confirm-edit" style="margin-top:6px;">确认</button>
      `;

      const input = body.querySelector<HTMLInputElement>(".edit-title-input");
      const confirmBtn = body.querySelector<HTMLButtonElement>(".btn-confirm-edit");

      if (input) {
        input.focus();
        input.select();
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
      });

      confirmBtn?.addEventListener("click", (e) => {
        e.stopPropagation();
        submitEdit();
      });
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
