# meetnote

MeetNote 是一个 Rust-native、本地优先的音频处理与 AI 可读文档后端。它负责把音频转换为带时间戳、说话人和来源关系的可靠文档资料，并向外部 Agent 提供读取、编辑、状态更新和文档保存能力。文档的内容可以是一次会议、一天的记录、一次访谈或其他任意音频资料；MeetNote 不根据内容类型替用户决定文档边界。

## 产品边界

MeetNote 的核心服务只有三类：

- 音频处理：管理 ASR 与 speaker diarization 模型，完成音频解码、转录、说话人识别和结果对齐。
- 文档管理：以目标目录下每份文档自身的 JSON 状态和内容文件为事实来源，提供扫描、读取、创建、编辑、状态更新、关系读取和保存，并实时维护由这些文档派生的语义项目树；项目树固定最多三层，不维护独立的权威数据库。
- Agent 接入：让任意外部 Agent 通过稳定的机器可读边界读取文档、编辑文档、更新状态和写入 Skill 定义的语义结构。

语义边界判断不属于 MeetNote 的自动推理职责。一天的录音是否拆成会议、闲谈和复盘，应该由 Agent 使用自己的 Skill、上下文和约束判断；Skill 指导 Agent 把语义项目树和其他状态记录到文档自身的 JSON/结构化内容中。MeetNote 会主动读取这些文档，校验三层上限，解析并实时维护语义项目树视图，但不凭空生成语义边界，也不以独立数据库保存第二份项目结构。

Assistant 页面是保留的内置 Agent 客户端，不是特殊业务层。它与任意外部 Agent 使用同一个文档查询/写回接口、相同权限和相同三层树约束；当前仍是占位 UI，尚未接入具体 Agent/provider。

## 当前状态

当前仓库已完成旧版 Tauri、TypeScript、Python 实现的清理，并已接入 Rust-native desktop workspace：GPUI Assistant 与项目树、音频导入、Rust 音频解码、后台 processing service、JSON/Markdown 结果保存、真实 `whisper-rs` ASR 和 `sherpa-onnx` diarization。以目标目录文档为事实来源的实时项目树仍是下一阶段的核心实现任务；当前 Assistant 是保留的占位客户端，不代表 MeetNote 已内置 LLM 服务。

## 目标架构

```text
GPUI UI / Assistant
  -> Rust application layer
  -> audio processing / document management / model managers
  -> Rust or native-library inference backends

External Agent / Skill
  -> machine-readable document commands and queries
  -> Rust application layer
```

项目不再依赖 Tauri、WebView、Python subprocess 或 Python 运行环境。

## 文档

- [架构总览](docs/architecture/overview.md)
- [ASR 与 diarization 后端管理层](docs/architecture/backend-system.md)
- [模型目录与下载协议](docs/architecture/model-management.md)
- [工程实施计划](docs/architecture/implementation-plan.md)
- [架构决策](docs/architecture/decisions.md)
- [GPUI UI 规划](docs/architecture/ui-plan.md)

## 后端方向

- ASR：`whisper-rs`、`sherpa-onnx`，后续可加入 Candle 或其他 native backend。
- Diarization：`sherpa-onnx`、`diarization` crate，后续可加入自定义 ONNX/CoreML pipeline。
- 后端通过统一 Rust trait 注册和选择，应用层不依赖具体推理实现。

LLM、语义拆分、自动总结、标签推断和语音库自然语言检索不作为 MeetNote 内置后端。它们由外部 Agent/Skill 完成，再把结果写回文档自身的内容或 JSON 状态。

## 本地处理示例

`meetnote` 默认启动 GPUI；传入 `process` 子命令可运行第一条真实 pipeline：

```bash
cargo run -p meetnote -- process \
  docs/example/2p_en_video.m4a \
  models/asr/whisper-large-v3-ggml/ggml-base.en.bin \
  models/diarization/sherpa-onnx-speaker-diarization-bundle \
  data/runs/2p_en_video
```

命令直接读取 M4A，并输出 JSON 和 Markdown；模型文件位于被 `.gitignore` 忽略的 `models/`，不会提交进仓库。

## 当前不执行

模型权重由用户按需放入本地 cache，不进入仓库。目标文档目录中的 Markdown 与 JSON 状态是权威数据源；MeetNote 从中生成并实时更新最多三层的项目树和查询索引，但不维护独立项目数据库，索引始终可以从文档重新扫描生成。语义项目拆分、LLM 整理与语音库索引由 Assistant 或外部 Agent 按 Skill 决定或写入，MeetNote 负责校验、读取和反映结果。SQLite 不纳入本项目架构。
