# GPUI UI 规划

## 产品第一性

MeetNote UI 是本地音频处理与 AI 可读文档项目后端的客户端。文档可以代表一天、一次会议、一次访谈或其他任意内容，UI 不根据内容类型建立固定的会议模型，但必须保留固定最多三层的项目树以控制复杂度。

```text
音频资产
  -> 原始转录
  -> 说话人识别与对齐
  -> 文档内容 + 文档自身 JSON 状态
  -> 实时三层项目树
  -> Assistant / 外部 Agent 读写
```

复合录音的场景边界、标题、标签和总结由 Assistant 或外部 Agent 使用 Skill 决定，并记录在文档自身的 JSON/结构化内容中。MeetNote 主动扫描目标目录、解析这些关系并实时维护最多三层的项目树；它不自动推断拆分边界，但负责结构校验、树展示、显式重排和错误反馈。

处理顺序必须是先完成整段音频转录和说话人对齐，再由外部 Agent 进行语义处理。语义拆分不能截断或替代完整转录。

MeetNote 文档管理需要读取和保存：

- 文档稳定标识、来源音频和时间范围。
- Markdown 或其他 AI 可读内容。
- 文档自身 JSON 状态，包括 Agent/Skill 定义的语义字段、父子关系和同级顺序。
- MeetNote 管理的处理状态、版本和原始结果引用。

文档目录和文档 JSON 是事实来源，不建立独立的项目数据库。UI 使用从目标目录实时派生的三层项目树；关闭、索引丢失或应用重启后必须能够从文档恢复。

## UI 责任

UI 只负责：

- 展示最多三层的项目树、文档内容和文档自身 JSON 状态。
- 在原始文与总结文之间切换。
- 将拖动、编辑和状态操作转换为文档 JSON 写回 command。
- 管理悬浮详情、设置表单和保留的 Assistant 客户端。

UI 不负责音频解码、语义切分、LLM 推理、模型下载或 Agent 决策。三层限制、循环和来源一致性由 domain/application 校验，UI 只显示结果。

## 信息架构

```text
meetnote
├── Assistant                 保留的内置 Agent 客户端
├── Meetings/Documents        实时三层项目树与文档工作区
│   └── Document detail       悬浮详情面板，不发生页面跳转
├── Models                    模型目录与下载任务
└── Settings                  backend、模型、设备和界面语言
```

`Meetings` 可以作为用户熟悉的导航文案保留，但领域模型必须使用中性的文档/项目类型。Assistant 和项目工作区都是正式入口，不是迁移后要删除的过渡页面。

## Assistant：内置 Agent 客户端

Assistant 页面长期保留，但自身不拥有特殊的 LLM 业务层。它可以连接任意 Agent/provider，负责发送文档查询或写回请求；它与外部 Agent 使用同一 document service、相同权限、相同版本校验和相同三层树约束。当前页面仍为占位客户端。

