import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const repoRoot = join(import.meta.dir, "..", "..");

describe("model documentation", () => {
  test("documents the PLAN section 3 model operations boundary", () => {
    const modelsDoc = readFileSync(join(repoRoot, "docs", "models.md"), "utf8");

    expect(modelsDoc).toContain("Whisper large-v3");
    expect(modelsDoc).toContain("模型矩阵");
    expect(modelsDoc).toContain("下载状态 JSON schema");
    expect(modelsDoc).toContain("缓存策略");
    expect(modelsDoc).toContain("pyannote/speaker-diarization-3.1");
    expect(modelsDoc).toContain("模型权重不进入 app 包");
    expect(modelsDoc).toContain("bun test model-manifest");
    expect(modelsDoc).toContain("bun test model-download.contract.test.ts");
  });
});
