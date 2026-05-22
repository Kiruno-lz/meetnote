import type { AppState } from "./ui-state.ts";
import {
  getWhisperModelList,
  formatFileSize,
  formatSpeed,
  formatEta,
  type ActiveDownload,
  type ModelFileInfo,
  type DownloadStatus,
} from "./model-manager.ts";

// Detect Tauri runtime environment
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// Module-level unlisten handle to avoid stacking listeners across re-renders
let _unlisten: (() => void) | null = null;

// Active downloads map (module-level so it survives re-renders within the session)
const activeDownloads = new Map<string, ActiveDownload>();

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
          <button class="btn btn-sm btn-secondary btn-pause" data-model-id="${escapeHtml(modelId)}">暂停</button>
          <button class="btn btn-sm btn-secondary btn-cancel" data-model-id="${escapeHtml(modelId)}">取消</button>
        </div>`;
    }
    if (status === "paused") {
      return `
        <div style="display:flex;gap:8px;">
          <button class="btn btn-sm btn-primary btn-resume" data-model-id="${escapeHtml(modelId)}">继续</button>
          <button class="btn btn-sm btn-secondary btn-cancel" data-model-id="${escapeHtml(modelId)}">取消</button>
        </div>`;
    }
  }

  if (isInstalled) {
    if (isActive) {
      return `<span style="color:#66bb6a;font-size:13px;font-weight:600;">✓ 当前使用</span>`;
    }
    return `
      <div style="display:flex;gap:8px;">
        <button class="btn btn-sm btn-secondary btn-use-model" data-model-id="${escapeHtml(modelId)}">切换使用</button>
        <button class="btn btn-sm btn-danger btn-delete-model" data-model-id="${escapeHtml(modelId)}">删除</button>
      </div>`;
  }

  return `<button class="btn btn-sm btn-primary btn-download" data-model-id="${escapeHtml(modelId)}">下载</button>`;
}

// ── Progress bar renderer ─────────────────────────────────────────────────────

function renderProgressBar(modelId: string): string {
  return `
    <div id="model-progress-${escapeHtml(modelId)}" style="display:none;margin-top:12px;">
      <div class="progress-bar-container" style="height:6px;background:#2e2e2e;border-radius:3px;overflow:hidden;">
        <div class="progress-bar" id="pbar-${escapeHtml(modelId)}" style="width:0%;height:100%;background:#4a9eff;border-radius:3px;transition:width .3s;"></div>
      </div>
      <div style="display:flex;justify-content:space-between;margin-top:4px;font-size:12px;color:#888;">
        <span id="pbar-detail-${escapeHtml(modelId)}">0 B / 0 B</span>
        <span id="pbar-speed-${escapeHtml(modelId)}">— B/s</span>
        <span id="pbar-eta-${escapeHtml(modelId)}">—</span>
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
  status: DownloadStatus
): void {
  const progressDiv = root.querySelector<HTMLElement>(`#model-progress-${modelId}`);
  const pbar = root.querySelector<HTMLElement>(`#pbar-${modelId}`);
  const pbarDetail = root.querySelector<HTMLElement>(`#pbar-detail-${modelId}`);
  const pbarSpeed = root.querySelector<HTMLElement>(`#pbar-speed-${modelId}`);
  const pbarEta = root.querySelector<HTMLElement>(`#pbar-eta-${modelId}`);
  const actionsDiv = root.querySelector<HTMLElement>(`#model-actions-${modelId}`);

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
    attachActionListeners(root, modelId);
  }
}

// ── Tauri invoke/listen (dynamic import to avoid crashing in browser) ─────────

interface DownloadProgressEvent {
  modelId: string;
  downloadedBytes: number;
  totalBytes: number;
  speedBps: number;
  status: DownloadStatus;
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

function attachActionListeners(root: HTMLElement, modelId: string): void {
  root.querySelector<HTMLElement>(`.btn-download[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void startDownload(root, modelId));

  root.querySelector<HTMLElement>(`.btn-pause[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void pauseDownload(modelId));

  root.querySelector<HTMLElement>(`.btn-resume[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void resumeDownload(root, modelId));

  root.querySelector<HTMLElement>(`.btn-cancel[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void cancelDownload(root, modelId));

  root.querySelector<HTMLElement>(`.btn-use-model[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void switchModel(root, modelId));

  root.querySelector<HTMLElement>(`.btn-delete-model[data-model-id="${modelId}"]`)
    ?.addEventListener("click", () => void deleteModel(root, modelId));
}

async function startDownload(root: HTMLElement, modelId: string): Promise<void> {
  if (!isTauri) return;
  const destPath = `~/.meetnote/models/${modelId}.bin`;
  const whisperModels = getWhisperModelList();
  const model = whisperModels.find((m) => m.id === modelId);
  if (!model) return;

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

  const actionsDiv = root.querySelector<HTMLElement>(`#model-actions-${modelId}`);
  if (actionsDiv) {
    actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: dl });
    attachActionListeners(root, modelId);
  }
  updateDownloadUI(root, modelId, 0, dl.totalBytes, 0, "queued");

  try {
    await tauriInvoke("start_model_download", { modelId, url: model.source, destPath });
  } catch (err) {
    console.error("start_model_download failed", err);
    activeDownloads.delete(modelId);
    // Re-render actions back to downloadable
    if (actionsDiv) {
      actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: undefined });
      attachActionListeners(root, modelId);
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

async function resumeDownload(root: HTMLElement, modelId: string): Promise<void> {
  if (!isTauri) return;
  const dl = activeDownloads.get(modelId);
  if (dl) activeDownloads.set(modelId, { ...dl, status: "downloading" });
  try {
    await tauriInvoke("resume_model_download", { modelId });
  } catch (err) {
    console.error("resume_model_download failed", err);
  }
  const actionsDiv = root.querySelector<HTMLElement>(`#model-actions-${modelId}`);
  if (actionsDiv) {
    actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: activeDownloads.get(modelId) });
    attachActionListeners(root, modelId);
  }
}

async function cancelDownload(root: HTMLElement, modelId: string): Promise<void> {
  if (!isTauri) return;
  try {
    await tauriInvoke("cancel_model_download", { modelId });
  } catch (err) {
    console.error("cancel_model_download failed", err);
  }
  activeDownloads.delete(modelId);
  const actionsDiv = root.querySelector<HTMLElement>(`#model-actions-${modelId}`);
  const progressDiv = root.querySelector<HTMLElement>(`#model-progress-${modelId}`);
  if (progressDiv) progressDiv.style.display = "none";
  if (actionsDiv) {
    actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: undefined });
    attachActionListeners(root, modelId);
  }
}

