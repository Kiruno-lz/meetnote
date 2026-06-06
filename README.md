# meetnote

> 一个本地桌面会议纪要工作台：**导入音频 → ASR 转录 → Speaker 标注 → 人工修正 → LLM 去口语化 → Markdown 历史库**。
>
> 📖 **完整文档见 [`docs/index.md`](docs/index.md)** → 项目所有文档已整理到 `docs/` 目录。

## 快速导航

| 你想了解… | 看这里 |
|-----------|--------|
| 项目定位、用户、设计原则 | [`docs/guide/product.md`](docs/guide/product.md) |
| 系统架构、目录结构、数据流 | [`docs/architecture/overview.md`](docs/architecture/overview.md) |
| 开发者快速上手、MVP 范围 | [`docs/guide/quickstart.md`](docs/guide/quickstart.md) |
| ASR 4 引擎支持与使用 | [`docs/models/asr-models-guide.md`](docs/models/asr-models-guide.md) |
| 模型下载与管理 | [`docs/models/model-management.md`](docs/models/model-management.md) |
| Markdown 历史库 API | [`docs/markdown-library/schema.md`](docs/markdown-library/schema.md) |
| ASR CLI 契约（JSON 格式） | [`docs/reference/asr-cli.md`](docs/reference/asr-cli.md) |
| 音频项目管理 | [`docs/reference/audio-projects.md`](docs/reference/audio-projects.md) |
| 当前进度与测试状态 | [`docs/project/progress.md`](docs/project/progress.md) |
| 差距分析与下一步计划 | [`docs/project/gap-analysis.md`](docs/project/gap-analysis.md) |

## 开发入口

```bash
bash scripts/dev.sh            # 启动完整环境
bash scripts/dev.sh --check    # 依赖检查
bash scripts/dev.sh --frontend # 仅前端 dev server
bash scripts/dev.sh --tauri    # 启动 Tauri dev mode
```

## 技术栈

| 层 | 技术 |
|---|---|
| 桌面壳 | Tauri 2.x (Rust) |
| 前端 | TypeScript + Bun + Web Components |
| ASR | Python (Whisper / Paraformer / Qwen3-ASR / MiMo) |
| 发言人区分 | Python (pyannote.audio 4.x) |
| LLM 去口语化 | TypeScript (Ollama / custom provider) |
| 测试 | Bun test / Playwright |

## 目录结构

```
meetnote/
├── frontend/          # TypeScript 前端
├── tauri/             # Rust 桌面壳 (Tauri 2.x)
├── python/            # Python AI 推理后端
├── tests/             # L1/L2/L3 测试
├── scripts/dev.sh     # 开发入口
└── docs/              # ← 所有文档
```
