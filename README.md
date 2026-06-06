# meetnote

meetnote 是一个面向本地桌面的会议纪要工作台，核心目标是把会议音频转录、发言人区分、忠实去口语化整理和 Markdown 历史库沉淀为可维护、可追溯的本地项目。

## 框架架构

```
meetnote/
├── frontend/          # TypeScript 前端 (Bun + Web Components + Tauri IPC)
│   └── src/
│       ├── components/   # UI 组件 (app-shell, section-*, meeting-record-item, markdown-editor)
│       ├── stores/       # 状态管理 (ui-state, markdown-library-storage)
│       ├── types/        # 领域类型与 Schema (asr, diarization, llm, audio-project, model-manifest)
│       └── utils/        # 工具模块 (model-manager, llm-provider, speaker, meeting-management)
├── tauri/             # Rust 桌面壳 (Tauri 2.x)
│   └── src/
│       ├── utils.rs      # 共享工具 (expand_tilde, find_repo_root, find_uv_binary)
│       ├── asr.rs        # ASR 桥接 → python/asr_cli.py
│       ├── diarization.rs# 发言人区分桥接 → python/diarization_cli.py
│       ├── download.rs   # 模型下载管理 (HTTP + library-managed)
│       ├── lib.rs        # Tauri 命令注册与插件配置
│       └── main.rs       # 入口
├── python/            # Python AI 推理后端
│   ├── meetnote/         # 共享包
│   │   ├── device.py     # PyTorch 设备解析 (auto/mps/cpu/cuda)
│   │   └── progress.py   # stderr PROGRESS: 事件发送
│   ├── asr_cli.py        # 多引擎 ASR CLI (Whisper/Paraformer/Qwen3-ASR/MiMo)
│   ├── diarization_cli.py# 发言人区分 CLI (pyannote.audio 4.x)
│   └── model_downloader.py # 库管理模型下载器
├── tests/
│   ├── api/ts/           # L2 API 契约测试
│   ├── e2e/specs/        # L3 Playwright E2E 测试
│   └── fixtures/         # 测试种子数据
├── scripts/dev.sh     # 一键开发环境 (环境自检 / 前端 / Tauri)
└── docs/              # 架构与配置文档
```

## 技术栈

| 层 | 技术 | 用途 |
|---|---|---|
| **桌面壳** | Tauri 2.x (Rust) | 原生窗口、文件系统、IPC 桥接 |
| **前端** | TypeScript + Bun + Web Components | UI 渲染、状态管理、用户交互 |
| **ASR 推理** | Python (openai-whisper / funasr / qwen-asr / transformers) | 多引擎语音转文字 |
| **发言人区分** | Python (pyannote.audio 4.x) | 说话人分割与聚类 |
| **LLM 去口语化** | TypeScript (llm-provider) | 忠实去口语化整理 |
| **包管理** | uv (Python) / Cargo (Rust) / Bun (TS) | 各语言依赖管理 |
| **E2E 测试** | Playwright | 浏览器自动化端到端验证 |

### 数据流路径

```
音频文件 (.wav/.mp3/.m4a)
  → Tauri IPC (前端 → Rust 桥接)
  → Python CLI subprocess (stdout JSON / stderr PROGRESS)
  → 前端 TranscriptionResult (segments[] 时间轴)
  → Speaker Diarization (pyannote, 可选)
  → LLM 去口语化 (忠实改写)
  → Markdown 历史库 (纯文件沉淀, 可追溯)
```

## 定位

- 中文会议优先，兼顾少量英文片段
- ASR 默认 Whisper large-v3，支持 Paraformer / Qwen3-ASR / MiMo 多引擎
- 模型不内置进应用包，安装后按需下载、配置、校验和切换
- speaker 标注与人工修正是核心能力，不是后置增强
- 去口语化由本地 LLM 或云端 custom provider 完成，输出必须受严格 schema 约束
- Markdown 纯文件历史库是核心数据资产，原始转录和修正记录必须可追溯
- 音频项目以纯 TS manifest 契约管理，数据目录和重复导入规则见 `docs/audio-projects.md`

模型矩阵、下载流程、缓存策略和 pyannote 外部配置见 `docs/models.md`。

音频项目管理、项目 manifest 和重复导入策略见 `docs/audio-projects.md`。

ASR CLI JSON 边界、转录结果 schema、进度事件和稳定错误码见 `docs/asr-cli.md`。

## MVP

MVP 只覆盖开发者可运行的最小闭环：

1. 导入 `.wav`、`.mp3`、`.m4a`
2. 项目化管理录音
3. ASR (多引擎)
4. speaker diarization
5. speaker 人工修正
6. LLM 忠实去口语化
7. Markdown 历史库保存与重建
8. `scripts/dev.sh` 环境自检

## 非 MVP

- TTS
- 摘要、行动项、概括性结论
- dmg、签名、公证、Homebrew cask
- 默认引入 SQLite 或 Tantivy
- 将模型权重打包进应用

## 开发入口

仓库约定的开发入口是 `scripts/dev.sh`：

```bash
bash scripts/dev.sh --check
bash scripts/dev.sh --frontend
bash scripts/dev.sh --tauri
```

`--frontend` 启动 Bun 前端开发服务器；`--tauri` 启动 Tauri dev mode，要求本机已有 Tauri CLI。脚本不会自动安装依赖、下载模型或写入 secret。

前端骨架位于 `frontend/`，包含纯 TypeScript domain modules、Bun 测试、类型检查、dev server 和 build script。Tauri 壳层位于 `tauri/`，暴露桌面壳、模型下载管理、ASR/Diarization Python 桥接命令。
