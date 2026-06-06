# 系统架构概览

> 从 README 架构部分 + docs/overview.md 提纯。

## 目录结构

```
meetnote/
├── frontend/          # TypeScript 前端 (Bun + Web Components + Tauri IPC)
│   └── src/
│       ├── components/   # UI 组件 (app-shell, section-*, markdown-editor)
│       ├── stores/       # 状态管理 (ui-state, markdown-library-storage)
│       ├── types/        # 领域 Schema (asr, diarization, llm, audio-project, model-manifest)
│       └── utils/        # 工具模块 (speaker, llm-provider, model-manager, etc.)
├── tauri/             # Rust 桌面壳 (Tauri 2.x)
│   └── src/
│       ├── utils.rs      # expand_tilde, find_repo_root, find_uv_binary
│       ├── asr.rs        # ASR 桥接 → python/asr_cli.py
│       ├── diarization.rs# 发言人区分桥接 → python/diarization_cli.py
│       ├── download.rs   # 模型下载 (HTTP + library-managed)
│       ├── lib.rs        # 命令注册与插件配置
│       └── main.rs       # 入口
├── python/            # Python AI 推理后端
│   ├── meetnote/         # 共享包 (device.py, progress.py)
│   ├── asr_cli.py        # 多引擎 ASR CLI
│   ├── diarization_cli.py# pyannote speaker diarization CLI
│   └── model_downloader.py# 库管理模型下载
├── tests/
│   ├── api/ts/           # L2 契约测试
│   ├── e2e/specs/        # L3 Playwright E2E 测试
│   └── fixtures/         # 测试种子数据 (5 个会议场景)
├── scripts/dev.sh     # 开发环境入口
└── docs/              # ← 你现在在这里
```

## 架构原则

| 原则 | 说明 |
|------|------|
| **TS 优先** | TypeScript 承载 UI、状态管理、数据模型和可测试业务逻辑 |
| **Rust 按需** | 仅在权限、性能或进程编排时引入 Rust |
| **CLI 边界** | ASR / diarization 走 subprocess/CLI，JSON schema 交换 |
| **模型分离** | 模型不打包进 app，按需下载/校验/配置/切换 |
| **数据可追溯** | 原始转录、speaker 修正、去口语化文本不可逆覆盖原文 |
| **纯文件历史库** | Markdown 用文件系统重建索引，不依赖 SQLite/Tantivy |

## 调用链

```
前端 (TypeScript)                  Rust 桥接 (Tauri)            Python CLI
┌──────────────┐    invoke()     ┌────────────────┐   uv run    ┌──────────────────┐
│ section-     │ ──────────────→ │ asr.rs          │ ──────────→ │ asr_cli.py        │
│ transcription │                 │ diarization.rs  │             │ diarization_cli.py│
│ progress.ts  │ ←────────────── │ download.rs     │ ←────────── │ model_downloader. │
└──────────────┘   JSON result   └────────────────┘   stdout    └──────────────────┘
                                                         JSON
```

## 关键数据模型

所有 TypeScript schema 位于 `frontend/src/types/`：

| Schema | 文件 | 用途 |
|--------|------|------|
| ASR | `asr-schema.ts` | 转录 segment 结果 |
| Diarization | `diarization-schema.ts` | speaker segment 结果 |
| LLM | `llm-schema.ts` | 去口语化输入/输出 |
| Audio Project | `audio-project.ts` | 项目 manifest |
| Model Manifest | `model-manifest.ts` | 模型下载矩阵 |
| Markdown Library | `markdown-library-schema.ts` | 历史库 frontmatter |
