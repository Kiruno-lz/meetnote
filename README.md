# MeetNote

MeetNote 是 Rust-native、本地优先的音频处理与 AI 可读文档工作区。

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


                 GPUI UI
          交互、状态投影、任务反馈
                    │
      ┌─────────────┼─────────────┐
      ▼             ▼             ▼
Workspace Host  Audio Host   Agent Host
文档、文件、     解码、模型、    capability 绑定、
结构校验、恢复   provider、处理  薄适配、最小 loop

详细边界见[架构总览](docs/architecture/overview.md)

## 文档索引

- [完整文档索引](docs/index.md)
- [架构总览](docs/architecture/overview.md)
- [模型管理](docs/architecture/model-management.md)
- [文档状态与音频项目模型](docs/architecture/document-state.md)
- [Audio Host 与模型后端](docs/architecture/audio-host.md)
- [架构决策](docs/architecture/decisions.md)
- [工程实施计划](docs/plan/implementation-plan.md)
- [GPUI UI 计划](docs/plan/ui-plan.md)
