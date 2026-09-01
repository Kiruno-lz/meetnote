# 文档模型与处理记录

## 目的

MeetNote 的持久化单位是文档包，而不是数据库中的项目行。文档可以代表一整段录音、一天中的一个语义片段、一次会议、访谈或个人整理。文档包中的 Markdown 与 JSON 必须能够在没有 MeetNote 索引和数据库的情况下被读取、迁移和重新建立关系。

```text
documents/<document-id>/
├── document.md
└── state.json
```

`document.md` 保存 AI 可读的正文、总结、引用和内部链接；`state.json` 保存机器可读的身份、来源、层级、文档状态、处理记录和原始结果引用。派生索引和三层项目树只能从这些文件重建。

## `DocumentState` 的目标分层

当前 Rust `DocumentState` 仍把身份、层级、内容、处理状态和推理结果放在一个结构中，这是 v0.2 的兼容性过渡，不能作为长期领域模型。目标模型按以下值对象拆分：

```text
DocumentState
├── schema
├── identity
├── source: AudioSource
├── hierarchy: DocumentHierarchy
├── document_status: DocumentStatus
├── content: DocumentContentMetadata
├── processing: ProcessingRecord
└── results: ProcessingResults
```

### `schema`

保存状态文件 schema 版本和文档版本：

- `schema_version`：状态文件结构版本；不随普通编辑递增。
- `revision`：文档包的并发控制版本；每次原子写回递增。
- `updated_at`：最近一次写回时间。

### `identity`

保存不可变或受严格校验的标识：

- `document_id`：文档 UUID。
- `created_at`：文档包创建时间。
- `title`、`tags`、`summary` 等语义字段属于 `content`，不属于身份。

### `source: AudioSource`

描述文档来自哪一个音频及其时间范围：

- `audio_asset_id`：同一份原始音频的稳定 UUID；主文档以自身 UUID 作为默认来源标识。
- `audio_path`：原始音频路径或项目内相对路径。
- `recorded_at`：录音发生时间，可为空。
- `start_ms`、`end_ms`：文档在原始音频中的范围；根文档可以覆盖全长。
- `audio_checksum`：处理记录需要的输入指纹，不以路径作为内容身份。

子文档必须保留根文档的 `audio_asset_id`，并且时间范围必须落在父文档范围内。跨音频挂接由 domain 拒绝。

### `hierarchy: DocumentHierarchy`

保存文档声明的关系：

- `parent_id`：可为空。
- `sibling_order`：同级稳定排序。
- `depth`：派生并校验的值，不应由 Agent 任意指定。
- `internal_links`：正文或状态中的文档引用；引用目标必须经过存在性和来源校验。

根节点为第 1 层，子文档最多到第 3 层。循环、缺失父节点、跨音频来源和第四层必须产生可定位错误；不能静默修正为另一棵树。

### `document_status: DocumentStatus`

这是知识文档的持久化工作流状态，不是音频处理状态。它可以表示收件、整理中、待复核、已定稿或归档等值。其语义由用户或 Agent 按 Skill 约定，MeetNote 只负责保存、版本校验、展示和结构校验。

`document_status` 必须独立于 `processing`。处理完成后它仍然存在；重新处理音频不得把它重置为默认值；处理失败不得覆盖 Agent 已经写入的标题、标签、总结或正文。

### `content: DocumentContentMetadata`

保存语义整理结果的索引元数据：

- `title`
- `tags`
- `summary`
- `content_kind`（可选，由 Skill 定义，不由 MeetNote 固定枚举）
- `links` 和其他 Skill 扩展字段

MeetNote 不判断标题、标签、总结和拆分边界是否“语义正确”。它只校验 JSON 结构、关系安全、版本和文件写入原子性。

### `processing: ProcessingRecord`

处理成功后只保留完成记录和不可变的运行来源，不保留一个需要持续维护的“正在处理项目状态”：

```text
ProcessingRecord
├── processing_revision
├── completed_at
├── input_audio_checksum
├── asr: ModelProvenance
├── diarization
│   ├── segmentation: ModelProvenance
│   ├── embedding: ModelProvenance
│   ├── clustering: AlgorithmProvenance
│   └── identity_matching: IdentityMatchingProvenance
├── alignment: AlgorithmProvenance
└── runtime: RuntimeProvenance
```

`ModelProvenance` 至少包含 `backend_id`、`model_id`、`revision`、已安装 artifact 的 SHA-256、输入格式和设备。`RuntimeProvenance` 包含采样率、语言、设备 provider、线程/批处理等影响结果的参数。

