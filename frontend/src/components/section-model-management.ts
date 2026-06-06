import type { AppState } from "../stores/ui-state.ts";
import { saveState } from "../stores/ui-state.ts";
import {
  getWhisperModelList,
  formatFileSize,
  formatSpeed,
  formatEta,
  type ActiveDownload,
  type ModelFileInfo,
  type DownloadStatus,
} from "../utils/model-manager.ts";

// Detect Tauri runtime environment
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// Module-level unlisten handle to avoid stacking listeners across re-renders
let _unlisten: (() => void) | null = null;

// Module-level root reference so the progress listener always targets the current DOM
let _currentRoot: HTMLElement | null = null;

// Active downloads map (module-level so it survives re-renders within the session)
const activeDownloads = new Map<string, ActiveDownload>();

// Installed model paths map (modelId → file path on disk)
const installedModelPaths = new Map<string, string>();

// Download errors map (modelId → error message)
const downloadErrors = new Map<string, string>();

// Tracks models installed during this session (survives re-renders)
const sessionInstalledModelIds = new Set<string>();

// Tracks the active model ID within this session
let sessionActiveModelId: string | null = null;

// ── Mock data for non-Tauri mode ─────────────────────────────────────────────

const MOCK_INSTALLED_MODELS: ModelFileInfo[] = [
  {
    modelId: "whisper-large-v3",
    path: "~/.meetnote/models/whisper-large-v3.bin",
    sizeBytes: 3_100_000_000,
    isActive: true,
  },
];

// ── DOM helpers ───────────────────────────────────────────────────────────────

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// ── Action buttons renderer ───────────────────────────────────────────────────

interface ModelRenderContext {
  modelId: string;
  isInstalled: boolean;
  isActive: boolean;
  download: ActiveDownload | undefined;
}

function renderModelActions(ctx: ModelRenderContext): string {
  const { modelId, isInstalled, isActive, download } = ctx;

  if (download) {
    const status: DownloadStatus = download.status;
    if (status === "downloading" || status === "queued") {
      return `
        <div style="display:flex;gap:8px;">
          <button class="btn btn-sm btn-secondary btn-pause" data-model-id="${modelId}">暂停</button>
          <button class="btn btn-sm btn-secondary btn-cancel" data-model-id="${modelId}">取消</button>
        </div>`;
    }
    if (status === "paused") {
      return `
        <div style="display:flex;gap:8px;">
          <button class="btn btn-sm btn-primary btn-resume" data-model-id="${modelId}">继续</button>
          <button class="btn btn-sm btn-secondary btn-cancel" data-model-id="${modelId}">取消</button>
        </div>`;
    }
  }

  if (isInstalled) {
    if (isActive) {
      return `<span style="color:#66bb6a;font-size:13px;font-weight:600;">✓ 当前使用</span>`;
    }
    return `
      <div style="display:flex;gap:8px;">
        <button class="btn btn-sm btn-secondary btn-use-model" data-model-id="${modelId}">切换使用</button>
        <button class="btn btn-sm btn-danger btn-delete-model" data-model-id="${modelId}">删除</button>
      </div>`;
  }

  return `<button class="btn btn-sm btn-primary btn-download" data-model-id="${modelId}">下载</button>`;
}

// ── Progress bar renderer ─────────────────────────────────────────────────────

function renderProgressBar(modelId: string): string {
  return `
    <div id="model-progress-${modelId}" style="display:none;margin-top:12px;">
      <div class="progress-bar-container" style="height:6px;background:#2e2e2e;border-radius:3px;overflow:hidden;">
        <div class="progress-bar" id="pbar-${modelId}" style="width:0%;height:100%;background:#4a9eff;border-radius:3px;transition:width .3s;"></div>
      </div>
      <div style="display:flex;justify-content:space-between;margin-top:4px;font-size:12px;color:#888;">
        <span id="pbar-detail-${modelId}">0 B / 0 B</span>
        <span id="pbar-speed-${modelId}">— B/s</span>
        <span id="pbar-eta-${modelId}">—</span>
      </div>
    </div>`;
}

// ── Download progress UI update ───────────────────────────────────────────────

