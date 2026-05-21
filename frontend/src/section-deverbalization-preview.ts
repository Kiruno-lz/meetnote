import type { AppState, MeetingRecord } from "./ui-state.ts";
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

export function renderDeverbalizationPreview(root: HTMLElement, state: AppState): void {
  const meeting = state.meetings.find((m) => m.id === state.activeMeetingId);

  if (!meeting) {
    root.innerHTML = `
      <div class="section">
        <div class="empty-state">未找到会议记录</div>
        <button class="btn btn-secondary" id="btn-back">返回</button>
      </div>
    `;
    root.querySelector("#btn-back")?.addEventListener("click", () => navigate("recording-detail"));
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
    .map((b, idx) => {
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

  // Display markdown content with simple formatting
  const markdownLines = meeting.markdownContent
    .split("\n")
    .map((line) => `<div class="md-line">${escapeHtml(line) || "&nbsp;"}</div>`)
    .join("");

  root.innerHTML = `
    <div class="section">
      <div class="section-header">
        <button class="btn btn-secondary btn-sm" id="btn-back">← 返回</button>
        <h2>去口语化预览</h2>
      </div>
      <div class="deverb-split">
        <div class="deverb-left">
          <h3 style="margin-bottom:12px;color:#aaa;font-size:14px;font-weight:600;text-transform:uppercase;letter-spacing:.05em;">原始转录</h3>
          ${leftHtml || '<div class="empty-state">无内容</div>'}
        </div>
        <div class="deverb-divider"></div>
        <div class="deverb-right">
          <h3 style="margin-bottom:12px;color:#aaa;font-size:14px;font-weight:600;text-transform:uppercase;letter-spacing:.05em;">去口语化 Markdown</h3>
          <div class="markdown-preview">${markdownLines}</div>
        </div>
      </div>
      <div class="action-bar">
        <button class="btn btn-primary" id="btn-save-markdown">保存到 Markdown 库</button>
      </div>
    </div>
  `;

  root.querySelector("#btn-back")?.addEventListener("click", () => navigate("recording-detail"));

  root.querySelector("#btn-save-markdown")?.addEventListener("click", () => {
    // Mark markdown as saved in state (already saved, just navigate to library)
    saveState(state);
    navigate("markdown-library");
  });
}