```text
┌──────────────────────────────────────────────────────────────┐
│ meetnote             Assistant  Meetings  Models  Settings  │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│                         meetnote                             │
│                 Work with your documents                     │
│                                                              │
│       ┌──────────────────────────────────────────────┐       │
│       │ Send a document request...               ↑   │       │
│       └──────────────────────────────────────────────┘       │
│                                                              │
│        [读取文档]     [编辑文档]       [更新状态]              │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

输入请求只能通过统一 Agent/document command 映射为文档读取、编辑、状态更新或 Skill 驱动的拆分建议。Agent 写回文档后，MeetNote 从目标目录实时刷新三层项目树；Assistant 不维护自己的项目副本。

## Meetings/Documents 工作区

### 总体布局

```text
┌──────────────────────────────────────────────────────────────┐
│ meetnote     Assistant  Meetings  Models  Settings      task │
├──────────────────┬───────────────────────────────────────────┤
│ PROJECT TREE    │ 文档项目工作区                            │
│                  │                                           │
│ + Import audio   │ 文档标题 / 时间 / 状态                    │
│                  │ [Summary] [Original] [Edit]               │
│ ▼ 全天录音       │ 当前文档内容                              │
│   ├ 上午会议     │                                           │
│   └ 下午复盘     │                                           │
│      └ 决策段    │                                           │
│                  │ [打开详情]                                │
└──────────────────┴───────────────────────────────────────────┘
```

左侧展示从目标目录文档和 JSON 状态实时派生的三层项目树，右侧展示当前文档。文档详情不通过 route 跳转替换主界面，而是通过悬浮详情面板打开。

### 项目树规则

每个树节点显示：

- 标题。
- 时间范围或日期。
- 分类标签。
- 文档自身 JSON 中的状态摘要。
- MeetNote 的 ASR/diarization 处理状态。
- 子项目数量。

层级规则：

- 根音频项目为第 1 层。
- 子项目最多到第 3 层；三层上限是正式产品特性。
- 不允许循环、缺失父节点或跨音频资产挂接。
- 移动节点时同时移动完整子树。
- 通过 MeetNote 写入的非法操作必须拒绝；外部 Agent 直接写入的非法关系必须在扫描后标记错误，不得静默生成第四层。

拖动放置仍支持目标上方、目标下方和目标内部三种位置。成功操作必须原子更新相关文档 JSON，再由目录刷新结果驱动 UI；不能只修改内存树。

### 文档内容

每个文档都必须有可展示内容。内容区默认显示总结文：

```text
┌──────────────────────────────────────────────────────────────┐
│ 全天录音                                                     │
│ 2026-08-28 09:00–10:00   #work #planning                    │
│                                                              │
│ [Summary]  [Original]                         [Edit]         │
├──────────────────────────────────────────────────────────────┤
│ 文档正文由外部 Agent/Skill 写入，内容格式保持 AI 可读。       │
│                                                              │
│ JSON 状态：status / source / relations / agent metadata      │
└──────────────────────────────────────────────────────────────┘
```

规则：

- 首次打开文档时显示总结文。
- `Summary` 与 `Original` 是同一文档的两个展示投影，不是两个项目。
- 原始文必须保留 speaker、时间戳和原始文本。
- 文档内容和 JSON 状态可以由 Agent/Skill 编辑，但不能覆盖原始处理结果。
- 项目树关系和总结字段由文档自身 JSON 记录，MeetNote 负责校验、保存、实时解析和展示。

## Document detail 悬浮面板

Document detail 保留为完整的详情视图，但不作为页面跳转目标。

```text
┌──────────────────── Meetings/Documents workspace ───────────┐
│ project tree                        main content             │
│                                      ┌──────────────────────┐ │
│                                      │ Document detail  ×  │ │
│                                      │ title / metadata    │ │
│                                      │ audio range         │ │
│                                      │ Summary / Original  │ │
│                                      │ speaker transcript  │ │
│                                      │ [Edit JSON]         │ │
│                                      └──────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

悬浮面板要求：

- 保留底层三层项目树上下文。
- 支持关闭、Esc 关闭和点击背景关闭。
- 面板内部滚动，不改变主窗口 route。
- 处理任务运行时关闭面板不取消任务。
- 从 Assistant 或外部 Agent 结果中的文档引用打开对应详情面板。

详情面板内容包括：

- 文档元数据：UUID、时间范围、标题、标签和来源。
- 音频播放定位信息。
- 处理状态和 ASR/diarization 进度。
- 总结文/原始文切换。
- speaker 修正。
- 文档 JSON 状态查看和编辑。
- Agent 写入的版本与变更来源。

## Models

模型页面消费 `ModelManager` 的 catalog 和任务事件，不直接读取文件或启动下载。

模型条目显示：

- 模型名称、任务、候选 backend、格式和 license。
- `Missing`、`Partial`、`Installed` 状态。
- 下载中的 artifact、进度和错误。
- `Pause`、`Resume`、`Cancel`。
- 是否需要认证，以及认证缺失原因。

模型下载任务是全局任务，切换到其他页面后仍继续；顶部只显示摘要，完整队列在 Models 页面展开。

## Settings

设置页是单列、表单式的应用配置页，不使用模型卡片、横向按钮组或说明卡片承载设置项。页面使用与主窗口相同的深色背景，内容从窗口顶部标题栏下方开始，标题为 `Settings`，标题下方不追加页面说明段落。

每个设置分组由一个左对齐的分组标题、若干设置行和分组底部分隔线组成。设置行必须保持同一高度和同一左右内边距：设置名称左对齐，当前值位于右侧固定宽度的选择器内，选择器显示当前值和向下箭头。未打开选择器时，所有分组只显示当前值；点击选择器后，在当前行下方展开选项，选项宽度与选择器一致，点击选项后立即收起。

页面的精确排列顺序如下，不得因为后端是否可用而改变分组顺序：

1. `Interface`
   - `Language / Interface locale`
2. `Transcription`
   - `ASR backend`
   - `ASR model`
   - `Transcription language`
3. `Speaker separation`
   - `Diarization backend`
   - `Diarization model`
   - `Expected speakers`
4. `Runtime`
   - `Compute device`

页面底部固定显示操作栏：左侧为 `Reset`，右侧为 `Save settings`。选择器修改的是页面内的 `AppSettings` 草稿，不得在每次点击选项时直接写入持久化存储；点击 `Save settings` 才调用 `SettingsService::update`。`Reset` 将草稿恢复为 `AppSettings::default()` 并同步界面显示，但仍通过操作栏明确保存结果。