function updateDownloadUI(
  root: HTMLElement,
  modelId: string,
  downloadedBytes: number,
  totalBytes: number,
  speedBps: number,
  status: DownloadStatus,
  state: AppState
): void {
  const progressDiv = root.querySelector<HTMLElement>(`#${CSS.escape(`model-progress-${modelId}`)}`);
  const pbar = root.querySelector<HTMLElement>(`#${CSS.escape(`pbar-${modelId}`)}`);
  const pbarDetail = root.querySelector<HTMLElement>(`#${CSS.escape(`pbar-detail-${modelId}`)}`);
  const pbarSpeed = root.querySelector<HTMLElement>(`#${CSS.escape(`pbar-speed-${modelId}`)}`);
  const pbarEta = root.querySelector<HTMLElement>(`#${CSS.escape(`pbar-eta-${modelId}`)}`);
  const actionsDiv = root.querySelector<HTMLElement>(`#${CSS.escape(`model-actions-${modelId}`)}`);

  // Update progress bar visibility
  if (progressDiv) {
    progressDiv.style.display =
      status === "downloading" || status === "paused" || status === "queued" ? "block" : "none";
  }

  if (pbar && totalBytes > 0) {
    const pct = Math.min(100, (downloadedBytes / totalBytes) * 100);
    pbar.style.width = `${pct.toFixed(1)}%`;
  }

  if (pbarDetail) {
    pbarDetail.textContent = `${formatFileSize(downloadedBytes)} / ${formatFileSize(totalBytes)}`;
  }

  if (pbarSpeed) {
    pbarSpeed.textContent = formatSpeed(speedBps);
  }

  if (pbarEta) {
    const remaining = speedBps > 0 ? (totalBytes - downloadedBytes) / speedBps : Infinity;
    pbarEta.textContent = formatEta(remaining);
  }

  // Re-render action buttons
  if (actionsDiv) {
    const dl = activeDownloads.get(modelId);
    actionsDiv.innerHTML = renderModelActions({
      modelId,
      isInstalled: false,
      isActive: false,
      download: dl,
    });
    attachActionListeners(root, modelId, state);
  }
}

// ── Download error UI update ──────────────────────────────────────────────────

function updateDownloadError(modelId: string, errMsg: string): void {
  const progressDiv = document.getElementById(`model-progress-${modelId}`);
  if (!progressDiv) return;
  progressDiv.style.display = "block";
  progressDiv.innerHTML = `
    <div style="color:#ef5350; font-size:12px; padding:6px 0; display:flex; align-items:center; gap:6px;">
      <span>⚠</span>
      <span>${escapeHtml(errMsg)}</span>
      <button class="btn btn-sm btn-secondary" style="margin-left:auto;"
              onclick="this.parentElement.parentElement.style.display='none'">关闭</button>
    </div>
  `;
}

// ── Tauri invoke/listen (dynamic import to avoid crashing in browser) ─────────

interface DownloadProgressEvent {
  modelId: string;
  downloadedBytes: number;
  totalBytes: number;
  speedBps: number;
  status: string;
}

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

async function tauriListen(
  event: string,
  handler: (payload: DownloadProgressEvent) => void
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<DownloadProgressEvent>(event, (e) => handler(e.payload));
}

// ── Action button listeners ───────────────────────────────────────────────────

function attachActionListeners(root: HTMLElement, modelId: string, state: AppState): void {
  root.querySelector<HTMLElement>(`.btn-download[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void startDownload(root, modelId, state));

  root.querySelector<HTMLElement>(`.btn-pause[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void pauseDownload(modelId));

  root.querySelector<HTMLElement>(`.btn-resume[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void resumeDownload(root, modelId, state));

  root.querySelector<HTMLElement>(`.btn-cancel[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void cancelDownload(root, modelId, state));

  root.querySelector<HTMLElement>(`.btn-use-model[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void switchModel(root, modelId, state));

  root.querySelector<HTMLElement>(`.btn-delete-model[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void deleteModel(root, modelId, state));
}

async function startDownload(root: HTMLElement, modelId: string, state: AppState): Promise<void> {
  if (!isTauri) return;
  const whisperModels = getWhisperModelList();
  const model = whisperModels.find((m) => m.id === modelId);
  if (!model) return;

  const isLibraryManaged = model.downloadMethod === "library-managed";
  // Library-managed models live in the HF/ModelScope cache; only http models map to a .bin file.
  const destPath = isLibraryManaged ? "" : `${state.modelCachePath}/${modelId}.bin`;

  const dl: ActiveDownload = {
    modelId,
    status: "queued",
    downloadedBytes: 0,
    totalBytes: model.sizeBytes ?? 0,
    speedBps: 0,
    eta: Infinity,
    error: null,
    destPath,
  };
  activeDownloads.set(modelId, dl);

  const actionsDiv = root.querySelector<HTMLElement>(`#${CSS.escape(`model-actions-${modelId}`)}`);
  if (actionsDiv) {
    actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: dl });
    attachActionListeners(root, modelId, state);
  }
  updateDownloadUI(root, modelId, 0, dl.totalBytes, 0, "queued", state);

  try {
    if (isLibraryManaged) {
      // model_downloader.py expects the full repo id (model.source) as --model-id
      await tauriInvoke("start_model_download", {
        modelId,
        url: "",
        destPath: "",
        downloadMethod: "library-managed",
        backend: model.backend,
        modelSource: model.source,
        hfToken: state.hfToken || null,
      });
    } else {
      await tauriInvoke("start_model_download", { modelId, url: model.source, destPath });
    }
  } catch (err) {
    const errMsg = err instanceof Error ? err.message : String(err);
    downloadErrors.set(modelId, errMsg);
    updateDownloadError(modelId, errMsg);
    const existing = activeDownloads.get(modelId);
    if (existing) {
      activeDownloads.set(modelId, { ...existing, status: "failed", error: errMsg });
    } else {
      activeDownloads.delete(modelId);
    }
    // Re-render actions back to downloadable
    if (actionsDiv) {
      actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: undefined });
      attachActionListeners(root, modelId, state);
    }
  }
}

