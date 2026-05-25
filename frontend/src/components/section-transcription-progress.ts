import type { AppState } from "../stores/ui-state.ts";
import { saveState } from "../stores/ui-state.ts";
import { navigate } from "../main.ts";
import type { TranscriptSegment, SpeakerProfile, SpeakerSegment } from "../types/audio-project.ts";

const STAGES: { id: string; label: string }[] = [
  { id: "queued", label: "排队中..." },
  { id: "loading-model", label: "加载模型..." },
  { id: "transcribing", label: "转录音频..." },
  { id: "finalizing", label: "整理结果..." }
];

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let asrActive = false;

export function renderTranscriptionProgress(root: HTMLElement, state: AppState): void {
  const currentStageIndex = STAGES.findIndex((s) => s.id === state.pipelineStage);
  const displayIndex = currentStageIndex >= 0 ? currentStageIndex : 0;
  const progressPct = Math.round(((displayIndex + 1) / STAGES.length) * 100);

  root.innerHTML = `
    <div class="section" style="max-width:520px;margin:0 auto;padding-top:60px;">
      <h2>正在处理音频</h2>
      <div class="progress-bar-container" style="margin:24px 0;">
        <div class="progress-bar" style="width:${progressPct}%"></div>
      </div>
      <div id="stage-label" style="margin-bottom:24px;color:#aaa;">
        ${STAGES[displayIndex]?.label ?? "处理中..."}
      </div>
      <div id="stage-steps">
        ${STAGES.map((s, i) => `
          <div class="stage-step${i <= displayIndex ? " stage-active" : ""}">
            <span class="stage-dot${i < displayIndex ? " done" : i === displayIndex ? " current" : ""}"></span>
            ${s.label}
          </div>
        `).join("")}
      </div>
    </div>
  `;

  // Route to real ASR or mock animation
  if (state.useRealAsr === true && state.pendingAudioPath !== undefined && isTauri) {
    if (!asrActive) {
      void runRealAsr(root, state);
    }
  } else {
    advanceStages(root, state, displayIndex);
  }
}

async function runRealAsr(root: HTMLElement, state: AppState): Promise<void> {
  asrActive = true;
  const audioPath = state.pendingAudioPath!;

  const rawModelId = state.activeModelId ?? "whisper-large-v3";
  const modelId = rawModelId.startsWith("whisper-") ? rawModelId.slice("whisper-".length) : rawModelId;

  // Advance UI to loading-model
  updateStageUI(root, 1);
  state.pipelineStage = "loading-model";
  saveState(state);

  try {
    const { invoke } = await import("@tauri-apps/api/core");
    const { listen } = await import("@tauri-apps/api/event");

    // Listen for progress events emitted by the Rust backend
    const unlisten = await listen<{ stage: string; message: string }>(
      "asr-progress",
      (event) => {
        const stage = event.payload.stage;
        if (stage === "transcribing") {
          updateStageUI(root, 2);
          state.pipelineStage = "transcribing";
          saveState(state);
        } else if (stage === "finalizing" || stage === "done") {
          updateStageUI(root, 3);
          state.pipelineStage = "finalizing";
          saveState(state);
        }
      }
    );

    // Invoke the Rust ASR command
    const asrOutputRaw = await invoke<string>("run_asr", {
      audioPath,
      modelId,
      language: state.transcriptionLanguage ?? "zh",
      device: state.transcriptionDevice ?? "auto",
    });

    unlisten();

    // Parse result
    const asrOutput = JSON.parse(asrOutputRaw) as {
      segments: Array<{
        id: string;
        startMs: number;
        endMs: number;
        text: string;
        language: string;
        confidence: number;
      }>;
      language: string;
      model: string;
    };

    // Find the active meeting record to update
    const meeting = state.meetings.find((m) => m.id === state.activeMeetingId);
    if (meeting) {
      const project = JSON.parse(meeting.projectSnapshot) as import("../types/audio-project.ts").MeetingProject;

      // Build transcript segments from real ASR output
      const realSegments: TranscriptSegment[] = asrOutput.segments.map((s) => ({
        id: s.id,
        startMs: s.startMs,
        endMs: s.endMs,
        text: s.text,
        language: (
          s.language === "zh" || s.language === "en"
            ? s.language
            : "unknown"
        ) as "zh" | "en" | "mixed" | "unknown",
      }));

      // All segments attributed to a single placeholder speaker (diarization is separate)
      const realSpeakerSegs: SpeakerSegment[] = realSegments.map((s) => ({
        id: `ss-${s.id}`,
        speakerId: "speaker-0",
        startMs: s.startMs,
        endMs: s.endMs,
      }));

      const realSpeakerProfiles: SpeakerProfile[] = [
        { id: "speaker-0", label: "发言人 1" },
      ];

      const updatedProject = {
        ...project,
        transcript: realSegments,
        originalTranscript: realSegments,
        speakerSegments: realSpeakerSegs,
        speakers: realSpeakerProfiles,
        corrections: [],
        deverbalizedSegments: [],
      };

      const idx = state.meetings.findIndex((m) => m.id === meeting.id);
      if (idx >= 0) {
        state.meetings[idx] = {
          ...meeting,
          projectSnapshot: JSON.stringify(updatedProject),
          speakerProfiles: realSpeakerProfiles,
        };
      }
    }

    // Clear ASR session flags using delete (exactOptionalPropertyTypes)
    delete state.pendingAudioPath;
    delete state.useRealAsr;
    state.pipelineRunning = false;
    state.pipelineStage = "";
    saveState(state);

    updateStageUI(root, STAGES.length); // show all complete
    setTimeout(() => navigate("recording-detail"), 300);
  } catch (err) {
    const errMsg = err instanceof Error ? err.message : String(err);

    delete state.pendingAudioPath;
    delete state.useRealAsr;
    state.pipelineRunning = false;

    // Remove the failed pending meeting record so no mock placeholder remains in the list
    if (state.activeMeetingId) {
      state.meetings = state.meetings.filter(m => m.id !== state.activeMeetingId);
      state.activeMeetingId = null;
    }
    saveState(state);

    // Append error message to the existing section
    const section = root.querySelector<HTMLElement>(".section");
    if (section) {
      const errorDiv = document.createElement("div");
      errorDiv.style.cssText =
        "margin-top:24px; padding:12px 16px; background:rgba(198,40,40,0.1); border:1px solid rgba(198,40,40,0.3); border-radius:8px; color:#ef5350; font-size:13px;";
      errorDiv.innerHTML = `
        <strong>转录失败：</strong>${errMsg.replace(/</g, "&lt;")}
        <br><span style="color:#888; margin-top:4px; display:block;">
          提示：需要在模型管理中下载 Whisper 模型，或运行 <code>cd python &amp;&amp; uv sync</code>
        </span>
      `;
      section.appendChild(errorDiv);

      const backBtn = document.createElement("button");
      backBtn.className = "btn btn-secondary";
      backBtn.style.marginTop = "16px";
      backBtn.textContent = "返回列表";
      backBtn.addEventListener("click", () => navigate("project-list"));
      section.appendChild(backBtn);
    }
  } finally {
    asrActive = false;
  }
}

