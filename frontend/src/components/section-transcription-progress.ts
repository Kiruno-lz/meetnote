import type { AppState } from "../stores/ui-state.ts";
import { saveState } from "../stores/ui-state.ts";
import { navigate } from "../main.ts";
import type { TranscriptSegment, SpeakerProfile, SpeakerSegment } from "../types/audio-project.ts";
import { mergeTranscriptWithDiarization } from "../utils/speaker.ts";
import { MODEL_MATRIX } from "../types/model-manifest.ts";

const STAGES: { id: string; label: string }[] = [
  { id: "queued", label: "排队中..." },
  { id: "loading-model", label: "加载 ASR 模型..." },
  { id: "transcribing", label: "转录音频..." },
  { id: "asr-finalizing", label: "整理转录结果..." },
  { id: "diarization-loading", label: "加载发言人模型..." },
  { id: "diarization-segmenting", label: "分析发言片段..." },
  { id: "diarization-clustering", label: "聚类发言人..." },
  { id: "diarization-finalizing", label: "整理发言人结果..." },
  { id: "merging", label: "合并结果..." },
];

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function showToast(options: {
  type: "info" | "success" | "warn" | "error";
  message: string;
  hint?: string;
  durationMs?: number;
}): void {
  // Remove existing toast
  document.querySelectorAll(".toast").forEach((el) => el.remove());

  const toast = document.createElement("div");
  toast.className = `toast toast-${options.type}`;
  const hintHtml = options.hint
    ? `<span class="toast-hint">${options.hint}</span>`
    : "";
  toast.innerHTML =
    `<button class="toast-dismiss" aria-label="关闭">&times;</button>` +
    options.message.replace(/</g, "&lt;") +
    hintHtml;

  document.body.appendChild(toast);

  const dismiss = toast.querySelector<HTMLButtonElement>(".toast-dismiss");
  dismiss?.addEventListener("click", () => toast.remove());

  const duration = options.durationMs ?? 8000;
  if (duration > 0) {
    setTimeout(() => {
      if (toast.isConnected) toast.remove();
    }, duration);
  }
}

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
  const modelEntry = MODEL_MATRIX.find((m) => m.id === rawModelId);
  const backend = modelEntry?.backend ?? "whisper";

  // Whisper passes the bare size name (e.g. "large-v3"); library-managed backends
  // pass their HF/ModelScope repo id (model.source) so the Python CLI loads the right model.
  let modelId: string;
  if (backend === "whisper") {
    modelId = rawModelId.startsWith("whisper-") ? rawModelId.slice("whisper-".length) : rawModelId;
  } else {
    modelId = modelEntry?.source ?? rawModelId;
  }

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
          state.pipelineStage = "asr-finalizing";
          saveState(state);
        }
      }
    );

    // Invoke the Rust ASR command
    const asrOutputRaw = await invoke<string>("run_asr", {
      audioPath,
      modelId,
      backend,
      language: state.transcriptionLanguage ?? "zh",
      device: state.transcriptionDevice ?? "auto",
    });

    unlisten();

    // Parse ASR result
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

    // --- Speaker Diarization ---
    let realSpeakerSegs: SpeakerSegment[];
    let realSpeakerProfiles: SpeakerProfile[];

    if (state.hfToken) {
      // Advance UI to diarization stage
      updateStageUI(root, 4);
      state.pipelineStage = "diarization-loading";
      saveState(state);

      const unlistenDiarization = await listen<{ stage: string; message: string }>(
        "diarization-progress",
        (event) => {
          const stage = event.payload.stage;
          if (stage === "loading-model") {
            updateStageUI(root, 4);
            state.pipelineStage = "diarization-loading";
            saveState(state);
          } else if (stage === "segmenting") {
            updateStageUI(root, 5);
            state.pipelineStage = "diarization-segmenting";
            saveState(state);
          } else if (stage === "clustering") {
            updateStageUI(root, 6);
            state.pipelineStage = "diarization-clustering";
            saveState(state);
          } else if (stage === "finalizing" || stage === "done") {
            updateStageUI(root, 7);
            state.pipelineStage = "diarization-finalizing";
            saveState(state);
          }
        }
      );

      try {
        const diarizationResultRaw = await invoke<string>("run_diarization", {
          audioPath,
          hfToken: state.hfToken,
          device: state.transcriptionDevice ?? "auto",
        });

        unlistenDiarization();

        // Advance to merging stage
        updateStageUI(root, 8);
        state.pipelineStage = "merging";
        saveState(state);

        const diarizationOutput = JSON.parse(diarizationResultRaw) as {
          speakers: Array<{ id: string; label: string }>;
          segments: Array<{
            id: string;
            speakerId: string;
            startMs: number;
            endMs: number;
            confidence: number;
            overlap: boolean;
          }>;
        };

        // Merge ASR transcript with diarization segments
        const merged = mergeTranscriptWithDiarization(
          realSegments,
          diarizationOutput.segments.map((s) => ({
            id: s.id,
            speakerId: s.speakerId,
            startMs: s.startMs,
            endMs: s.endMs,
            confidence: s.confidence,
          })),
          { minOverlapMs: 100, minOverlapRatio: 0.1 }
        );

        // Build SpeakerProfile[] from diarization speakers, ensuring all referenced IDs exist
        const speakerIdSet = new Set(merged.map((s) => s.speakerId));
        const profileMap = new Map(diarizationOutput.speakers.map((sp) => [sp.id, sp]));
        realSpeakerProfiles = [...speakerIdSet].map((sid) => {
          const profile = profileMap.get(sid);
          return profile
            ? { id: profile.id, label: profile.label }
            : { id: sid, label: sid };
        });

        // Convert merged result to SpeakerSegment[]
        realSpeakerSegs = merged.map((s) => ({
          id: `ss-${s.id}`,
          speakerId: s.speakerId,
          startMs: s.startMs,
          endMs: s.endMs,
          ...(s.confidence !== undefined ? { confidence: s.confidence } : {}),
        }));
      } catch (diarErr) {
        unlistenDiarization();
        // Diarization failed -- fall back to speaker-0 for all segments
        const diarErrMsg = diarErr instanceof Error ? diarErr.message : String(diarErr);
        console.warn("Diarization failed, falling back to single speaker:", diarErrMsg);

        realSpeakerSegs = realSegments.map((s) => ({
          id: `ss-${s.id}`,
          speakerId: "speaker-0",
          startMs: s.startMs,
          endMs: s.endMs,
        }));
        realSpeakerProfiles = [
          { id: "speaker-0", label: "发言人 1" },
        ];

        showToast({
          type: "warn",
          message: `发言人区分失败: ${diarErrMsg}`,
          hint: "所有段落已标注为同一发言人。请检查 HF Token 和网络连接。",
          durationMs: 0,
        });
      }
    } else {
      // No HF token -- all segments attributed to a single placeholder speaker
      realSpeakerSegs = realSegments.map((s) => ({
        id: `ss-${s.id}`,
        speakerId: "speaker-0",
        startMs: s.startMs,
        endMs: s.endMs,
      }));
      realSpeakerProfiles = [
        { id: "speaker-0", label: "发言人 1" },
      ];

      showToast({
        type: "info",
        message: "发言人区分已跳过（未配置 HuggingFace Token）",
        hint: "在设置页面配置 HF Token 后，重新导入音频即可自动识别发言人。",
        durationMs: 0,
      });
    }

    // Update meeting record with ASR + diarization results
    const meeting = state.meetings.find((m) => m.id === state.activeMeetingId);
    if (meeting) {
      const project = JSON.parse(meeting.projectSnapshot) as import("../types/audio-project.ts").MeetingProject;

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

    showToast({
      type: "success",
      message: `发言人区分完成，识别到 ${realSpeakerProfiles.length} 位发言人`,
      hint: "可在「修正 Speaker」中调整发言人标注",
      durationMs: 6000,
    });

    updateStageUI(root, STAGES.length); // show all complete
    // Navigate after a brief delay to let the user see the completed state
    // Toast notifications persist independently on document.body
    setTimeout(() => navigate("recording-detail"), 800);
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

    // Append error toast and back button to the existing section
    showToast({
      type: "error",
      message: `转录失败: ${errMsg}`,
      hint: "需要在模型管理中下载对应 ASR 模型，或运行 cd python && uv sync",
      durationMs: 0,
    });

    const section = root.querySelector<HTMLElement>(".section");
    if (section) {
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
