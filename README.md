# MeetNote

MeetNote 是 Rust-native、本地优先的音频处理与 AI 可读文档工作区。它把任意音频资产转换为带时间戳、说话人和处理来源的文档；语义拆分、标题、标签与总结由 Assistant 或外部 Agent 按 Skill 决定，MeetNote 负责统一读写、结构校验和最多三层的项目树。

当前仓库已经具备 GPUI 桌面入口、文档目录扫描、模型管理、ASR、speaker diarization、Speaker Library、短音频整文件处理和长音频离线分窗。Assistant 与外部 Agent 的接入边界已经定义，具体 Agent/provider 尚未接入；长音频的 20 小时资源与精度验收、macOS 发布流程仍未完成。

## 快速构建

要求 macOS、Rust stable 与可用的 Xcode Command Line Tools。编译和默认测试不需要模型权重。

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo run -p meetnote
```

CLI 使用 settings 中已选择并安装的 ASR 与 diarization 模型处理音频：

```bash
cargo run -p meetnote -- process /absolute/path/to/audio.m4a
```

当前运行目录：

```text
data/documents/                 Markdown + JSON 文档包
data/settings/settings.json     应用设置
data/speakers/library.json      Speaker Library
models/                         本地模型缓存
```

模型权重、音频样本、运行数据与构建产物不进入版本控制。

## 架构

```text
GPUI / CLI / Agent adapter
          │
          ▼
application ── document service / processing orchestration / identity
   │              │                         │
   ▼              ▼                         ▼
domain         persistence               runtime
contracts      settings/models           worker jobs
   │
   ├── ASR backend manager ── whisper-rs / sherpa-onnx
   └── diarization manager ── sherpa-onnx / native speakrs pipeline
```

文档包是项目结构的唯一事实来源，不使用 SQLite 或全局项目数据库。短音频走整文件 pipeline；长音频走重叠的有界离线窗口，并在录音级完成 speaker 聚类和身份匹配。详细边界见[架构总览](docs/architecture/overview.md)。

## 文档索引

- [完整文档索引](docs/index.md)
- [架构总览](docs/architecture/overview.md)
- [模型管理](docs/architecture/model-management.md)
- [文档与音频项目模型](docs/architecture/document-model.md)
- [模型后端系统](docs/architecture/backend-system.md)
- [架构决策](docs/architecture/decisions.md)
- [工程实施计划](docs/plan/implementation-plan.md)
- [GPUI UI 计划](docs/plan/ui-plan.md)