function updateStageUI(root: HTMLElement, completedCount: number): void {
  const clampedIdx = Math.min(completedCount, STAGES.length - 1);
  const progressPct = Math.round(((clampedIdx + 1) / STAGES.length) * 100);

  const bar = root.querySelector<HTMLElement>(".progress-bar");
  if (bar) bar.style.width = `${progressPct}%`;

  const stage = STAGES[clampedIdx];
  const label = root.querySelector<HTMLElement>("#stage-label");
  if (label && stage) label.textContent = stage.label;

  const steps = root.querySelectorAll<HTMLElement>(".stage-step");
  steps.forEach((step, i) => {
    if (i <= clampedIdx) step.classList.add("stage-active");
    else step.classList.remove("stage-active");
  });

  const dots = root.querySelectorAll<HTMLElement>(".stage-dot");
  dots.forEach((dot, i) => {
    dot.className = "stage-dot";
    if (i < clampedIdx) dot.classList.add("done");
    else if (i === clampedIdx) dot.classList.add("current");
  });
}

function advanceStages(root: HTMLElement, state: AppState, fromIndex: number): void {
  let idx = fromIndex;

  function advance(): void {
    idx++;
    if (idx >= STAGES.length) {
      // Done
      state.pipelineRunning = false;
      state.pipelineStage = "";
      saveState(state);
      navigate("recording-detail");
      return;
    }

    const stage = STAGES[idx];
    if (!stage) return;

    state.pipelineStage = stage.id;
    saveState(state);

    // Update UI
    const progressPct = Math.round(((idx + 1) / STAGES.length) * 100);
    const bar = root.querySelector<HTMLElement>(".progress-bar");
    if (bar) bar.style.width = `${progressPct}%`;

    const label = root.querySelector<HTMLElement>("#stage-label");
    if (label) label.textContent = stage.label;

    const steps = root.querySelectorAll<HTMLElement>(".stage-step");
    steps.forEach((step, i) => {
      if (i <= idx) step.classList.add("stage-active");
      else step.classList.remove("stage-active");
    });

    const dots = root.querySelectorAll<HTMLElement>(".stage-dot");
    dots.forEach((dot, i) => {
      dot.className = "stage-dot";
      if (i < idx) dot.classList.add("done");
      else if (i === idx) dot.classList.add("current");
    });

    setTimeout(advance, 600);
  }

  setTimeout(advance, 600);
}