async function switchModel(root: HTMLElement, modelId: string): Promise<void> {
  if (!isTauri) return;
  try {
    await tauriInvoke("set_active_model", { modelId });
    // Refresh the active model label
    const label = root.querySelector<HTMLElement>("#active-model-label");
    if (label) label.textContent = modelId;
  } catch (err) {
    console.error("set_active_model failed", err);
  }
}

async function deleteModel(root: HTMLElement, modelId: string): Promise<void> {
  if (!isTauri) return;
  try {
    await tauriInvoke("delete_cached_model", { modelId });
    // Re-render the card actions
    const actionsDiv = root.querySelector<HTMLElement>(`#model-actions-${modelId}`);
    if (actionsDiv) {
      actionsDiv.innerHTML = renderModelActions({ modelId, isInstalled: false, isActive: false, download: undefined });
      attachActionListeners(root, modelId);
    }
  } catch (err) {
    console.error("delete_cached_model failed", err);
  }
}

// ── Listener setup ────────────────────────────────────────────────────────────

async function setupProgressListener(root: HTMLElement): Promise<void> {
  if (!isTauri) return;
  // Tear down previous listener if any
  if (_unlisten) {
    _unlisten();
    _unlisten = null;
  }
  _unlisten = await tauriListen("model-download-progress", (payload) => {
    activeDownloads.set(payload.modelId, {
      modelId: payload.modelId,
      status: payload.status,
      downloadedBytes: payload.downloadedBytes,
      totalBytes: payload.totalBytes,
      speedBps: payload.speedBps,
      eta: payload.speedBps > 0 ? (payload.totalBytes - payload.downloadedBytes) / payload.speedBps : Infinity,
      error: null,
      destPath: `~/.meetnote/models/${payload.modelId}.bin`,
    });
    updateDownloadUI(
      root,
      payload.modelId,
      payload.downloadedBytes,
      payload.totalBytes,
      payload.speedBps,
      payload.status
    );
  });
}

// ── Main render function ──────────────────────────────────────────────────────

export function renderModelManagement(root: HTMLElement, _state: AppState): void {
  void (async () => {
    // Load installed models
    let installedModels: ModelFileInfo[];
    let activeModelId: string | null = null;

    if (isTauri) {
      try {
        installedModels = await tauriInvoke<ModelFileInfo[]>("list_cached_models", {
          dir: "~/.meetnote/models",
        });
        activeModelId = installedModels.find((m) => m.isActive)?.modelId ?? null;
      } catch {
        installedModels = [];
      }
    } else {
      installedModels = MOCK_INSTALLED_MODELS;
      activeModelId = installedModels.find((m) => m.isActive)?.modelId ?? null;
    }

    const whisperModels = getWhisperModelList();
    const installedIds = new Set(installedModels.map((m) => m.modelId));

    // Build model cards HTML
    const modelCardsHtml = whisperModels
      .map((m) => {
        const isInstalled = installedIds.has(m.id);
        const isActive = m.id === activeModelId;
        const download = activeDownloads.get(m.id);
        const sizeStr = m.sizeBytes !== null ? formatFileSize(m.sizeBytes) : "大小未知";

        return `
          <div class="card" id="model-card-${escapeHtml(m.id)}" style="margin-bottom:12px;">
            <div style="display:flex;justify-content:space-between;align-items:flex-start;">
              <div style="flex:1;min-width:0;margin-right:16px;">
                <div style="font-weight:600;color:#e0e0e0;margin-bottom:2px;">${escapeHtml(m.label)}</div>
                <div style="font-size:12px;color:#888;">${escapeHtml(sizeStr)} · ${escapeHtml(m.description)}</div>
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
        <h3 style="font-size:15px;color:#e0e0e0;margin-bottom:12px;">Whisper 模型</h3>
        <div id="model-list">${modelCardsHtml || '<div class="empty-state">无可用模型</div>'}</div>
      </div>
    `;

    // Attach action listeners for each model card
    for (const m of whisperModels) {
      attachActionListeners(root, m.id);
      // Restore download UI state if there's an active download
      const dl = activeDownloads.get(m.id);
      if (dl && (dl.status === "downloading" || dl.status === "paused" || dl.status === "queued")) {
        updateDownloadUI(root, m.id, dl.downloadedBytes, dl.totalBytes, dl.speedBps, dl.status);
      }
    }

    // Setup Tauri event listener
    await setupProgressListener(root);
  })();
}
