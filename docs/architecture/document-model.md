# 文档模型与处理记录

## 目的

MeetNote 的持久化单位是文档包，而不是数据库中的项目行。文档可以代表一整段录音、一天中的一个语义片段、一次会议、访谈或个人整理。文档包中的 Markdown 与 JSON 必须能够在没有 MeetNote 索引和数据库的情况下被读取、迁移和重新建立关系。

```text
documents/<document-id>/
├── document.md
└── state.json
```

`document.md` 保存 AI 可读的正文、总结、引用和内部链接；`state.json` 保存机器可读的身份、来源、层级、文档状态、当前处理记录和当前处理结果。派生索引和三层项目树只能从这些文件重建。

## `DocumentState` 的当前分层

当前 Rust `DocumentState` 已使用 v2 分层结构。旧的扁平 v1 只存在于反序列化迁移逻辑中，加载旧文件后会原子写回 v2；新的代码和文档不得继续增加扁平字段。

```text
DocumentState
├── schema
├── identity
├── source: AudioSource
├── hierarchy: DocumentHierarchy
├── document_status: DocumentStatus
├── content: DocumentContent
├── processing: ProcessingMetadata
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
- 输入音频 checksum：当前写入 `processing.latest.input_audio_checksum`，不以路径作为内容身份。

子文档当前必须保留根文档的 `audio_asset_id`；项目树保留最多三层。子文档时间范围与内部链接的精确约束列入 roadmap，当前不作为 application 写回的强校验。

### `hierarchy: DocumentHierarchy`

保存文档声明的关系：

- `parent_id`：可为空。
- `sibling_order`：同级稳定排序。
- `depth`：当前保存并检查声明值，自动派生和修复列入 roadmap。
- `internal_links`：正文或状态中的文档引用；结构化链接模型和目标校验列入 roadmap。

根节点为第 1 层，子文档最多到第 3 层。当前实现保留三层上限和基础关系错误报告；更强的循环、缺失父节点、来源范围和内部链接一致性校验列入 roadmap，不能在没有明确 schema 的情况下扩展为新的强约束。

### `document_status: DocumentStatus`

这是知识文档的持久化工作流状态，不是音频处理状态。字段已经存在，当前以可扩展字符串值保存，例如收件、整理中、待复核、已定稿或归档。其语义由用户或 Agent 按 Skill 约定，MeetNote 不固定枚举。

`document_status` 必须独立于 `processing`。处理完成后它仍然存在；重新处理音频不得把它重置为默认值；处理失败不得覆盖 Agent 已经写入的标题、标签、总结或正文。

当前缺口不是状态模型，而是写回边界：`DocumentPatch` 和内部 document service 需要提供显式的状态更新入口，并通过同一 revision 校验保存。状态迁移历史和状态值 schema 暂不属于当前版本。

### `content: DocumentContent`

保存语义整理结果的索引元数据：

- `title`
- `tags`
- `summary`
- `content_kind` 和结构化 `links` 尚未进入当前 v2 结构，列入 roadmap。

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

`ModelProvenance` 当前包含 `backend_id`、`model_id`、`revision`、已安装 artifact 的聚合 SHA-256、
解析后的 artifact 路径，以及逐 artifact 的期望 checksum、实际 checksum 和 archive member。
`RuntimeProvenance` 包含采样率、语言、设备、线程和 expected speakers 等影响结果的参数。

diarization 不强行假设有三个神经网络模型：segmentation 和 embedding 通常是模型，clustering 通常是 AHC/PLDA/VBx 算法及其参数；speaker identity matching 还需要记录 embedding space、profile library schema 和阈值。若某后端确实使用 clustering 模型，则在 `clustering` 中记录 `ModelProvenance`，否则记录 `AlgorithmProvenance`。

重新处理属于同一音频项目的再次生成。当前策略是直接替换 `processing.latest` 和当前处理结果，不保留旧处理记录或旧结果版本；如果用户需要另一套结果，应创建新的文档项目。处理记录只服务于当前结果的可追溯性。

### `results: ProcessingResults`

结果与语义内容分层保存：

- `transcript`：当前处理生成的 ASR 段及其时间戳。
- `diarization`：原始 speaker 段及 cluster/profile 标识。
- `speaker_segments`：应用层将 ASR 与 diarization 对齐后的结果。
- `markdown_path`：当前 Markdown 输出引用。

未来可以把结果拆到文档包内部的独立文件，但当前 v2 将结果内嵌在 `state.json`，语义总结和人工修正更新文档内容，不作为另一个处理版本保存。

## `processing_state` 与 `document_status`

`processing_state` 只描述一次音频处理任务的运行过程：

```text
queued -> running -> completed
                   ├-> failed
                   └-> cancelled
