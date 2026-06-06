# meetnote 项目概览

> 合并自：AGENTS.md（项目目标/架构边界）、PRODUCT.md（产品定位）、README.md（简介）、PLAN.md（MVP 范围）、docs/decisions.md（决策记录）
> 最后更新：2026-06-06

## 项目定位

meetnote 是一个本地桌面会议纪要工作台，把会议音频的**转录、发言人区分与人工修正、忠实去口语化整理和 Markdown 纯文件历史库**整合成一个可追溯的本地工作流。

**核心价值主张**：隐私（数据不离本机）和可控性（用户决定模型和修正结果）。

**目标用户**：中文会议参与者（产品经理、研发、运营、研究员）在 Mac 桌面上处理会议录音。使用场景为**会后静态处理**，将几十分钟到几小时的会议音频转化为可维护的文字记录。

## MVP 范围

### ✅ 包含（MVP 必须覆盖）

1. 导入 `.wav`、`.mp3`、`.m4a` 音频文件
2. 项目化管理录音（标题、音频指纹、转录、speaker、修正、Markdown）
3. ASR 多引擎转录（Whisper / Paraformer / Qwen3-ASR / MiMo）
4. Speaker diarization（pyannote.audio 4.x）
5. Speaker 人工修正（重命名、合并、拆分、批量替换）
6. LLM 忠实去口语化（Ollama / custom provider，严格 schema 约束）
7. Markdown 纯文件历史库保存与重建（4 维度搜索）
8. `scripts/dev.sh` 环境自检与启动

### ❌ 非 MVP（后置到 release 或后续版本）

- TTS（语音合成）
- 摘要、行动项、概括性结论（去口语化仅做忠实改写）
- dmg、签名、公证、Homebrew cask
- 默认使用 SQLite 或 Tantivy 索引引擎
- 将模型权重打包进应用

## 架构边界

```
前端 (TypeScript + Web Components)
  │ invoke("run_asr", ...)
  │ invoke("start_model_download", ...)
  ▼
Rust 桥接 (Tauri 2.x)
  ├─ asr.rs         → python/asr_cli.py
  ├─ diarization.rs → python/diarization_cli.py
  ├─ download.rs    → python/model_downloader.py
  └─ utils.rs       (expand_tilde, find_repo_root, find_uv_binary)
  ▼
Python CLI (uv 管理)
  ├─ asr_cli.py           (Whisper / Paraformer / Qwen3-ASR / MiMo)
  ├─ diarization_cli.py   (pyannote.audio 4.x)
  ├─ model_downloader.py  (library-managed downloads)
  └─ meetnote/            (共用包: device.py, progress.py)
```

### 关键架构原则

| 原则 | 说明 |
|------|------|
| TS 优先 | TypeScript 承载 UI、状态管理、数据模型和可测试业务逻辑 |
| Rust 按需引入 | 仅在权限、性能或进程编排时使用 Rust |
| CLI 边界 | ASR / diarization 走 subprocess/CLI，交换 JSON schema |
| 模型分离 | 模型不打包进 app，提供按需下载、校验、配置和切换 |
| 数据可追溯 | 原始转录、speaker 修正、去口语化文本不可逆处理覆盖原文 |
| 纯文件历史库 | Markdown 历史库采用文件系统重建索引，不依赖 SQLite/Tantivy |

## 技术栈

| 层 | 技术 | 用途 |
|----|------|------|
| 桌面壳 | Tauri 2.x (Rust) | 原生窗口、文件系统、IPC 桥接 |
| 前端 | TypeScript + Bun + Web Components | UI 渲染、状态管理、用户交互 |
| ASR 推理 | Python (openai-whisper / funasr / qwen-asr / transformers) | 多引擎语音转文字 |
| 发言人区分 | Python (pyannote.audio 4.x) | 说话人分割与聚类 |
| LLM 去口语化 | TypeScript (llm-provider) | 忠实去口语化整理 |
| 包管理 | uv (Python) / Cargo (Rust) / Bun (TS) | 各语言依赖管理 |
| E2E 测试 | Playwright | 浏览器自动化端到端验证 |

## 产品设计原则

1. **流程优先于功能展示**：每个屏幕只有一个主任务
2. **密度匹配内容类型**：转录文本紧凑，操作区域有足够点击目标
3. **可逆、可追溯**：编辑操作有确认和回退路径
4. **工具气质**：控件朴素精确，无多余装饰
5. **隐私可见**：用户看到数据/模型在哪里，不做"黑箱"

## 开发入口

```bash
bash scripts/dev.sh          # 启动完整环境
bash scripts/dev.sh --check  # 依赖检查
bash scripts/dev.sh --frontend  # 仅前端 dev server
bash scripts/dev.sh --tauri  # 启动 Tauri dev
```

脚本不自动安装依赖、不下载模型、不写入 secret。

## 关键目录结构

```
meetnote/
├── frontend/          # TypeScript 前端 (Bun + Web Components + Tauri IPC)
│   ├── src/
│   │   ├── components/   # UI 组件 (app-shell, section-*)
│   │   ├── stores/       # 状态管理
│   │   ├── types/        # 领域 Schema (asr, diarization, llm, audio-project, model-manifest)
│   │   └── utils/        # 工具模块 (speaker, llm-provider, model-manager, etc.)
│   └── tests/            # L1 单元测试
├── tauri/             # Rust 桌面壳 (Tauri 2.x)
│   └── src/           # asr.rs, diarization.rs, download.rs, utils.rs, lib.rs
├── python/            # Python AI 推理后端
│   ├── asr_cli.py, diarization_cli.py, model_downloader.py
│   └── meetnote/      # 共享包 device.py, progress.py
├── tests/
│   ├── api/ts/        # L2 契约测试
│   ├── e2e/           # L3 Playwright E2E 测试
│   └── fixtures/      # 测试种子数据
├── scripts/dev.sh     # 开发环境入口
└── docs/              # 架构与配置文档
```
