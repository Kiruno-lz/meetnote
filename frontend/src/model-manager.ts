import { MODEL_MATRIX } from "./model-manifest.ts";

// Download status (frontend-managed)
export type DownloadStatus =
  | "idle"
  | "queued"
  | "downloading"
  | "paused"
  | "completed"
  | "cancelled"
  | "failed";

export interface ActiveDownload {
  modelId: string;
  status: DownloadStatus;
  downloadedBytes: number;
  totalBytes: number;
  speedBps: number;   // bytes per second (instantaneous)
  eta: number;        // seconds remaining (Infinity if unknown)
  error: string | null;
  destPath: string;
}

export interface ModelFileInfo {
  modelId: string;
  path: string;
  sizeBytes: number;
  isActive: boolean;
}

// ── State factory ────────────────────────────────────────────────────────────

export function createActiveDownload(
  modelId: string,
  destPath: string
): ActiveDownload {
  return {
    modelId,
    status: "idle",
    downloadedBytes: 0,
    totalBytes: 0,
    speedBps: 0,
    eta: Infinity,
    error: null,
    destPath,
  };
}

// ── Progress update (returns new object, does not mutate) ────────────────────

export function applyProgressUpdate(
  current: ActiveDownload,
  downloadedBytes: number,
  totalBytes: number,
  speedBps: number
): ActiveDownload {
  return {
    ...current,
    downloadedBytes,
    totalBytes,
    speedBps,
    eta: computeEtaSeconds(downloadedBytes, totalBytes, speedBps),
    status: "downloading",
  };
}

// ── Speed / ETA computation ───────────────────────────────────────────────────

export function computeSpeedBps(
  prevBytes: number,
  currBytes: number,
  deltaMs: number
): number {
  if (deltaMs <= 0) return 0;
  return (currBytes - prevBytes) / (deltaMs / 1000);
}

export function computeEtaSeconds(
  downloadedBytes: number,
  totalBytes: number,
  speedBps: number
): number {
  if (speedBps <= 0) return Infinity;
  const remaining = totalBytes - downloadedBytes;
  if (remaining <= 0) return 0;
  return remaining / speedBps;
}

// ── Formatting utilities ──────────────────────────────────────────────────────

const KB = 1024;
const MB = 1024 * 1024;
const GB = 1024 * 1024 * 1024;

export function formatFileSize(bytes: number): string {
  if (bytes < KB) {
    return `${Math.floor(bytes)} B`;
  }
  if (bytes < MB) {
    return `${(bytes / KB).toFixed(1)} KB`;
  }
  if (bytes < GB) {
    return `${(bytes / MB).toFixed(1)} MB`;
  }
  return `${(bytes / GB).toFixed(2)} GB`;
}

export function formatSpeed(bps: number): string {
  if (bps < KB) {
    return `${Math.floor(bps)} B/s`;
  }
  if (bps < MB) {
    return `${(bps / KB).toFixed(1)} KB/s`;
  }
  return `${(bps / MB).toFixed(1)} MB/s`;
}

export function formatEta(seconds: number): string {
  if (!Number.isFinite(seconds)) return "—";
  if (seconds < 60) return "< 1 分钟";
  if (seconds < 3600) {
    const minutes = Math.floor(seconds / 60);
    return `${minutes} 分钟`;
  }
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return `${hours} 小时 ${minutes} 分钟`;
}

// ── Whisper model list ────────────────────────────────────────────────────────

const WHISPER_DISPLAY_META: Record<string, { label: string; description: string }> = {
  "whisper-large-v3": {
    label: "Whisper Large v3",
    description: "OpenAI Whisper Large v3 — high accuracy, Chinese + English mixed speech",
  },
  "whisper-large-v3-turbo": {
    label: "Whisper Large v3 Turbo",
    description: "蒸馏版 Large v3，速度提升 ~8x，精度接近原版 — 推荐日常使用",
  },
  "whisper-large-v3-turbo-q5_0": {
    label: "Whisper Large v3 Turbo (Q5量化)",
    description: "Turbo 的 Q5_0 量化版，体积更小，适合内存受限设备",
  },
  "whisper-medium": {
    label: "Whisper Medium",
    description: "OpenAI Whisper Medium — balanced speed and accuracy",
  },
  "whisper-small": {
    label: "Whisper Small",
    description: "OpenAI Whisper Small — fast, lower memory footprint",
  },
  "whisper-base": {
    label: "Whisper Base",
    description: "OpenAI Whisper Base — lightweight, minimal memory requirements",
  },
  "whisper-tiny": {
    label: "Whisper Tiny",
    description: "OpenAI Whisper Tiny — smallest and fastest Whisper model",
  },
};

export function getWhisperModelList(): Array<{
  id: string;
  label: string;
  sizeBytes: number | null;
  source: string;
  description: string;
}> {
  return MODEL_MATRIX.filter((m) => m.kind === "asr").map((m) => {
    const meta = WHISPER_DISPLAY_META[m.id];
    return {
      id: m.id,
      label: meta?.label ?? m.id,
      sizeBytes: m.downloadSizeBytes,
      source: m.source,
      description: meta?.description ?? m.source,
    };
  });
}
