import { describe, it, expect } from "bun:test";
import {
  createActiveDownload,
  applyProgressUpdate,
  computeSpeedBps,
  computeEtaSeconds,
  formatFileSize,
  formatSpeed,
  formatEta,
  getWhisperModelList,
} from "../src/utils/model-manager.ts";

// ── createActiveDownload ──────────────────────────────────────────────────────

describe("createActiveDownload", () => {
  it("initializes with expected default values", () => {
    const d = createActiveDownload("whisper-large-v3", "/tmp/large-v3.pt");
    expect(d).toEqual({
      modelId: "whisper-large-v3",
      destPath: "/tmp/large-v3.pt",
      status: "idle",
      downloadedBytes: 0,
      totalBytes: 0,
      speedBps: 0,
      eta: Infinity,
      error: null
    });
  });

  it("does not share state between two independent instances", () => {
    const a = createActiveDownload("whisper-small", "/tmp/small.pt");
    const b = createActiveDownload("whisper-large-v3", "/tmp/large-v3.pt");
    expect(a).not.toBe(b);
    expect(a.modelId).not.toBe(b.modelId);
    expect(a.destPath).not.toBe(b.destPath);
  });
});

// ── applyProgressUpdate ───────────────────────────────────────────────────────

describe("applyProgressUpdate", () => {
  const base = createActiveDownload("whisper-large-v3", "/tmp/large-v3.pt");

  it("applies all progress fields and transitions status to downloading", () => {
    const updated = applyProgressUpdate(base, 512_000, 1_000_000, 100_000);
    expect(updated).toMatchObject({
      downloadedBytes: 512_000,
      totalBytes: 1_000_000,
      speedBps: 100_000,
      status: "downloading"
    });
  });

  it("does not mutate the original object", () => {
    const original = { ...base };
    applyProgressUpdate(base, 512_000, 1_000_000, 100_000);
    expect(base.downloadedBytes).toBe(original.downloadedBytes);
    expect(base.totalBytes).toBe(original.totalBytes);
    expect(base.speedBps).toBe(original.speedBps);
    expect(base.status).toBe(original.status);
  });

  it("computes eta from given speed", () => {
    // 500_000 remaining at 100_000 bps = 5 seconds
    const updated = applyProgressUpdate(base, 500_000, 1_000_000, 100_000);
    expect(updated.eta).toBe(5);
  });

  it("sets eta to Infinity when speedBps is 0", () => {
    const updated = applyProgressUpdate(base, 0, 1_000_000, 0);
    expect(updated.eta).toBe(Infinity);
  });

  it("sets eta to 0 when download is complete", () => {
    const updated = applyProgressUpdate(base, 1_000_000, 1_000_000, 50_000);
    expect(updated.eta).toBe(0);
  });
});

// ── computeSpeedBps ───────────────────────────────────────────────────────────

describe("computeSpeedBps", () => {
  it("calculates speed correctly", () => {
    // 200KB in 1000ms → 200 KB/s
    expect(computeSpeedBps(0, 204_800, 1000)).toBe(204_800);
  });

  it("returns 0 when deltaMs is 0 (no divide-by-zero)", () => {
    expect(computeSpeedBps(0, 1000, 0)).toBe(0);
  });

  it("returns 0 when deltaMs is negative", () => {
    expect(computeSpeedBps(0, 1000, -100)).toBe(0);
  });

  it("handles equal byte counts (no progress)", () => {
    expect(computeSpeedBps(500, 500, 1000)).toBe(0);
  });

  it("scales with time correctly", () => {
    // 100KB in 2000ms → 50 KB/s
    expect(computeSpeedBps(0, 102_400, 2000)).toBe(51_200);
  });
});

// ── computeEtaSeconds ─────────────────────────────────────────────────────────

describe("computeEtaSeconds", () => {
  it("returns Infinity when speedBps is 0", () => {
    expect(computeEtaSeconds(0, 1_000_000, 0)).toBe(Infinity);
  });

  it("calculates remaining seconds correctly", () => {
    // 500KB remaining at 100KB/s → 5s
    expect(computeEtaSeconds(500_000, 1_000_000, 100_000)).toBe(5);
  });

  it("returns 0 when already fully downloaded", () => {
    expect(computeEtaSeconds(1_000_000, 1_000_000, 100_000)).toBe(0);
  });

  it("returns Infinity for negative speed", () => {
    expect(computeEtaSeconds(0, 1_000_000, -1)).toBe(Infinity);
  });

  it("handles non-integer results", () => {
    // 750KB remaining at 200KB/s → 3.75s
    expect(computeEtaSeconds(250_000, 1_000_000, 200_000)).toBeCloseTo(3.75);
  });
});

// ── formatFileSize ────────────────────────────────────────────────────────────