```

它属于 `JobRuntime` 的短生命周期状态。任务成功后，从运行时移除；文档只写入 `ProcessingRecord.completed_at` 和模型/运行元数据。任务失败或进程中断时，运行时报告失败事件，当前不会把未完成任务恢复成可继续的处理任务，也不得把它误当成文档工作流状态。

当前代码中的 `ProjectStatus::{Imported, Processing, Ready, Failed}` 已不再作为持久化字段；`processing_status()` 只根据 `processing.latest` 与 `processing.last_error` 为旧 UI 提供兼容投影。运行时任务状态应继续由 JobRuntime 提供。

目标分层由以下部分组成：

1. `JobRuntime` 的处理任务状态，负责队列、进度、取消、失败、完成、任务查询和阻塞 worker 并发限制。
2. `DocumentState.processing`，只负责当前已完成处理记录；当前写入 `latest`、输入 checksum、模型/算法来源和运行参数。
3. `DocumentState.document_status`，负责独立的知识文档工作流状态。

验收必须覆盖：成功处理后重启不自动重新处理；处理完成时间、ASR 模型、diarization stage、clustering 算法和运行参数可从 `state.json` 读取；处理失败不改变 `document_status`；重新处理后只保留当前结果。

## Skill-first Agent 边界

MeetNote 不把外部 Agent 锁定在一个强约束的 command ABI 上。Agent 的主协作方式是读取 Skill 指导，直接操作可读的 Markdown/JSON 文档包，再由 MeetNote 监听、校验和刷新派生视图。

Skill 必须明确：

- 文档包布局和可读字段。
- 哪些字段由 Agent/用户维护，哪些字段由 MeetNote 写入。
- 如何决定语义拆分、标题、标签、总结、父子关系和内部链接。
- 如何保留原始音频、ASR、diarization 和处理记录。
- 写回前如何使用 `revision`，写回后如何检查扫描错误。
- 创建子文档时如何满足同一音频来源和最多三层规则。

MeetNote 当前强制执行 schema、UUID、版本冲突、原子写入、三层上限，以及基础的循环、缺失父节点和跨音频来源检查。子文档时间范围、内部链接模型和更强的关系一致性规则列入 roadmap；它不强制执行 Skill 的语义字段枚举，也不判断 Agent 的总结质量。

当前 `application::DocumentCommandService` 保留为 GPUI 和未来适配器使用的内部应用边界。它不是外部 Agent 必须实现的公共协议；任何 CLI、IPC 或 Agent adapter 都必须最终复用同一 document service、同一权限和同一校验规则。

## 当前实现与 roadmap

已完成：分层 v2、旧扁平状态迁移、当前处理记录、文档状态字段、文档包原子保存和基础三层树。

已完成：catalog revision、逐 artifact checksum、diarization stage provenance、显式 document status 更新、JobRuntime 任务状态和下载后快速 backend probe。

roadmap：结果外部文件化、内部链接模型、子文档时间范围约束、深度自动派生、强关系一致性校验。旧扁平结构只存在于读取迁移逻辑中。
