# 文档与音频项目模型

## 项目定义

MeetNote 的项目是由音频事实派生的 AI 可读文档，不是固定的会议记录。根文档可以覆盖一整段录音；Assistant 或外部 Agent 可以按 Skill 将其拆成最多两级子文档，因此整棵树最多三层。

MeetNote 负责保存和校验事实，Agent 负责判断语义。音频必须先完成完整 ASR、diarization 与对齐，语义拆分不能截断或替代原始结果。

## 存储架构

```text
data/
├── documents/
│   └── <document-id>/
│       ├── document.md
│       └── state.json
├── settings/settings.json
└── speakers/library.json
models/
└── <task>/<model-id>/...
```

- `document.md`：面向用户和 Agent 的正文。
- `state.json`：身份、音频来源、层级、工作流状态、当前处理记录和当前处理结果。
- `settings`、模型缓存和 Speaker Library 是独立运行资产，不是项目树事实。
- 当前音频导入保存源文件路径，不复制音频；音频资产托管和丢失恢复属于生产化计划。

文档目录是唯一事实来源。列表、查询视图和项目树通过扫描重建，文件变化通过 watcher 触发刷新；不得增加 SQLite、全局 project store 或无法从文档恢复的权威索引。

## `DocumentState`

```text
DocumentState
├── schema
│   ├── schema_version
│   ├── revision
│   └── updated_at_ms
├── identity
│   ├── id
│   └── created_at_ms
├── source
│   ├── audio_path
│   ├── audio_asset_id
│   ├── recorded_at_ms
│   └── audio_start_ms / audio_end_ms
├── hierarchy
│   ├── parent_id
│   ├── depth
│   └── sibling_order
├── document_status
├── content
│   ├── title
│   ├── tags
│   └── summary
├── processing
│   ├── latest
│   └── last_error
└── results
    ├── transcript
    ├── diarization
    ├── speaker_segments
    └── markdown_path
```

`revision` 是写回并发控制版本；通过 `DocumentCommandService` 更新内容、状态或关系时必须携带预期 revision。`processing` 和 `results` 由 MeetNote 维护，普通文档 patch 不能覆盖它们。

## 音频处理顶层流程

```text
导入音频路径
  -> 创建根文档
  -> model/stage preflight
  -> JobRuntime 创建 ASR 与 diarization 任务
  -> 短音频：whole-file pipeline
     长音频：有界离线窗口 pipeline
  -> 标准化 ASR 与 diarization 结果
  -> 录音级匿名 speaker 聚类
  -> Speaker Library identity matching
  -> transcript/speaker alignment
  -> 更新 processing.latest 与 results
  -> 生成 document.md
  -> 保存文档包并触发目录刷新
```

处理失败只更新 `processing.last_error`，不得删除既有结果或改变 `document_status`、标题、标签、总结和正文。重新处理成功后替换 `processing.latest` 与当前 `results`；如需保留另一套结果，应创建另一个文档项目。

### 长音频

达到 30 分钟或无法预估长度时启用离线分窗：连续解码为 16 kHz mono，以 60 秒 valid core 和前后各 15 秒 context 构造窗口。ASR 负责模型族特有的拼接；native diarization 保存窗口 inference artifact，在全录音范围执行一次匿名 speaker 聚类，再裁剪 valid core、匹配 Speaker Library 并对齐。

窗口、局部 speaker track、VAD、embedding observation 都是一次处理任务的技术状态，不能成为文档节点或语义拆分依据。完整后端规则见[模型后端系统](backend-system.md)。

## 处理记录

`processing.latest` 只描述当前结果：

```text
ProcessingRecord
├── processing_revision / completed_at_ms
├── input_audio_checksum
├── asr: ModelProvenance
├── diarization
│   ├── backend_id / model_revision
│   ├── segmentation / embedding
│   ├── clustering
│   └── identity_matching
├── alignment
└── runtime
    ├── device / language / sample_rate_hz
    ├── threads / expected_speakers
    └── windowing
```

`results.transcript` 保存 ASR 时间段，`results.diarization` 保存匿名 cluster 与可选 profile 标识，`results.speaker_segments` 保存对齐后的 speaker-labelled transcript。语义正文可以引用这些结果，但不能代替它们。

## 三层项目树

```text
根文档                     depth 0
└── 语义子文档             depth 1
    └── 语义子文档         depth 2
```

MeetNote 写入关系时必须拒绝：

- 第四层。
- 自身父节点或循环。
- 不存在的父节点。
- 不同 `audio_asset_id` 之间的父子关系。
- 非法或空的子文档时间范围。

扫描外部修改时检查目录 ID、schema、重复 ID、父节点、循环、来源、声明深度和三层上限。无效文档保留为可定位 issue，不得静默生成另一棵树。

拖动到目标内部表示 reparent；拖动到目标前后表示同级 reorder。关系写回必须更新文档 JSON，而不是只修改 UI 内存。跨多个文档的事务一致性、严格的时间范围包含关系和结构化内部链接仍属于生产化计划。

## 子项目分割标准

语义分割只由 Assistant 或外部 Agent 按 Skill 发起：

1. 读取根文档完整 transcript、diarization、speaker alignment 和当前 revision。
2. 决定语义边界、标题、标签、总结与父节点。
3. 为子文档提供非空时间范围，继承相同音频路径与根 `audio_asset_id`。
4. MeetNote 从父文档复制与时间范围相交的 transcript、diarization 和 speaker segments。
5. MeetNote 校验 revision、来源、循环和三层上限后保存并刷新项目树。

声学静音、speaker 切换、ASR 窗口边界和 embedding cluster 都不能直接等同于语义边界。

## `document_status`

`document_status` 是持久化的知识工作流状态，当前为非空字符串，默认值是 `inbox`。具体值由用户或 Skill 约定，例如整理中、待复核、定稿或归档；MeetNote 不固定状态枚举，也不根据状态自动执行语义工作。

它与处理任务严格分离：

- `JobRuntime::JobState`：一次运行中的 queued/running/completed/failed/cancelled。
- `processing.latest`：处理成功后的模型、算法和运行来源。
- `processing.last_error`：最近一次处理失败信息。
- `document_status`：长期文档工作流状态。

更新 `document_status` 使用与正文相同的 revision 检查。处理成功、失败、取消和重新处理都不得重置它。

## Assistant 与外部 Agent

Assistant 是内置 Agent 客户端，外部 Agent 是可替换客户端；两者必须使用相同文档权限和校验规则。

```text
Skill + Agent
  -> 读取 Markdown/JSON 与 revision
  -> 形成查询或写回意图
  -> application/document service
  -> schema、revision、来源与三层校验
  -> 文档包保存
  -> watcher 刷新派生树
```

`DocumentCommandService` 提供 list、read、create、update、create semantic child、reparent 和 reorder 的内部应用边界，但不是外部 Agent 必须绑定的 ABI。CLI、IPC 或其他 adapter 必须复用相同 service；不得直接改写 MeetNote 管理的 processing/results 字段。

当前 Assistant 只通过 document service 读取列表并显示占位反馈，外部 Agent adapter 尚未实现。接入进度见[工程实施计划](../plan/implementation-plan.md)。
