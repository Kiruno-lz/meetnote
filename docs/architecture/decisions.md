# 决策记录

> 从 docs/decisions.md + PLAN.md 风险节提纯。记录了项目至今的关键决策和边界约束。

## 已确认范围

### MVP 边界

| 包含 | 不包含 |
|------|--------|
| 音频导入 (.wav/.mp3/.m4a) | TTS |
| 项目化管理 | 摘要、行动项、概括性结论 |
| ASR 多引擎 (Whisper/Paraformer/Qwen3-ASR/MiMo) | 默认 SQLite / Tantivy |
| Speaker diarization (pyannote) | 模型权重打包进应用 |
| Speaker 人工修正 | dmg / 签名 / 公证 / Homebrew cask |
| LLM 忠实去口语化 (Ollama / custom) | |
| Markdown 纯文件历史库 | |

### 架构决策

- **TS 优先**：TypeScript 优先承载 UI、状态管理和可测试业务逻辑；Rust 仅在权限/性能/进程编排时引入。
- **CLI 边界**：ASR 与 pyannote 走 subprocess/CLI 边界，使用稳定 JSON schema + 错误码 + 进度事件连接 app。
- **模型下载**：模型权重不进入 app 包，安装后按需下载/配置/校验/切换。
- **纯文件历史库**：Markdown 历史库采用纯文件重建索引，不默认引入 SQLite/Tantivy。
- **数据可追溯**：原始转录、speaker 修正、去口语化文本和最终 Markdown 必须可追溯；不可逆处理不得覆盖原文。
- **开发入口**：`scripts/dev.sh` 是仓库约定的开发入口，支持 `--check`、`--frontend`、`--tauri`；不自动安装依赖、不下载模型、不写入 secret。

### ASR 策略

- 质量优先，优先评估 Whisper large-v3
- 主要面向中文会议，支持夹杂少量英文片段
- 现已支持 4 引擎：Whisper / Paraformer / Qwen3-ASR / MiMo

## 风险与待定决策

| 风险/决策 | 状态 |
|-----------|------|
| Whisper large-v3 本地性能与内存 | 持续评估，用户可选切换其他引擎 |
| 中文会议夹杂英文识别质量 | 用混合语言夹具评估 |
| pyannote 模型授权与认证流程 | Python CLI 边界，用户自行配置 HF token |
| ASR 时间戳与 speaker segment 合并误差 | 合并算法已实现，需真实数据验证 |
| LLM 去口语化过度改写 | 严格 schema + prompt 约束 |
| 纯文件历史库何时升级索引引擎 | >10k 会议时评估 |
| Release 是否优先 macOS | 后置到 release 阶段决策 |