async function pauseDownload(modelId: string): Promise<void> {
  if (!isTauri) return;
  const dl = activeDownloads.get(modelId);
  if (dl) activeDownloads.set(modelId, { ...dl, status: "paused" });
  try {
    await tauriInvoke("pause_model_download", { modelId });
  } catch (err) {
    console.error("pause_model_download failed", err);
  }
}

async function resumeDownload(root: HTMLElement, modelId: string, state: AppState): Promise<void> {
  if (!isTauri) return;
  const dl = activeDownloads.get(modelId);
  if (dl) activeDownloads.set(modelId, { ...dl, status: "downloading" });
  try {
    const whisperModels = getWhisperModelList();
    const model = whisperModels.find((m) => m.id === modelId);
    if (!model) return;
    if (model.downloadMethod === "library-managed") {
      await tauriInvoke("start_model_download", {
        modelId,
        url: "",
        destPath: "",
        downloadMethod: "library-managed",
        backend: model.backend,
        modelSource: model.source,
        hfToken: state.hfToken || null,
      });
    } else {
      const destPath = `${state.modelCachePath}/${modelId}.bin`;
      await tauriInvoke("start_model_download", { modelId, url: model.source, destPath });
    }
  } catch (err) {
    console.error("start_model_download (resume) failed", err);
  }
  const actionsDiv = root.querySelector<HTMLElement>(`#${CSS.escape(`model-actions-${modelId}`)}`);
  if (actionsDiv) {
    actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: activeDownloads.get(modelId) });
    attachActionListeners(root, modelId, state);
  }
}

async function cancelDownload(root: HTMLElement, modelId: string, state: AppState): Promise<void> {
  if (!isTauri) return;
  try {
    await tauriInvoke("cancel_model_download", { modelId });
  } catch (err) {
    console.error("cancel_model_download failed", err);
  }
  activeDownloads.delete(modelId);
  const actionsDiv = root.querySelector<HTMLElement>(`#${CSS.escape(`model-actions-${modelId}`)}`);
  const progressDiv = root.querySelector<HTMLElement>(`#${CSS.escape(`model-progress-${modelId}`)}`);
  if (progressDiv) progressDiv.style.display = "none";
  if (actionsDiv) {
    actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: undefined });
    attachActionListeners(root, modelId, state);
  }
}

async function switchModel(root: HTMLElement, modelId: string, state: AppState): Promise<void> {
  if (!isTauri) return;
  try {
    await tauriInvoke("set_active_model_id", { modelId });
    sessionActiveModelId = modelId;
    state.activeModelId = modelId;
    saveState(state);
    // Update all model cards' action buttons to reflect new active state
    const whisperModels = getWhisperModelList();
    for (const m of whisperModels) {
      const actionsDiv = root.querySelector<HTMLElement>(`#model-actions-${CSS.escape(m.id)}`);
      if (actionsDiv) {
        actionsDiv.innerHTML = renderModelActions({
          modelId: m.id,
          isInstalled: sessionInstalledModelIds.has(m.id),
          isActive: m.id === modelId,
          download: activeDownloads.get(m.id),
        });
        attachActionListeners(root, m.id, state);
      }
    }
    // Update header label
    const label = root.querySelector<HTMLElement>("#active-model-label");
    if (label) label.textContent = modelId;
  } catch (err) {
    console.error("set_active_model failed", err);
  }
}