diarization 不强行假设有三个神经网络模型：segmentation 和 embedding 通常是模型，clustering 通常是 AHC/PLDA/VBx 算法及其参数；speaker identity matching 还需要记录 embedding space、profile library schema 和阈值。若某后端确实使用 clustering 模型，则在 `clustering` 中记录 `ModelProvenance`，否则记录 `AlgorithmProvenance`。

对于重新处理，旧的 `ProcessingRecord` 不应被无痕覆盖。目标 schema 采用 `processing_history` 追加完成记录，并把最新记录投影为 `processing.latest`；原始 ASR、原始 diarization 和对齐结果必须保留版本或引用，语义文档编辑不能覆盖它们。

### `results: ProcessingResults`

结果与语义内容分层保存：

- `raw_transcript`：原始 ASR 段及其时间戳。
- `diarization`：原始 speaker 段及 cluster/profile 标识。
- `aligned_transcript`：应用层将两者对齐后的规范化结果。
- `storage`：结果内嵌或外部文件引用及其 checksum。

目标模型允许结果从 `state.json` 拆到文档包内部的独立文件，但迁移前必须保持从文档包重建同一结果。语义总结和人工修正不能替换原始结果。

## `processing_state` 与 `document_status`

`processing_state` 只描述一次音频处理任务的运行过程：

```text
queued -> running -> completed
                   ├-> failed
                   └-> cancelled
```

它属于 `JobRuntime` 的短生命周期状态。任务成功后，从运行时移除；文档只写入 `ProcessingRecord.completed_at` 和模型/运行元数据。任务失败或进程中断时，运行时可以留下可恢复的失败记录，但不得把它误当成文档工作流状态。

当前代码中的 `ProjectStatus::{Imported, Processing, Ready, Failed}` 已不再作为持久化字段；`processing_status()` 只根据 `processing.latest` 与 `processing.last_error` 为旧 UI 提供兼容投影。运行时任务状态应继续由 JobRuntime 提供。

目标分层由以下部分组成：

1. `JobRuntime` 的 `ProcessingJobState`，负责队列、进度、取消、恢复和错误。
2. `DocumentState.processing`，只负责已完成处理记录和历史；当前已写入 `latest`、`history`、输入 checksum、模型/算法来源和运行参数。
3. `DocumentState.document_status`，负责独立的知识文档工作流状态。

迁移验收必须覆盖：成功处理后重启不重新处理；处理完成时间、ASR 模型、diarization segmentation/embedding 模型、clustering 算法和运行参数可从 `state.json` 读取；处理失败不改变 `document_status`；旧结果和新处理记录可追溯。

## Skill-first Agent 边界

MeetNote 不把外部 Agent 锁定在一个强约束的 command ABI 上。Agent 的主协作方式是读取 Skill 指导，直接操作可读的 Markdown/JSON 文档包，再由 MeetNote 监听、校验和刷新派生视图。

Skill 必须明确：

- 文档包布局和可读字段。
- 哪些字段由 Agent/用户维护，哪些字段由 MeetNote 写入。
- 如何决定语义拆分、标题、标签、总结、父子关系和内部链接。
- 如何保留原始音频、ASR、diarization 和处理记录。
- 写回前如何使用 `revision`，写回后如何检查扫描错误。
- 创建子文档时如何满足同一音频来源和最多三层规则。

MeetNote 必须强制执行：schema、UUID、版本冲突、原子写入、三层上限、循环、缺失父节点、跨音频来源和结果不可覆盖。它不强制执行 Skill 的语义字段枚举，也不判断 Agent 的总结质量。

当前 `application::DocumentCommandService` 保留为 GPUI 和未来适配器使用的内部应用边界。它不是外部 Agent 必须实现的公共协议；任何 CLI、IPC 或 Agent adapter 都必须最终复用同一 document service、同一权限和同一校验规则。

## 迁移顺序

1. 为 `DocumentState` 建立 `Schema`、`Identity`、`AudioSource`、`Hierarchy`、`DocumentStatus`、`ContentMetadata`、`ProcessingRecord` 和 `ProcessingResults` 值对象。
2. 提供旧扁平 `state.json` 到新结构的一次性读取迁移，并以原子写回新 schema；迁移失败时保留原文件并显示错误。
3. 将处理任务状态移入 `JobRuntime`，让 persistence 只保存完成记录和历史。
4. 在完整 pipeline 成功后写回处理来源和模型运行元数据；每次重处理追加历史记录。
5. 在 application/persistence 测试中覆盖迁移、并发写回、处理中断、重启恢复和完整 provenance 读取。

前四项已落地基础版本；剩余工作是将 catalog checksum、backend stage 元数据和原始结果版本引用继续补齐。旧扁平结构只存在于读取迁移逻辑中，新的 UI 和 Agent 文档不得再扩展旧字段。
