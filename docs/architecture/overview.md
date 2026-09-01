# Rust-native 系统架构

## 目标

MeetNote 是一个本地优先的音频处理与 AI 可读文档后端。它不假设一份文档必须是一次会议，也不替 Agent 判断一天的录音应当如何按语义拆分；它会持续读取目标目录中的文档信息，主动解析并维护固定最多三层的语义项目树：

```text
导入音频资产
  -> ASR
  -> speaker diarization
  -> transcript/speaker 对齐
  -> 可追溯的原始文档资料
  -> 文档内容、JSON 状态和处理结果保存
  -> 外部 Agent 读取或写回文档
```

处理顺序固定为先完成整段音频转录和说话人对齐，再由 Assistant 或外部 Agent 使用 Skill 进行语义理解。语义拆分、标题、标签和总结不是 MeetNote 的自动推理功能。Agent 把结果记录到文档自身的 JSON/结构化内容中，MeetNote 实时读取、校验三层上限并更新项目树。树是文档的派生视图，不是第二份事实来源。

桌面应用由单一 Rust 进程提供。GPUI 是 UI 层，application layer 是业务编排层，推理后端通过 Rust trait 接入。

## 顶层数据流

```text
┌────────────────────┐   ┌────────────────────┐
│ GPUI Assistant     │   │ External Agent     │
│ built-in client    │   │ Skill-driven client│
└─────────┬──────────┘   └─────────┬──────────┘
          └────────────┬───────────┘
                       │ same document service and events
             ┌─────────▼──────────┐
             │ Rust application   │
             │ processing/docs/tree│
             └─────┬────────┬─────┘
                   │        │
        ┌──────────▼──┐  ┌──▼──────────┐
        │ target docs │  │ JobRuntime  │
        │ md + json   │  │ jobs/events │
        └──────┬──────┘  └──┬──────────┘
               │ scan/watch   │
        ┌──────▼──────────┐   ├── ASR manager
        │ derived project │   └── DIA manager
        │ tree, max depth 3│
        └─────────────────┘
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
└── ui/
resources/models/catalog.json
```

`crates/` 下去除重复的 `meetnote-` 前缀。具体后端使用任务和实现名称区分。

当前工作区中的 `crates/llm/` 是占位适配边界，只允许为 Assistant 或外部 Agent 接入可选 provider；它不拥有项目树、拆分、总结或索引规则，也不改变 Assistant 与外部 Agent 的同层关系。

## 层职责

### domain

纯 Rust 领域模型和算法：音频资产、文档、最多三层的项目树派生视图、UUID、父子关系、时间范围、分类标签、转录段、speaker 段、人工修正、对齐、处理状态和领域错误。domain 解释文档声明的结构并校验三层上限、循环和来源一致性，但不替 Agent 判断语义边界。

### contracts

统一的版本化请求、结果、进度、能力、模型引用和错误结构。所有 backend 必须转换成 contracts 类型后才能进入 application。

### application

用例编排：导入、转录、分离、对齐、目标目录扫描/监听、文档读取和写回、项目树刷新、处理状态更新、人工修正、保存和基于文档信息的查询。只依赖接口，不依赖 GPUI、具体 Agent 或推理库；不实现 LLM 调用和语义边界推理。

### runtime

任务调度、取消、进度、日志、并发限制、模型 session 生命周期和恢复状态。

### persistence

文件系统保存音频、文档内容和模型资产；每份文档自身的 Markdown 与 JSON 状态是权威数据源。MeetNote 扫描或监听目标目录，实时维护文档索引和最多三层的项目树；索引与树必须可丢弃、可重建，不能替代文档 JSON。v0.2 不引入 SQLite、全局项目数据库或 localStorage。

### models

模型目录、解析、下载、断点续传、SHA-256 校验、原子安装、缓存索引和 backend probe。

### settings

版本化设置 schema、JSON 原子持久化、运行时设置快照与更新；不使用 SQLite。

### ui

GPUI Application、Window、Entity、View、Action 和交互状态投影。Assistant 是保留的内置 Agent 客户端，与外部 Agent 共用 controller/application 文档接口；UI 不直接调用 backend 或 repository。

## 关键边界

```text
domain 不知道 backend 和具体 Agent/provider
application 只知道 backend/document storage/tree projection trait
backend 只负责模型加载和推理
Assistant 与外部 Agent 使用相同 document read/write 边界
项目树只从目标目录文档派生，固定最多三层
UI 只发送 command 和消费 event
```

ASR 和 diarization 相互独立，最终由 `AlignmentService` 合并：

```text
ASR segments + diarization segments
          -> AlignmentService
          -> speaker-tagged transcript
```

## Assistant 与外部 Agent 的知识后端边界

MeetNote 的知识后端能力分为“文档管理/写回”和“实时项目树”两部分：

