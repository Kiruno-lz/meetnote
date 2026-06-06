import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const repoRoot = join(import.meta.dir, "..", "..");

describe("model documentation", () => {
  test("documents the PLAN section 3 model operations boundary", () => {
    const modelsDoc = readFileSync(join(repoRoot, "docs", "models", "model-management.md"), "utf8");

    expect(modelsDoc).toContain("Whisper large-v3");
    expect(modelsDoc).toContain("模型矩阵");
    expect(modelsDoc).toContain("缓存策略");
    expect(modelsDoc).toContain("pyannote/speaker-diarization-3.1");
    expect(modelsDoc).toContain("不由 app 下载");
    expect(modelsDoc).toContain("Manifest Schema");
    expect(modelsDoc).toContain("meetnote.model-manifest.v1");
  });

  test("documents the PLAN section 6 ASR CLI boundary", () => {
    const asrDoc = readFileSync(join(repoRoot, "docs", "reference", "asr-cli.md"), "utf8");

    expect(asrDoc).toContain("ASR CLI 契约");
    expect(asrDoc).toContain("meetnote.asr.v1");
    expect(asrDoc).toContain("ASR_USER_CANCELLED");
    expect(asrDoc).toContain("confidence");
    expect(asrDoc).toContain("Progress JSON");
    expect(asrDoc).toContain("Error JSON");
  });
});