async function deleteModel(root: HTMLElement, modelId: string, state: AppState): Promise<void> {
  if (!isTauri) return;
  try {
    const model = getWhisperModelList().find((m) => m.id === modelId);
    // Library-managed models live in the shared HF/ModelScope cache.
    // Confirm with the user before removing, since other apps may share the cache.
    if (model?.downloadMethod === "library-managed") {
      const confirmed = window.confirm(
        `模型 ${model.source} 存放于共享缓存 (~/.cache/huggingface)，删除可能影响其他使用该模型的应用。\n\n确定要删除吗？`
      );
      if (!confirmed) return;

      await tauriInvoke("remove_library_model", { modelSource: model.source });
      sessionInstalledModelIds.delete(modelId);
      installedModelPaths.delete(modelId);

      // Re-render the card actions as not installed
      const actionsDiv = root.querySelector<HTMLElement>(`#${CSS.escape(`model-actions-${modelId}`)}`);
      if (actionsDiv) {
        actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: undefined });
        attachActionListeners(root, modelId, state);
      }
      return;
    }
    const path = installedModelPaths.get(modelId);
    if (!path) {
      console.error("deleteModel: no path found for", modelId);
      return;
    }
    await tauriInvoke("delete_model_file", { path });
    sessionInstalledModelIds.delete(modelId);
    installedModelPaths.delete(modelId);
    // Re-render the card actions
    const actionsDiv = root.querySelector<HTMLElement>(`#${CSS.escape(`model-actions-${modelId}`)}`);
    if (actionsDiv) {
      actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: undefined });
      attachActionListeners(root, modelId, state);
    }
  } catch (err) {
    console.error("delete_cached_model failed", err);
  }
}

// ── Listener setup ────────────────────────────────────────────────────────────

async function setupProgressListener(state: AppState): Promise<void> {
  if (!isTauri) return;
  // Tear down previous listener if any
  if (_unlisten) {
    _unlisten();
    _unlisten = null;
  }
  _unlisten = await tauriListen("model-download-progress", (payload) => {
    // Use module-level root reference so the listener always targets the current DOM
    const root = _currentRoot;
    if (!root) return;
    // Detect failed status (emitted as "failed" or "failed: <msg>")
    const isFailed = payload.status === "failed" || payload.status.startsWith("failed:");
    if (isFailed) {
      const errMsg = payload.status.startsWith("failed:")
        ? payload.status.slice("failed:".length).trim()
        : "下载失败，请检查网络后重试";
      downloadErrors.set(payload.modelId, errMsg);
      updateDownloadError(payload.modelId, errMsg);
      activeDownloads.delete(payload.modelId);
      const actionsDiv = root.querySelector<HTMLElement>(`#${CSS.escape(`model-actions-${payload.modelId}`)}`);
      if (actionsDiv) {
        actionsDiv.innerHTML = renderModelActions({
          modelId: payload.modelId,
          isInstalled: false,
          isActive: false,
          download: undefined,
        });
        attachActionListeners(root, payload.modelId, state);
      }
      return;
    }

    if (payload.status === "completed") {
      activeDownloads.delete(payload.modelId);
      sessionInstalledModelIds.add(payload.modelId);
      // Library-managed models live in the HF/ModelScope cache; record their repo id
      // as the "path" so delete/active logic can distinguish them from .bin files.
      const model = getWhisperModelList().find((m) => m.id === payload.modelId);
      const destPath =
        model?.downloadMethod === "library-managed"
          ? model.source
          : `${state.modelCachePath}/${payload.modelId}.bin`;
      installedModelPaths.set(payload.modelId, destPath);

      // Hide progress bar
      const progressDiv = root.querySelector<HTMLElement>(`#model-progress-${CSS.escape(payload.modelId)}`);
      if (progressDiv) progressDiv.style.display = "none";

      // Re-render actions as installed (not active yet)
      const actionsDiv = root.querySelector<HTMLElement>(`#model-actions-${CSS.escape(payload.modelId)}`);
      if (actionsDiv) {
        actionsDiv.innerHTML = renderModelActions({
          modelId: payload.modelId,
          isInstalled: true,
          isActive: sessionActiveModelId === payload.modelId,
          download: undefined,
        });
        attachActionListeners(root, payload.modelId, state);
      }
      return;
    }

    // Normalize library-managed download statuses to UI-visible states
    const normalizedStatus: DownloadStatus =
      (payload.status === "starting" || payload.status === "loading")
        ? "downloading"
        : payload.status as DownloadStatus;

    activeDownloads.set(payload.modelId, {
      modelId: payload.modelId,
      status: normalizedStatus,
      downloadedBytes: payload.downloadedBytes,
      totalBytes: payload.totalBytes,
      speedBps: payload.speedBps,
      eta: payload.speedBps > 0 ? (payload.totalBytes - payload.downloadedBytes) / payload.speedBps : Infinity,
      error: null,
      destPath: `${state.modelCachePath}/${payload.modelId}.bin`,
    });
    updateDownloadUI(
      root,
      payload.modelId,
      payload.downloadedBytes,
      payload.totalBytes,
      payload.speedBps,
      normalizedStatus,
      state
    );
  });
}

