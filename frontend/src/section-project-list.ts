import type { AppState, MeetingRecord } from "./ui-state.ts";
import { saveState } from "./ui-state.ts";
import { runMockPipeline } from "./mock-pipeline.ts";
import { DEMO_TRANSCRIPT_SEGMENTS, DEMO_SPEAKER_SEGMENTS, DEMO_SPEAKER_PROFILES } from "./mock-fixtures.ts";
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

export function renderProjectList(root: HTMLElement, state: AppState): void {
  const hasItems = state.meetings.length > 0;

  const listHtml = hasItems
    ? state.meetings
        .map(
          (m) => `
      <div class="card meeting-item" data-id="${escapeHtml(m.id)}" style="cursor:pointer;">
        <div class="meeting-title">${escapeHtml(m.title)}</div>
        <div class="meeting-date">${escapeHtml(formatDate(m.createdAt))}</div>
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
      <div style="margin-top:24px;">
        <button class="btn btn-primary" id="btn-import-mock">导入 Mock 音频</button>
      </div>
    </div>
  `;

  // Attach meeting item click events
  root.querySelectorAll<HTMLElement>(".meeting-item").forEach((el) => {
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

  // Attach import button
  const importBtn = root.querySelector<HTMLButtonElement>("#btn-import-mock");
  if (importBtn) {
    importBtn.addEventListener("click", () => {
      startMockImport(state);
    });
  }
}

function startMockImport(state: AppState): void {
  // Generate a unique hash each time
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
