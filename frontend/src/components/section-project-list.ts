import type { AppState, MeetingRecord } from "../stores/ui-state.ts";
import { saveState } from "../stores/ui-state.ts";
import { runMockPipeline } from "../utils/mock-pipeline.ts";
import { DEMO_TRANSCRIPT_SEGMENTS, DEMO_SPEAKER_SEGMENTS, DEMO_SPEAKER_PROFILES } from "../utils/mock-fixtures.ts";
import { validateAudioFile, computeFileHash, extractDefaultTitle, extractDurationFromAudioElement, MockableAudioElement } from "../utils/audio-file-import.ts";
import { createMeetingProject } from "../types/audio-project.ts";
import { updateMeetingTitle, deleteMeeting, isTitleValid } from "../utils/meeting-management.ts";
import { navigate } from "../main.ts";
import { injectMeetingItemStyles, renderMeetingItemHtml, escapeHtml, formatDate } from "./meeting-record-item.ts";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function renderProjectList(root: HTMLElement, state: AppState): void {
  injectMeetingItemStyles();

  const visibleMeetings = state.pipelineRunning && state.activeMeetingId
    ? state.meetings.filter((m) => m.id !== state.activeMeetingId)
    : state.meetings;
  const hasItems = visibleMeetings.length > 0;

  const listHtml = hasItems
    ? visibleMeetings
        .map((m) => renderMeetingItemHtml(m, "inline"))
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
      // After a re-render, detached buttons must not re-enter edit mode
      if (!document.contains(btn)) return;
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

      // Replace only the title div with an inline input, preserving the date div
      const titleDiv = body.querySelector<HTMLElement>(".meeting-title");
      const dateDiv = body.querySelector<HTMLElement>(".meeting-date");

      const input = document.createElement("input");
      input.type = "text";
      input.className = "edit-title-input";
      input.value = meeting.title;
      input.style.cssText =
        "flex:1; padding:3px 8px; font-size:14px; border:1px solid #4a9eff; border-radius:4px; width:100%; background:#1a1a1a; color:#e0e0e0; outline:none;";

      if (titleDiv) {
        titleDiv.replaceWith(input);
      } else {
        body.insertBefore(input, dateDiv ?? null);
      }
      input.focus();
      input.select();

      function restoreTitleDiv(titleText: string): void {
        const restoredTitle = document.createElement("div");
        restoredTitle.className = "meeting-title";
        restoredTitle.textContent = titleText;
        // body is non-null (guarded above with `if (!body) return`)
        const editInput = body!.querySelector<HTMLInputElement>(".edit-title-input");
        if (editInput) {
          editInput.replaceWith(restoredTitle);
        }
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
        // Re-render the whole list for clean state (avoids event listener conflicts)
        renderProjectList(root, state);
      }

      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") submitEdit();
        if (e.key === "Escape") {
          // Restore original title in place, then restore the edit button
          restoreTitleDiv(meeting.title);
          restoreEditBtn();
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

  if (importAudioBtn) {
    importAudioBtn.addEventListener("click", async () => {
      if (isTauri) {
        // Tauri mode: use native dialog with enforced file-type filter
        try {
          const { open } = await import("@tauri-apps/plugin-dialog");
          const selected = await open({
            multiple: false,
            directory: false,
            filters: [{ name: "音频文件", extensions: ["wav", "mp3", "m4a"] }],
          });
          if (!selected || typeof selected !== "string") return;
          await handleNativeTauriAudioImport(selected, state, root, importAudioBtn, errorDiv);
        } catch (err) {
          showImportError(errorDiv, `文件选择失败: ${String(err)}`);
        }
      } else {
        // Browser / non-Tauri mode: fall back to <input type="file">
        if (fileInput) {
          fileInput.value = "";
          fileInput.click();
        }
      }
    });

    if (fileInput) {
      fileInput.addEventListener("change", () => {
        const file = fileInput.files?.[0];
        if (!file) return;
        void handleRealAudioImport(file, state, root, importAudioBtn, errorDiv);
      });
    }
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
    let objectUrl: string | null = null;
    let durationSeconds: number;
    try {
      objectUrl = URL.createObjectURL(file);
      durationSeconds = await extractDurationFromAudioElement(
        () => new Audio() as unknown as MockableAudioElement,
        objectUrl
      );
    } finally {
      if (objectUrl !== null) {
        URL.revokeObjectURL(objectUrl);
      }
    }
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

    // Upsert: avoid duplicates by id (same file re-imported via deterministic hash → same id → update in place)
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

async function handleNativeTauriAudioImport(
  filePath: string,
  state: AppState,
  root: HTMLElement,
  importBtn: HTMLButtonElement,
  errorDiv: HTMLDivElement | null
): Promise<void> {
  if (errorDiv) errorDiv.textContent = "";

  const filename = filePath.split("/").pop() ?? filePath.split("\\").pop() ?? "audio";

  // Get real file size from the filesystem via Tauri command
  let sizeBytes = 0;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    sizeBytes = await invoke<number>("get_file_size", { path: filePath });
  } catch {
    // silently fall through — validation will fail with 0 bytes if needed
  }

  const validation = validateAudioFile(filename, sizeBytes);
  if (!validation.ok) {
    showImportError(errorDiv, validation.errors.join("；"));
    return;
  }

  const title = extractDefaultTitle(filename);
  // Deterministic fingerprint from path + size for basic dedup.
  // Real SHA-256 would require reading the file content via Tauri fs plugin.
  let hashStr = "";
  try {
    const rawBytes = new TextEncoder().encode(`${filePath}:${sizeBytes}`);
    const hashBuf = await crypto.subtle.digest("SHA-256", rawBytes);
    hashStr = Array.from(new Uint8Array(hashBuf)).map((b) => b.toString(16).padStart(2, "0")).join("").slice(0, 32);
  } catch {
    // Fallback if crypto.subtle is unavailable (non-secure context)
    hashStr = `tauri-${Date.now()}-${filename}`;
  }
  const uniqueHash = hashStr;
  // Estimate duration from file size (conservative: ~20 KB/s for compressed audio).
  // This is a placeholder — real ASR will replace the transcript with actual timestamps.
  const durationSeconds = Math.max(1.0, sizeBytes / 20000);

  const originalText = importBtn.textContent ?? "导入音频文件";
  importBtn.disabled = true;
  importBtn.textContent = "正在准备...";

  try {
    const emptyProject = createMeetingProject({
      path: filePath,
      hash: uniqueHash,
      durationSeconds,
      createdAt: new Date().toISOString(),
      title,
    });

    const record: MeetingRecord = {
      id: emptyProject.id,
      title: emptyProject.title,
      createdAt: emptyProject.createdAt,
      markdownContent: "",
      speakerProfiles: [],
      projectSnapshot: JSON.stringify(emptyProject),
    };

    const existing = state.meetings.findIndex((m) => m.id === record.id);
    if (existing >= 0) {
      state.meetings[existing] = record;
    } else {
      state.meetings.unshift(record);
    }

    state.activeMeetingId = record.id;
    state.pipelineRunning = true;
    state.pipelineStage = "queued";
    state.pendingAudioPath = filePath;
    state.useRealAsr = true;
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