- 查询：Assistant 或外部 Agent 可以按文档 ID、项目树位置、文档 JSON 字段、时间范围和来源信息读取文档元数据、原始转录、说话人对齐结果和 Markdown 内容；MeetNote 不为 Skill 定义固定的语义字段。
- 编辑：Assistant 或外部 Agent 可以按 Skill 约定写回标题、标签、正文、文档状态、父子关系、内部链接和人工 speaker 修正。
- 拆分：Agent/Skill 自行决定是否拆分、拆成什么文档以及语义项目树如何记录；MeetNote 提供文档创建、原子保存、三层约束校验和树刷新，不提供自动语义拆分算法。
- 实时维护：目标目录发生新增、修改、移动或删除后，MeetNote 必须刷新派生项目树；通过 MeetNote 写入时拒绝超过三层、循环或跨音频来源的结构，外部无效修改在扫描后显示可定位错误。
- 追溯：MeetNote 保存文档版本、原始资料引用和处理结果；原始音频、原始 ASR 和原始 diarization 结果不可被覆盖。

Agent 接入不绑定某个模型、厂商或 Skill，也不把某个 command enum 作为外部 Agent 的强制 ABI。首要边界是 Skill 指导下的 Markdown/JSON 文档包读写；CLI、IPC 或其他适配器都必须复用同一 document service，不拥有额外权限。当前 `DocumentCommandService` 只是 application 内部的版本校验与用例入口。语义字段和拆分规则属于 Skill；三层上限和结构一致性属于 MeetNote domain contract。详细字段分层见[文档模型与处理记录](document-model.md)。

## 文档包与事实来源

MeetNote 管理的是文档包，而不是一张集中式项目表。最小文档包由内容文件和状态文件组成：

```text
documents/<document-id>/
├── document.md       # AI 可读正文
└── state.json        # 该文档的权威状态
```

`state.json` 按 `schema`、`identity`、`source`、`hierarchy`、`document_status`、`content`、`processing` 和 `results` 分层。父子关系、拆分关系、标签、总结和其他语义字段由 Skill 定义并记录在这里；MeetNote 解析这些字段，维护最多三层的项目树，但不为它们建立第二份权威持久化结构。当前代码使用分层 v2 `DocumentState`，旧扁平 v1 只存在于读取时的兼容迁移逻辑中。

文档管理服务必须扫描或监听 `documents/`，生成文档列表、过滤、查询索引和最多三层的项目树。索引或树丢失、过期、或被 Assistant/外部 Agent 修改后，必须通过增量刷新或完整重扫恢复；任何缓存都不能成为文档状态的事实来源。

## 两类状态

系统处理状态与知识文档状态必须分开：

- `processing_state`：由 `JobRuntime` 管理，表示一次音频任务的排队、运行、完成、失败或取消。任务成功后不作为长期项目状态保留。
- `processing`：处理完成后写入文档 JSON，保存完成时间、输入指纹、ASR 模型、diarization segmentation/embedding 模型、clustering 算法、identity matching 和运行参数。重处理追加历史记录。
- `document_status`：由外部 Agent 或用户按 Skill 写入文档自身 JSON，表示文档处于收件、编辑、复核、定稿或归档等知识工作流阶段；MeetNote 负责保存和展示，但不解释这些语义值。

Agent 写入文档状态不得伪造或覆盖 MeetNote 的 `processing` 记录；处理失败也不得删除 Agent 已经编辑的文档内容或改变 `document_status`。当前 `ProjectStatus` 只作为旧 UI 的兼容投影，不能继续承载新的持久化语义。

## 持久化原则

文档是面向 Agent 的知识单元，Markdown 是稳定、可读、可迁移的内容表达，文档自身 JSON/结构化状态是该文档的管理事实来源。MeetNote 不维护独立的权威项目数据库；文档列表、筛选、查询和最多三层的项目树只能从目标目录文档生成。文档可以代表一天、一次会议或其他任意语义单元。文档之间的父子关系、标签和内部链接由 Assistant 或外部 Agent 按 Skill 写入文档状态，MeetNote 负责校验、保存并实时更新派生树。完整 pipeline 成功后写入处理记录：

- 使用的 ASR backend、model 和 revision
- 使用的 diarization backend、segmentation/embedding model 和 revision
- clustering 算法或模型、identity matching embedding space、阈值和 profile library schema
- speaker 修正版本
- 外部 Agent/Skill 标识及变更说明（如果由 Agent 写入）
- 原始转录和最终文档之间的来源关系

不可逆处理不得覆盖原始转录；语义结果由 Agent 作为文档内容、JSON 状态或新的文档文件保存。当前 pipeline 已将 `processing.latest`、处理历史、ASR/diarization backend 与 model、对齐算法、运行参数和可计算的输入/artifact checksum 写回 `state.json`；catalog 提供的固定 checksum 传播和更细的 backend stage provenance 仍是后续补强项。