页面内容超过窗口可视区域时，只有设置内容区滚动，标题和底部操作栏保持可见。滚动区域必须使用独立的 GPUI `ScrollHandle`，不能依赖窗口整体溢出。

组件验收标准：

- `Settings` 标题、四个分组和操作栏在所有语言下保持相同顺序。
- 每一行的标签左对齐，选择器右对齐且宽度一致；不允许用多行说明文字改变行高。
- 关闭状态下每个选择器只显示“当前值 + 向下箭头”。
- ASR 与 diarization 后端选择器只展示各自任务支持的 backend；模型选择器只展示当前 backend 已安装且兼容的模型。
- 语言选择器至少提供 `简体中文` 与 `English`，并区分 UI 语言与 ASR 转录语言。
- 修改后未保存时必须有可见的未保存状态；保存成功或重置成功后必须有可见反馈。
- 在 1180×760 的窗口中，页面首屏应显示标题、`Interface`、`Transcription` 和 `Speaker separation` 的开头；继续滚动可以看到 `Runtime` 与操作栏。

```text
┌──────────────────────────────────────────────────────────────┐
│ Settings                                                     │
├──────────────────────────────────────────────────────────────┤
│ Interface                                                    │
│ Language / Interface locale [简体中文                 ▼]     │
├──────────────────────────────────────────────────────────────┤
│ Transcription                                                │
│ ASR backend              [whisper-rs                 ▼]       │
│ ASR model                [whisper-base-en-ggml       ▼]       │
│ Transcription language   [Auto                       ▼]       │
├──────────────────────────────────────────────────────────────┤
│ Speaker separation                                           │
│ Diarization backend      [sherpa-onnx                ▼]       │
│ Diarization model        [speaker bundle             ▼]       │
│ Expected speakers        [Auto]                                │
├──────────────────────────────────────────────────────────────┤
│ Runtime                                                       │
│ Compute device            [Auto                       ▼]       │
│                                                              │
│ [Reset]                                      [Save settings]  │
└──────────────────────────────────────────────────────────────┘
```

`Interface locale` 与 `Transcription language` 是两个不同字段：前者控制 UI i18n，后者控制 ASR。i18n 第一阶段只覆盖导航、按钮、状态、错误和空状态，不把用户生成的转录或总结文本翻译成界面语言。

## Agent 写回与文档状态

拆分边界、标题、标签、总结和索引由 Assistant 或外部 Agent 按 Skill 决定，并记录在文档自身的 JSON/结构化内容中。MeetNote 负责把这些关系解析为实时三层项目树；Assistant 与外部 Agent 的结果使用同一预览、确认和写回流程。UI 负责：

- 展示文档 JSON 中已有的状态和关系字段。
- 展示 Agent 写回后的文档版本与变更来源。
- 通过通用文档编辑操作修改内容或 JSON 状态。
- 展示三层项目树刷新、结构冲突和无效外部修改。
- 展示 MeetNote 自己管理的 ASR、diarization 和对齐处理状态。

任何改变文档内容、项目树关系、总结或标签的 Agent 结果都必须作为通用文档写回明确确认；确认后原子更新文档，项目树由目录变化重新派生。纯查询结果不建立 MeetNote 的对话历史。

## UI 状态模型

```text
WorkspaceUiState
├── active_area: Assistant | Meetings | Models | Settings
├── projects: ProjectTreeState
│   ├── roots
│   ├── expanded_nodes
│   ├── selected_node
│   ├── validation_errors
│   └── source_revision
├── detail_overlay: Option<DocumentDetailState>
├── content_mode: Summary | Original
├── assistant: AgentClientState
├── processing: ProcessingQueueState
├── downloads: DownloadQueueState
├── settings: SettingsFormState
├── locale: LocaleState
└── notice: NoticeState
```

状态规则：

- 项目树、Assistant、详情面板和全局任务互相独立。
- 关闭详情面板不清除文档选择和后台任务。
- 内容模式按文档保存 UI 偏好，但默认值永远是 `Summary`。
- 项目树来自目标目录扫描/监听；索引丢失后必须能够从文档 JSON 恢复，且不得出现第四层。
- Assistant 只保存短生命周期请求/结果，不保存独立项目副本。
- 表单草稿与已保存设置分离。
- render 不执行 IO；controller 是唯一的 service 调用入口。
- 后台任务只保存 ID、最新快照和取消句柄，不保存 backend 内部对象。

## Command 与 Event 边界

### UI command

```text
ImportAudio(path)
ScanDocuments
SubmitAssistantRequest(text)
SelectProject(project_id)
OpenDocumentDetail(document_id)
CloseDocumentDetail
SetDocumentContentMode(document_id, mode)
UpdateDocumentContent(document_id, expected_version, content)
UpdateDocumentJsonState(document_id, expected_version, json_state)
ReparentProject(project_id, new_parent_id, sibling_index, expected_version)
ApplyAgentDocumentChange(change_set, expected_versions)
StartDocumentProcessing(document_id)
CancelProcessing(document_id)
StartModelDownload(model_id)
PauseDownload(download_id)
ResumeDownload(download_id)
CancelDownload(download_id)
UpdateSettings(AppSettings)
ResetSettings
```

