import type { AppState } from "./ui-state.ts";
import { saveState } from "./ui-state.ts";
import { navigate } from "./main.ts";

const STAGES: { id: string; label: string }[] = [
  { id: "queued", label: "排队中..." },
  { id: "loading-model", label: "加载模型..." },
  { id: "transcribing", label: "转录音频..." },
  { id: "finalizing", label: "整理结果..." }
];

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

  // Kick off stage transitions
  advanceStages(root, state, displayIndex);
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
