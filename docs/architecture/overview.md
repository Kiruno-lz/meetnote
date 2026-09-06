# 系统架构总览

## 产品边界

MeetNote 是 Rust-native、本地优先的音频处理与 AI 可读文档工作区。音频可以是一整天的记录、会议、访谈或其他资料，不进入固定的会议领域模型。

MeetNote 负责：

- 音频解码、ASR、speaker diarization、Speaker Library 匹配和结果对齐。
- Markdown/JSON 文档包的扫描、读取、写回、版本校验和错误报告。
- 从文档声明实时派生并维护最多三层的项目树。
- 为 GPUI、CLI、Assistant 和外部 Agent 提供同一 application/document service 边界。

Assistant 或外部 Agent 按 Skill 决定语义边界、标题、标签、总结和父子关系。MeetNote 不内置 LLM、自动语义拆分器或另一份项目数据库。

## 顶层数据流

```text
音频路径
  -> 解码为 16 kHz mono
  -> ASR + diarization
  -> Speaker Library identity matching
  -> transcript/speaker alignment
  -> document.md + state.json
  -> 目录扫描/监听
  -> 最多三层的项目树
  -> GPUI / Assistant / external Agent
```

短音频使用整文件 pipeline。达到 30 分钟或无法预估长度时，application 使用 60 秒 valid core 与前后各 15 秒 context 的离线窗口；ASR 在 backend 内拼接，native diarization 汇总窗口 artifact 后执行录音级聚类。完整规则见[模型后端系统](backend-system.md)和[文档与音频项目模型](document-model.md)。

## Workspace 分区

```text
apps/meetnote/                    进程装配、CLI 与 GPUI 启动
crates/
├── domain/                       领域模型、校验与纯算法
├── contracts/                    跨层请求、结果、能力与错误结构
├── application/                  用例编排与稳定服务边界
├── audio/                        解码、重采样、chunk 与窗口规划
├── runtime/                      任务、进度、取消与 worker gate
├── persistence/                  Markdown/JSON 文档包与目录监听
├── settings/                     版本化设置与原子 JSON 保存
├── models/                       catalog、下载、安装与 probe 调度
├── asr/                          ASR registry 与 manager
├── asr-whisper-rs/               whisper.cpp backend
├── asr-sherpa-onnx/              sherpa-onnx ASR backend
├── diarization/                  diarization registry 与 manager
├── diarization-sherpa-onnx/      sherpa-onnx diarization backend
├── diarization-native/           speakrs 三阶段与分窗 backend
├── llm/                          可选 Agent/provider 描述边界
└── ui/                           GPUI 视图、交互与状态投影
resources/models/catalog.json     模型资产清单
third_party/speakrs/              固定版本的 native diarization 实现
```

## 分区边界

### `domain`

只包含 `DocumentState`、音频来源、处理结果、三层关系、speaker segment、对齐和校验。不得依赖 GPUI、文件系统、runtime 或具体推理库。

### `contracts`

定义版本化文档命令、`ModelRef`、backend descriptor、ASR/diarization 请求与归一化结果、embedding space、speaker profile 和统一错误码。具体 backend 字段只能进入明确的 metadata 或 provenance。

### `application`

编排文档读写、模型 preflight、长短音频处理、ASR/diarization 并发、对齐、Speaker Library 和 processing provenance。只通过 trait 选择后端与 repository，不依赖 GPUI。

### `audio`

负责音频 demux/decode、downmix、连续重采样、整数 sample offset、取消检查、整文件读取和有界窗口。技术窗口不是领域文档，也不决定语义边界。

### `runtime`

提供任务 ID、状态、进度、取消、查询和阻塞 worker 并发门。处理任务是进程内状态，不写入文档事实。生产化并发约束见[工程实施计划](../plan/implementation-plan.md)。

### `persistence`

以 `document.md + state.json` 保存文档，通过目录扫描重建列表和项目树，通过文件监听触发刷新。无效外部修改必须形成可定位 issue。完整格式见[文档与音频项目模型](document-model.md)。

### `settings`

保存 UI locale、backend/model 选择、设备、转录语言、speaker 数提示、Speaker Library 路径和 diarization stage 配置；不保存项目事实。

### `models`

读取 catalog，管理模型可用性、HTTP Range 下载、暂停/继续/取消、校验、原子安装、任务恢复和下载后 probe。完整协议见[模型管理](model-management.md)。

### `asr` 与 `diarization`

分别维护 registry/manager，通过 provider trait 向 application 返回后端。两类后端独立选择和组合，具体接口见[模型后端系统](backend-system.md)。

### 具体 backend

每个 backend 独立处理模型族的文件布局、预处理、推理、后处理、设备能力和窗口拼接。新增 backend 不修改 domain、application 或 UI contract。

### `llm`

只保留可选 provider descriptor/trait。项目树、拆分、总结、标签和查询规则属于 Skill 与文档服务，不进入此 crate。

### `ui`

GPUI 只发送 application command、订阅事件和投影视图状态，不直接执行文件 IO、模型下载、推理或语义判断。当前完成度见[GPUI UI 计划](../plan/ui-plan.md)。

## 依赖原则

```text
domain <- contracts <- application <- adapters/backends/UI
                         |
                         +-> audio/runtime traits

persistence/models/settings 实现 application 或独立服务边界
apps/meetnote 只负责组装具体实现
```

- 单一 Rust 进程，不启动 Python subprocess。
- 文档目录是项目结构的唯一事实来源。
- ASR 与 diarization 输出必须先归一化，再由 application 对齐和保存。
- Assistant 与外部 Agent 不得绕过文档版本、来源和三层约束。
- 原始音频引用、原始转录、原始 diarization 与 processing provenance 不得被语义编辑覆盖。

## 当前实现边界

核心 Rust workspace、文档包、模型管理、四个真实 backend、GPUI 功能脚手架和 native 长音频链路已经落地。当前未完成的是具体 Agent/provider、完整文档编辑 UI、文档多文件/多文档事务、长音频资源上限验收、精度基准和 macOS 发布。进度与验收顺序统一记录在[工程实施计划](../plan/implementation-plan.md)。