### 后台 event

```text
DocumentsScanned(DocumentList)
DocumentChanged(document_id, content_version)
ProjectTreeChanged(ProjectTreeSnapshot)
ProjectTreeValidationFailed(document_id, error)
AssistantResult(AgentResult)
ProcessingEvent(JobEvent)
DownloadEvent(DownloadEvent)
SettingsChanged(AppSettings)
ServiceError(UiError)
```

GPUI action handler 只创建 command 或调用 controller；controller 负责调用 service、启动后台订阅，并将结果回送到 `WorkspaceUiState`。

## 目标代码拆分

当前实现已将导航和 UI 状态拆出；后续继续按以下边界拆分视图和 controller：

```text
crates/ui/src/
├── lib.rs                 GPUI application 启动和根 view
├── state.rs               WorkspaceUiState 与页面状态
├── route.rs               顶部区域导航；详情不使用 route
├── controller.rs          command 到 service 的异步协调
├── events.rs              runtime/model event 适配
├── views/
│   ├── shell.rs           全局导航、任务摘要和页面容器
│   ├── assistant.rs       内置 Agent 客户端
│   ├── meetings.rs        三层项目树与文档内容区
│   ├── document_detail.rs 悬浮详情面板
│   ├── models.rs          模型浏览与下载队列
│   └── settings.rs        设置表单与 locale
└── components/
    ├── project_tree.rs
    ├── document_content.rs
    ├── transcript_view.rs
    ├── model_card.rs
    ├── progress_row.rs
    └── overlay.rs
```

不要建立一个同时处理 Assistant、项目树、详情、下载和设置的通用巨型 View。Agent 客户端、项目树、内容投影和详情悬浮层分别拥有清晰的输入状态。

## 实施顺序与当前状态

### UI-0：领域对象冻结

当前代码使用中性的 `DocumentState` 和三层项目树；项目树从目标目录文档 JSON 实时派生，三层上限保留在 domain。

验收：文档可从自身 JSON 恢复；扫描索引丢失后可重建；第四层、循环和跨音频来源关系被拒绝或标记错误。

### UI-1：文档客户端与应用壳

已实现应用壳和 Assistant 占位入口。Assistant 作为内置 Agent 客户端长期保留，后续接入统一 document service，不增加独立项目状态。

验收：默认进入 Assistant；Assistant 与外部 Agent 使用相同权限、版本检查和三层树约束；导航到 Meetings/Models/Settings 不丢失后台任务。

### UI-2：实时三层项目树与内容

现有代码已实现三层项目树、展开折叠、重排和内容卡片；后续将树的数据源改为目标目录文档扫描/监听，并展示文档内容和 JSON 状态。

验收：文档可打开、编辑、保存和重新扫描恢复；项目树实时更新、最多三层，外部非法关系有明确错误。

### UI-3：悬浮详情与处理流程

已将现有会议详情迁移为 overlay，并绑定导入、处理、取消和 speaker 修正；后续绑定文档 JSON 状态、内容版本、树来源 revision 和 Agent 变更来源。

验收：打开/关闭详情不改变主工作区；处理任务和下载任务不因 overlay 关闭而取消。

### UI-4：Settings 与 i18n 基础

已增加 UI locale 字段，与转录语言分离，并绑定 backend、模型快照、设备和语言设置；导航、按钮、状态、错误和空状态已覆盖中文与英文。

验收：设置可保存、重启恢复；界面字符串通过 locale 查找；用户内容不被错误翻译。

### UI-5：Models 与处理状态

已绑定模型目录、下载队列、暂停/继续/取消；后续只补充 ASR/diarization 处理状态和文档写回结果，不接入内置 LLM。

验收：模型任务全局持续；文档 JSON 是状态事实来源；Assistant 或外部 Agent 写回后，三层项目树可以实时刷新并展示。

## 明确不纳入本轮

- 不引入 SQLite、localStorage 或 WebView。
- 不把应用定位锁定为会议记录软件。
- 不移除固定三层项目树；MeetNote 必须主动解析和维护文档驱动的派生树。
- 不在 MeetNote 内实现 LLM、自动拆分、自动总结、自动标签或语音库索引。
- 不在 UI 中直接调用 backend、repository、模型文件或网络客户端。
- 不在本规划确认前修改 `crates/ui` 实现。
- Assistant 与外部 Agent 写入的项目结构必须保存在文档自身 JSON/结构化内容中，UI 负责通过统一接口读取、预览、确认和保存。
