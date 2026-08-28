# Rust-native 系统架构

## 目标

meetnote 是一个本地优先的音频转录与语音资料整理工作台。会议只是其中一种场景：

```text
导入音频资产
  -> ASR
  -> speaker diarization
  -> transcript/speaker 对齐
  -> 语义场景拆分（可选）
  -> 项目树与内部链接
  -> LLM 整理、总结和索引
```

处理顺序固定为先完成整段音频转录，再进行语义分析，最后判断是否需要整理和拆分；拆分不能替代完整转录。

桌面应用由单一 Rust 进程提供。GPUI 是 UI 层，application layer 是业务编排层，推理后端通过 Rust trait 接入。

## 顶层数据流

```text
┌──────────────────────┐
│ GPUI UI              │
│ views / actions      │
└──────────┬───────────┘
           │ typed commands/events
┌──────────▼───────────┐
│ application          │
│ use cases / pipeline │
└────┬─────────┬───────┘
     │         │
┌────▼────┐ ┌──▼──────────┐
│ domain  │ │ JobRuntime  │
│ rules   │ │ jobs/events │
└─────────┘ └──┬──────────┘
                │
      ┌─────────┼─────────┐
      │         │         │
┌─────▼────┐ ┌──▼─────┐ ┌─▼────────┐
│ ASR      │ │ DIA    │ │ LLM      │
│ manager  │ │manager │ │ manager  │
└─────┬────┘ └──┬─────┘ └─┬────────┘
      │         │         │
┌─────▼─────────▼─────────▼───────┐
│ concrete Rust/native backends    │
│ whisper-rs / sherpa / ONNX/etc. │
└──────────────────────────────────┘
```

## Workspace

```text
apps/meetnote/
crates/
├── domain/
├── contracts/
├── application/
├── runtime/
├── persistence/
├── settings/
├── models/
├── asr/
├── asr-whisper-rs/
├── asr-sherpa-onnx/
├── diarization/
├── diarization-sherpa-onnx/
├── diarization-native/
├── llm/
└── ui/
resources/models/catalog.json
```

`crates/` 下去除重复的 `meetnote-` 前缀。具体后端使用任务和实现名称区分。

## 层职责

### domain

纯 Rust 领域模型和算法：音频资产、项目节点、UUID、父子层级、时间范围、分类标签、转录段、speaker 段、人工修正、对齐、文档版本和领域错误。

### contracts

统一的版本化请求、结果、进度、能力、模型引用和错误结构。所有 backend 必须转换成 contracts 类型后才能进入 application。

### application

用例编排：导入、转录、分离、对齐、语义拆分、项目整理、人工修正、LLM 索引、保存和搜索。只依赖接口，不依赖 GPUI 或具体推理库。

### runtime

任务调度、取消、进度、日志、并发限制、模型 session 生命周期和恢复状态。

### persistence

v0.2 当前使用原子替换的 JSON project store 保存可编辑项目状态；文件系统保存音频、模型和 Markdown 产物。不引入 SQLite，也不使用 localStorage。

### models

模型目录、解析、下载、断点续传、SHA-256 校验、原子安装、缓存索引和 backend probe。

### settings

版本化设置 schema、JSON 原子持久化、运行时设置快照与更新；不使用 SQLite。

### ui

GPUI Application、Window、Entity、View、Action 和交互状态投影。UI 不直接调用 backend 或 repository。

## 关键边界

```text
domain 不知道 backend
application 只知道 backend trait
backend 只负责模型加载和推理
UI 只发送 command 和消费 event
```

ASR 和 diarization 相互独立，最终由 `AlignmentService` 合并：

```text
ASR segments + diarization segments
          -> AlignmentService
          -> speaker-tagged transcript
```

## 持久化原则

JSON project store 是当前可编辑项目树和内容的运行数据源，Markdown 是用户可读和可迁移的文档产物。主项目通过内部 Markdown 链接引用子项目，不复制子项目全文。每次生成结果时记录：

- 使用的 ASR backend、model 和 revision
- 使用的 diarization backend、model 和 revision
- speaker 修正版本
- 去口语化 provider、model 和参数
- 原始转录和最终文档之间的来源关系

不可逆处理不得覆盖原始转录。
