# 目录说明

> ⚠️ 此文件已过时。实际仓库已从初始化阶段演进到完整 MVP 实现。
> 请参考 `docs/overview.md` 获取最新架构和目录说明，参考 `docs/progress.md` 获取实现状态。
>
> 保留此文件仅作历史参考。

## 历史参考（过时内容）

### 当前初始化目录（已不准确）

- `frontend/`：Tauri 前端与 TypeScript 业务逻辑
- `src-tauri/`：**已重命名为 `tauri/`**
- `python/`：**已实现 ASR/Diarization/Model Downloader CLI**
- `tests/`：**已包含 L1/L2/L3 测试，但 L2 当前因路径问题失败**
- `script/`：已包含 `dev.sh`
- `docs/`：决策、架构等设计文档

### 当前未实现内容（已实现）

以下条目在 docs/structure.md 创建时标注为"未实现"，现已全部实现：

- ✅ ASR subprocess → `python/asr_cli.py`
- ✅ speaker diarization → `python/diarization_cli.py`
- ✅ LLM provider → `frontend/src/utils/llm-provider.ts`
- ✅ Markdown 文件 IO → `frontend/src/markdown-library-storage.ts`
- ✅ 模型下载器 → `python/model_downloader.py` + `tauri/src/download.rs`
- ✅ 音频项目存储 → `frontend/src/types/audio-project.ts`
- ✅ Real ASR TypeScript schema → `frontend/src/types/asr-schema.ts`
