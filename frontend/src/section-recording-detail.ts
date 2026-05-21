import type { AppState } from "./ui-state.ts";
import { saveState } from "./ui-state.ts";
import type { MeetingProject } from "./audio-project.ts";
import { navigate } from "./main.ts";

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

function getSpeakerColor(speakerId: string, profiles: { id: string; label: string }[]): string {
  const idx = profiles.findIndex((p) => p.id === speakerId);
  return SPEAKER_COLORS[((idx >= 0 ? idx : 0) % SPEAKER_COLORS.length)] ?? SPEAKER_COLORS[0] ?? "#4a9eff";
}

export function renderRecordingDetail(root: HTMLElement, state: AppState): void {
  const meeting = state.meetings.find((m) => m.id === state.activeMeetingId);

  if (!meeting) {
    root.innerHTML = `
      <div class="section">
        <div class="empty-state">未找到会议记录</div>
        <button class="btn btn-secondary" id="btn-back">返回首页</button>
      </div>
    `;
    root.querySelector("#btn-back")?.addEventListener("click", () => navigate("project-list"));
    return;
  }

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
        <h2>${escapeHtml(meeting.title)}</h2>
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

  root.querySelector("#btn-speaker-editor")?.addEventListener("click", () => {
    saveState(state);
    navigate("speaker-editor");
  });

  root.querySelector("#btn-deverb")?.addEventListener("click", () => {
    saveState(state);
    navigate("deverbalization-preview");
  });
}