describe("formatFileSize", () => {
  it("formats 0 bytes", () => {
    expect(formatFileSize(0)).toBe("0 B");
  });

  it("formats 500 bytes", () => {
    expect(formatFileSize(500)).toBe("500 B");
  });

  it("formats 1023 bytes (just below 1 KB)", () => {
    expect(formatFileSize(1023)).toBe("1023 B");
  });

  it("formats 1024 bytes as 1.0 KB", () => {
    expect(formatFileSize(1024)).toBe("1.0 KB");
  });

  it("formats 1 MB", () => {
    expect(formatFileSize(1024 * 1024)).toBe("1.0 MB");
  });

  it("formats 1.5 GB", () => {
    expect(formatFileSize(1.5 * 1024 * 1024 * 1024)).toBe("1.50 GB");
  });

  it("formats 1 GB boundary", () => {
    expect(formatFileSize(1024 * 1024 * 1024)).toBe("1.00 GB");
  });

  it("formats fractional KB", () => {
    expect(formatFileSize(1536)).toBe("1.5 KB");
  });
});

// ── formatSpeed ───────────────────────────────────────────────────────────────

describe("formatSpeed", () => {
  it("formats 0 bps", () => {
    expect(formatSpeed(0)).toBe("0 B/s");
  });

  it("formats 512 bps", () => {
    expect(formatSpeed(512)).toBe("512 B/s");
  });

  it("formats 1023 bps (just below 1 KB/s)", () => {
    expect(formatSpeed(1023)).toBe("1023 B/s");
  });

  it("formats 1024 bps as 1.0 KB/s", () => {
    expect(formatSpeed(1024)).toBe("1.0 KB/s");
  });

  it("formats 1048576 bps as 1.0 MB/s", () => {
    expect(formatSpeed(1048576)).toBe("1.0 MB/s");
  });

  it("formats 10 MB/s", () => {
    expect(formatSpeed(10 * 1024 * 1024)).toBe("10.0 MB/s");
  });

  it("formats fractional KB/s", () => {
    expect(formatSpeed(2048)).toBe("2.0 KB/s");
  });
});

// ── formatEta ─────────────────────────────────────────────────────────────────

describe("formatEta", () => {
  it("formats Infinity as em dash", () => {
    expect(formatEta(Infinity)).toBe("—");
  });

  it("formats NaN as em dash", () => {
    expect(formatEta(NaN)).toBe("—");
  });

  it("formats 30 seconds as < 1 分钟", () => {
    expect(formatEta(30)).toBe("< 1 分钟");
  });

  it("formats 59 seconds as < 1 分钟", () => {
    expect(formatEta(59)).toBe("< 1 分钟");
  });

  it("formats 90 seconds as 1 分钟", () => {
    expect(formatEta(90)).toBe("1 分钟");
  });

  it("formats 120 seconds as 2 分钟", () => {
    expect(formatEta(120)).toBe("2 分钟");
  });

  it("formats 3700 seconds as 1 小时 1 分钟", () => {
    expect(formatEta(3700)).toBe("1 小时 1 分钟");
  });

  it("formats 7200 seconds as 2 小时 0 分钟", () => {
    expect(formatEta(7200)).toBe("2 小时 0 分钟");
  });

  it("formats 3599 seconds (just under 1 hour) as minutes", () => {
    expect(formatEta(3599)).toBe("59 分钟");
  });
});

// ── getWhisperModelList ───────────────────────────────────────────────────────

describe("getWhisperModelList", () => {
  it("returns a non-empty list of well-formed ASR model entries", () => {
    const list = getWhisperModelList();
    expect(list.length).toBeGreaterThan(0);
    for (const entry of list) {
      expect(typeof entry.id).toBe("string");
      expect(entry.id.length).toBeGreaterThan(0);
      expect(typeof entry.label).toBe("string");
      expect(entry.label.length).toBeGreaterThan(0);
      expect(typeof entry.source).toBe("string");
      expect(entry.source.length).toBeGreaterThan(0);
      expect(typeof entry.description).toBe("string");
      expect(entry.description.length).toBeGreaterThan(0);
      expect(entry.sizeBytes === null || typeof entry.sizeBytes === "number").toBe(true);
    }
  });

  it("only returns asr kind models", () => {
    // All returned entries should exist in MODEL_MATRIX as kind=asr
    const { MODEL_MATRIX } = require("../src/types/model-manifest.ts");
    const asrIds = new Set(MODEL_MATRIX.filter((m: { kind: string }) => m.kind === "asr").map((m: { id: string }) => m.id));
    const list = getWhisperModelList();
    for (const entry of list) {
      expect(asrIds.has(entry.id)).toBe(true);
    }
  });

  it("does not include diarization models", () => {
    const list = getWhisperModelList();
    const ids = list.map((e) => e.id);
    expect(ids.some((id) => id.includes("pyannote"))).toBe(false);
  });
});