// ── Main render function ──────────────────────────────────────────────────────

export function renderModelManagement(root: HTMLElement, state: AppState): void {
  // Update the module-level root reference so the progress listener targets the current DOM
  _currentRoot = root;
  void (async () => {
    // Load installed models
    let installedModels: ModelFileInfo[];
    let activeModelId: string | null = null;

    if (isTauri) {
      try {
        installedModels = await tauriInvoke<ModelFileInfo[]>("list_cached_models", {
          cacheDir: state.modelCachePath,
        });
        activeModelId = installedModels.find((m) => m.isActive)?.modelId ?? null;
        for (const m of installedModels) {
          installedModelPaths.set(m.modelId, m.path);
          sessionInstalledModelIds.add(m.modelId);
        }
        sessionActiveModelId = activeModelId;
        if (activeModelId && activeModelId !== state.activeModelId) {
          state.activeModelId = activeModelId;
          saveState(state);
        }
      } catch {
        installedModels = [];
      }
    } else {
      installedModels = MOCK_INSTALLED_MODELS;
      activeModelId = installedModels.find((m) => m.isActive)?.modelId ?? null;
      for (const m of installedModels) {
        sessionInstalledModelIds.add(m.modelId);
      }
      sessionActiveModelId = activeModelId;
    }

    const whisperModels = getWhisperModelList();

    // Library-managed models (Paraformer/Qwen3/MiMo) live in the HF/ModelScope cache,
    // not as .bin files — probe each via the backend before building the card list.
    if (isTauri) {
      await Promise.all(
        whisperModels
          .filter((m) => m.downloadMethod === "library-managed")
          .map(async (m) => {
            try {
              const installed = await tauriInvoke<boolean>("check_library_model_installed", {
                modelSource: m.source,
              });
              if (installed) {
                sessionInstalledModelIds.add(m.id);
                installedModelPaths.set(m.id, m.source);
              }
            } catch {
              // Probe failed — leave model as not-installed
            }
          })
      );
    }

    const installedIds = new Set([...installedModels.map((m) => m.modelId), ...sessionInstalledModelIds]);

    // Build model cards HTML
    const modelCardsHtml = whisperModels
      .map((m) => {
        const isInstalled = installedIds.has(m.id);
        const isActive = m.id === (sessionActiveModelId ?? activeModelId);
        const download = activeDownloads.get(m.id);
        const sizeStr = m.sizeBytes !== null ? formatFileSize(m.sizeBytes) : "大小未知";

        // Engine/backend badge
        const backendColors: Record<string, string> = {
          whisper: "#4a9eff",
          paraformer: "#66bb6a",
          "qwen3-asr": "#ab47bc",
          mimo: "#ffa726",
        };
        const backendColor = backendColors[m.backend] ?? "#888";
        const backendBadge = `<span style="display:inline-block;padding:1px 7px;border-radius:4px;font-size:11px;font-weight:600;background:${backendColor}22;color:${backendColor};">${escapeHtml(m.backend)}</span>`;
        const gpuBadge = m.requiresGpu
          ? `<span style="display:inline-block;padding:1px 7px;border-radius:4px;font-size:11px;font-weight:600;background:#ef535022;color:#ef5350;">需要 GPU</span>`
          : "";
        const depHint = m.libraryDependencies.length
          ? `<div style="font-size:11px;color:#666;margin-top:3px;">依赖：${escapeHtml(m.libraryDependencies.join(", "))}</div>`
          : "";

        return `
          <div class="card" id="model-card-${escapeHtml(m.id)}" style="margin-bottom:12px;">
            <div style="display:flex;justify-content:space-between;align-items:flex-start;">
              <div style="flex:1;min-width:0;margin-right:16px;">
                <div style="display:flex;align-items:center;gap:8px;margin-bottom:2px;flex-wrap:wrap;">
                  <span style="font-weight:600;color:#e0e0e0;">${escapeHtml(m.label)}</span>
                  ${backendBadge}
                  ${gpuBadge}
                </div>
                <div style="font-size:12px;color:#888;">${escapeHtml(sizeStr)} · ${escapeHtml(m.description)}</div>
                ${depHint}
              </div>
              <div id="model-actions-${escapeHtml(m.id)}" style="flex-shrink:0;">
                ${renderModelActions({ modelId: m.id, isInstalled, isActive, download })}
              </div>
            </div>
            ${renderProgressBar(m.id)}
          </div>`;
      })
      .join("");

    const mockBanner = !isTauri
      ? `<div style="margin-bottom:16px;padding:8px 12px;background:#2a2000;border:1px solid #554400;border-radius:6px;font-size:12px;color:#aa9900;">
           ⚠ 仅在 Tauri 桌面模式下可用，当前显示示例数据
         </div>`
      : "";

    root.innerHTML = `
      <div class="section">
        <div class="section-header">
          <h2>模型管理</h2>
        </div>
        ${mockBanner}
        <div style="margin-bottom:20px;padding:12px 16px;background:#1e1e1e;border-radius:8px;border:1px solid #2e2e2e;">
          <span style="color:#888;font-size:13px;">当前 ASR 模型：</span>
          <span id="active-model-label" style="color:#4a9eff;font-weight:600;">${escapeHtml(activeModelId ?? "未设置")}</span>
        </div>
        <div style="margin-bottom:16px; padding:10px 14px; background:#1e1e1e; border-radius:8px; border:1px solid #2e2e2e; display:flex; align-items:center; gap:12px;">
          <span style="color:#888; font-size:13px; flex-shrink:0;">模型存储目录：</span>
          <span id="model-path-display" style="color:#ccc; font-size:13px; font-family:monospace; flex:1; word-break:break-all;">${escapeHtml(state.modelCachePath)}</span>
          <button class="btn btn-sm btn-secondary" id="btn-change-path" style="flex-shrink:0;">修改</button>
        </div>
        <h3 style="font-size:15px;color:#e0e0e0;margin-bottom:12px;">ASR 模型</h3>
        <div id="model-list">${modelCardsHtml || '<div class="empty-state">无可用模型</div>'}</div>
      </div>
    `;

    // Path edit-in-place handler
    root.querySelector("#btn-change-path")?.addEventListener("click", () => {
      const display = root.querySelector<HTMLElement>("#model-path-display");
      if (!display) return;
      const currentPath = state.modelCachePath;
      const input = document.createElement("input");
      input.type = "text";
      input.value = currentPath;
      input.style.cssText =
        "flex:1;background:#1a1a1a;border:1px solid #4a9eff;border-radius:4px;color:#e0e0e0;padding:4px 8px;font-size:13px;font-family:monospace;outline:none;min-width:200px;";
      display.replaceWith(input);
      input.focus();
      input.select();

      function savePath(): void {
        const newPath = input.value.trim() || "~/.meetnote/models";
        state.modelCachePath = newPath;
        saveState(state);
        const span = document.createElement("span");
        span.id = "model-path-display";
        span.style.cssText =
          "color:#ccc;font-size:13px;font-family:monospace;flex:1;word-break:break-all;";
        span.textContent = newPath;
        input.replaceWith(span);
      }

      input.addEventListener("blur", savePath);
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") input.blur();
        if (e.key === "Escape") {
          const span = document.createElement("span");
          span.id = "model-path-display";
          span.style.cssText =
            "color:#ccc;font-size:13px;font-family:monospace;flex:1;word-break:break-all;";
          span.textContent = currentPath;
          input.removeEventListener("blur", savePath);
          input.replaceWith(span);
        }
      });
    });

    // Attach action listeners for each model card
    for (const m of whisperModels) {
      attachActionListeners(root, m.id, state);
      // Restore download UI state if there's an active download
      const dl = activeDownloads.get(m.id);
      if (dl && (dl.status === "downloading" || dl.status === "paused" || dl.status === "queued")) {
        updateDownloadUI(root, m.id, dl.downloadedBytes, dl.totalBytes, dl.speedBps, dl.status, state);
      }
    }

    // Setup Tauri event listener
    await setupProgressListener(state);
  })();
}
