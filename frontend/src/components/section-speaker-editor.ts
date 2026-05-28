import type { AppState } from "../stores/ui-state.ts";
import { saveState } from "../stores/ui-state.ts";
import type { SpeakerProfile } from "../types/audio-project.ts";
import { renameSpeaker, mergeSpeakers } from "../utils/speaker.ts";
import { navigate } from "../main.ts";

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

const SPEAKER_COLORS = ["#4a9eff", "#ff7043", "#66bb6a", "#ab47bc"];

export function renderSpeakerEditor(root: HTMLElement, state: AppState): void {
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

  // Work on a mutable copy of speaker profiles
  let profiles: SpeakerProfile[] = [...meeting.speakerProfiles];

  function buildSpeakerRows(): string {
    return profiles
      .map((p, idx) => {
        const color = SPEAKER_COLORS[idx % SPEAKER_COLORS.length] ?? SPEAKER_COLORS[0] ?? "#4a9eff";
        const otherOptions = profiles
          .filter((other) => other.id !== p.id)
          .map((other) => `<option value="${escapeHtml(other.id)}">${escapeHtml(other.label)}</option>`)
          .join("");

        return `
          <div class="card speaker-row" data-id="${escapeHtml(p.id)}">
            <div class="speaker-row-header">
              <span class="speaker-tag" style="background:${escapeHtml(color)}20;color:${escapeHtml(color)};border:1px solid ${escapeHtml(color)}40;">
                ${escapeHtml(p.label)}
              </span>
              <span style="color:#888;font-size:12px;">${escapeHtml(p.id)}</span>
            </div>
            <div class="speaker-row-actions">
              <input
                class="speaker-rename-input"
                type="text"
                value="${escapeHtml(p.label)}"
                placeholder="新名称"
                data-id="${escapeHtml(p.id)}"
                style="flex:1;"
              />
              <button class="btn btn-secondary btn-sm btn-rename" data-id="${escapeHtml(p.id)}">改名</button>
              ${
                profiles.length > 1
                  ? `<select class="speaker-merge-select" data-id="${escapeHtml(p.id)}">
                  <option value="">合并到...</option>
                  ${otherOptions}
                </select>
                <button class="btn btn-secondary btn-sm btn-merge" data-id="${escapeHtml(p.id)}">合并</button>`
                  : ""
              }
            </div>
          </div>`;
      })
      .join("");
  }

  function rerender(): void {
    const list = root.querySelector<HTMLElement>("#speaker-list");
    if (list) {
      list.innerHTML = buildSpeakerRows();
      attachRowListeners();
    }
  }

  function attachRowListeners(): void {
    // Rename buttons
    root.querySelectorAll<HTMLButtonElement>(".btn-rename").forEach((btn) => {
      btn.addEventListener("click", () => {
        const id = btn.dataset["id"] ?? "";
        const input = root.querySelector<HTMLInputElement>(`.speaker-rename-input[data-id="${CSS.escape(id)}"]`);
        const newLabel = input?.value.trim() ?? "";
        if (!newLabel) return;
        try {
          profiles = renameSpeaker(profiles, id, newLabel);
          rerender();
        } catch (err) {
          alert(err instanceof Error ? err.message : "改名失败");
        }
      });
    });

    // Merge buttons
    root.querySelectorAll<HTMLButtonElement>(".btn-merge").forEach((btn) => {
      btn.addEventListener("click", () => {
        const sourceId = btn.dataset["id"] ?? "";
        const select = root.querySelector<HTMLSelectElement>(`.speaker-merge-select[data-id="${CSS.escape(sourceId)}"]`);
        const targetId = select?.value ?? "";
        if (!targetId) {
          alert("请选择合并目标");
          return;
        }

        // Remove source from profiles, keep target
        profiles = profiles.filter((p) => p.id !== sourceId);

        // Sync updated speakerSegments and corrections into projectSnapshot
        const meetingIdx = state.meetings.findIndex((m) => m.id === state.activeMeetingId);
        if (meetingIdx >= 0) {
          const meeting = state.meetings[meetingIdx];
          if (meeting) {
            const project = JSON.parse(meeting.projectSnapshot) as import("../types/audio-project.ts").MeetingProject;
            const { segments: mergedSegments, corrections: mergedCorrections } = mergeSpeakers(
              project.speakerSegments,
              project.corrections,
              sourceId,
              targetId,
              new Date().toISOString()
            );
            const updatedProject = {
              ...project,
              speakerSegments: mergedSegments,
              corrections: mergedCorrections,
              speakers: profiles,
            };
            state.meetings[meetingIdx] = {
              ...meeting,
              projectSnapshot: JSON.stringify(updatedProject),
              speakerProfiles: profiles,
            };
            saveState(state);
          }
        }

        rerender();
      });
    });
  }

  root.innerHTML = `
    <div class="section">
      <div class="section-header">
        <button class="btn btn-secondary btn-sm" id="btn-back">← 返回</button>
        <h2>修正 Speaker</h2>
      </div>
      <div id="speaker-list">
        ${buildSpeakerRows()}
      </div>
      <div class="action-bar">
        <button class="btn btn-primary" id="btn-confirm">确认修正</button>
      </div>
    </div>
  `;

  attachRowListeners();

  root.querySelector("#btn-back")?.addEventListener("click", () => navigate("recording-detail"));

  root.querySelector("#btn-confirm")?.addEventListener("click", () => {
    const idx = state.meetings.findIndex((m) => m.id === state.activeMeetingId);
    if (idx >= 0) {
      const existing = state.meetings[idx];
      if (existing) {
        const project = JSON.parse(existing.projectSnapshot) as import("../types/audio-project.ts").MeetingProject;
        const updatedProject = {
          ...project,
          speakers: profiles,
        };
        state.meetings[idx] = {
          ...existing,
          speakerProfiles: profiles,
          projectSnapshot: JSON.stringify(updatedProject),
        };
      }
    }
    saveState(state);
    navigate("recording-detail");
  });
}
