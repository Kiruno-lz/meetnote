# GPUI UI 规划

## 产品第一性

meetnote 首先是本地音频转录与语音资料整理工具，会议只是当前最明确的一种使用场景。系统的核心对象不是 `Meeting`，而是：

```text
音频资产
  -> 原始转录
  -> 语义项目树
  -> 项目摘要、标签和索引
```

一个音频可以对应一个明确项目，也可以对应多个按语义场景拆出的子项目：

```text
全天录音
├── 上午会议
├── 午休闲谈
└── 下午复盘
    └── 项目决策段
```

小型且语义单一的录音不强制拆分。拆分是整理手段，不是转录前置条件。
处理顺序必须是先完成整段音频转录，再自动进行语义分析，最后由语义分析结果判断是否需要整理和拆分。语义拆分不能截断或替代完整转录。

每个音频项目或语义子项目至少需要：

- 唯一 UUID。
- 所属音频资产 UUID。
- 父项目 UUID（根项目为空）。
- 创建时间和录音时间。
- 音频时间范围。
- 标题。
- 分类标签。
- 原始转录引用。
- 总结文引用。
- 子项目顺序和层级信息。

层级最大为三层。主项目的内容不重复承载所有子项目正文，而是通过内部 Markdown 链接索引子项目。
主项目承担对子项目的整理、总结和索引；子项目承担具体语义场景的独立内容。

## UI 责任

UI 只负责：

- 展示音频项目树和项目内容。
- 在原始文与总结文之间切换。
- 发起语义拆分、整理和 LLM 索引任务。
- 将拖动操作转换为项目树变更 command。
- 管理悬浮详情、设置表单和短生命周期的助手输入。

UI 不负责音频解码、语义切分、LLM 调用、模型下载、项目持久化或领域校验。

## 信息架构

```text
meetnote
├── Assistant                 默认首页：一次性语音库助手输入
├── Meetings                  音频项目树与整理工作区
│   └── Meeting detail        悬浮详情面板，不发生页面跳转
├── Models                    模型目录与下载任务
└── Settings                  backend、模型、设备和界面语言
```

`Meetings` 是历史命名，后续领域文档和代码应逐步使用 `AudioProject`、`ProjectNode` 或等价中性名称。UI 可以保留 Meetings 作为用户熟悉的导航文案，但不能把数据模型限制为会议。

## 默认首页：Assistant

默认首页采用类似 Open WebUI 的单次输入工作台：

```text
┌──────────────────────────────────────────────────────────────┐
│ meetnote                    Assistant  Meetings  Models  ⚙   │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│                         meetnote                             │
│                 Ask your voice library                       │
│                                                              │
│       ┌──────────────────────────────────────────────┐       │
│       │ Ask about your recordings...             ↑   │       │
│       └──────────────────────────────────────────────┘       │
│                                                              │
│        [整理语音库]   [查找一次会议]   [拆分一个项目]          │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

首页输入是一次性请求，不保存对话历史，也不建立 conversation/session history。提交后可以展示当前结果，但离开首页后不把问答追加到历史列表。

输入请求的目标可以是：

- 查询语音库中的事实或原文片段。
- 要求 LLM 整理、归类或生成项目摘要。
- 要求对一个大型音频项目提出语义拆分建议。
- 跳转到相关音频项目或子项目。

查询和整理结果可以引用项目 UUID 与时间范围，但不改变原始转录。需要写入项目树或摘要的操作必须经过明确的确认 command。

## Meetings 工作区

### 总体布局

```text
┌──────────────────────────────────────────────────────────────┐
│ meetnote       Assistant  Meetings  Models  Settings   task │
├──────────────────┬───────────────────────────────────────────┤
│ AUDIO PROJECTS   │ 项目树工作区                              │
│                  │                                           │
│ + Import audio   │ 项目标题 / 时间 / 标签                    │
│                  │ [Summary] [Original] [Split] [Organize]   │
│ ▼ 全天录音       │                                           │
│   ├ 上午会议     │ 当前层级项目内容                          │
│   ├ 午休闲谈     │                                           │
│   └ 下午会议     │                                           │
│                  │ [打开详情]                                │
└──────────────────┴───────────────────────────────────────────┘
```

左侧导航展示项目树，右侧展示当前选中节点的内容。项目详情不通过 route 跳转替换主界面，而是通过悬浮详情面板打开。

### 项目树规则

每个树节点显示：

- 标题。
- 时间范围或日期。
- 分类标签。
- 当前内容模式：总结文或原始文。
- 处理状态。
- 子项目数量。

层级规则：

- 根音频项目为第 1 层。
- 语义子项目最多到第 3 层。
- 不允许形成循环。
- 移动一个节点时同时移动其完整子树。
- 如果目标位置会使子树超过第 3 层，拒绝放置并说明原因。

### 拖动重排

拖动树节点左侧的具体项目图标，不拖动整行文本作为隐式操作。放置目标分三种：

```text
拖到目标上方     -> 调整为目标同级、排在目标之前
拖到目标下方     -> 调整为目标同级、排在目标之后
拖到目标节点     -> 调整为目标子项目
```

拖动结束后先生成预览状态，再提交：

```text
ReparentProject(node_id, new_parent_id, sibling_index)
```

领域层负责验证最大深度、循环和跨音频资产移动；UI 不自行判断并写入树结构。

### 项目内容

每个层级的项目都必须有可展示内容。内容区默认显示总结文：

```text
┌──────────────────────────────────────────────────────────────┐
│ 上午会议                                                     │
│ 2026-08-28 09:00–10:00   #work #planning                    │
│                                                              │
│ [Summary]  [Original]                         [Edit]         │
├──────────────────────────────────────────────────────────────┤
│ 本次会议确定了发布范围、负责人和下一步时间点。               │
│                                                              │
│ 相关子项目：                                                 │
│ [接口决策]  [待办确认]                                      │
└──────────────────────────────────────────────────────────────┘
```

规则：

- 首次打开项目时显示总结文。
- `Summary` 与 `Original` 是同一项目的两个展示投影，不是两个项目。
- 原始文必须保留 speaker、时间戳和原始文本。
- 总结文可以由 LLM 生成和编辑，但不能覆盖原始文。
- 主项目总结文中使用内部链接引用子项目，不复制子项目全文。
- 总结未生成时显示明确空状态，不用原文冒充总结文。

## Meeting detail 悬浮面板

Meeting detail 保留为完整的详情视图，但不作为页面跳转目标。

```text
┌──────────────────────── Meetings workspace ──────────────────┐
│ project tree                         main content             │
│                                      ┌──────────────────────┐ │
│                                      │ Meeting detail    ×  │ │
│                                      │ title / metadata    │ │
│                                      │ audio range         │ │
│                                      │ Summary / Original  │ │
│                                      │ speaker transcript  │ │
│                                      │ [Split] [Organize]  │ │
│                                      └──────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

悬浮面板要求：

- 保留底层项目树上下文。
- 支持关闭、Esc 关闭和点击背景关闭。
- 面板内部滚动，不改变主窗口 route。
- 处理任务运行时关闭面板不取消任务。
- 从助手结果点击项目链接时打开对应详情面板。

详情面板内容包括：

- 项目元数据：UUID、时间范围、标题、标签、父项目和子项目。
- 音频播放定位信息。
- 处理状态和 ASR/diarization 进度。
- 总结文/原始文切换。
- speaker 修正。
- 语义拆分建议及确认操作。
- LLM 整理、重生成和索引状态。

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

设置页编辑 `AppSettings`，保存通过 `SettingsService::update` 完成。

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

## LLM 与语音库索引

LLM 不是首页聊天历史的替代品，而是语音库的整理和索引服务：

```text
原始转录
  -> 语义场景候选
  -> 用户确认或修改边界
  -> 子项目标题、标签和总结
  -> 项目内部链接
  -> 语音库索引
```

UI 需要显示：

- LLM 当前任务阶段。
- 待确认的语义拆分建议。
- 每个建议的时间范围、标题、标签和摘要预览。
- 接受、修改、拒绝操作。
- 已索引/待索引/索引失败状态。

任何会改变项目树、总结文或标签的 LLM 结果都必须经过用户确认；纯查询结果可以短生命周期显示，不写入对话历史。

## UI 状态模型

```text
WorkspaceUiState
├── active_area: Assistant | Meetings | Models | Settings
├── projects: ProjectTreeState
│   ├── roots
│   ├── expanded_nodes
│   ├── selected_node
│   └── drag_preview
├── detail_overlay: Option<ProjectDetailState>
├── content_mode: Summary | Original
├── processing: ProcessingQueueState
├── downloads: DownloadQueueState
├── llm_tasks: LlmTaskState
├── settings: SettingsFormState
├── locale: LocaleState
└── notice: NoticeState
```

状态规则：

- 项目树、详情面板和全局任务互相独立。
- 关闭详情面板不清除项目选择和后台任务。
- 内容模式按项目保存 UI 偏好，但默认值永远是 `Summary`。
- 拖动只修改本地预览，成功提交后再刷新 canonical project tree。
- 表单草稿与已保存设置分离。
- render 不执行 IO；controller 是唯一的 service 调用入口。
- 后台任务只保存 ID、最新快照和取消句柄，不保存 backend 内部对象。

## Command 与 Event 边界

### UI command

```text
SubmitAssistantRequest(text)
ImportAudio(path)
SelectProject(project_id)
OpenProjectDetail(project_id)
CloseProjectDetail
SetProjectContentMode(project_id, mode)
ReparentProject(node_id, new_parent_id, sibling_index)
SplitProject(project_id, ranges)
AcceptSemanticSplit(proposal_id)
RejectSemanticSplit(proposal_id)
StartProjectProcessing(project_id)
CancelProcessing(project_id)
StartModelDownload(model_id)
PauseDownload(download_id)
ResumeDownload(download_id)
CancelDownload(download_id)
UpdateSettings(AppSettings)
ResetSettings
```

### 后台 event

```text
ProjectTreeChanged(ProjectTree)
ProjectContentChanged(project_id, content_version)
ProcessingEvent(JobEvent)
DownloadEvent(DownloadEvent)
LlmTaskEvent(LlmTaskEvent)
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
├── events.rs              runtime/model/LLM event 适配
├── views/
│   ├── shell.rs           全局导航、任务摘要和页面容器
│   ├── assistant.rs       单次输入助手首页
│   ├── meetings.rs        项目树与内容区
│   ├── project_detail.rs  悬浮详情面板
│   ├── models.rs          模型浏览与下载队列
│   └── settings.rs        设置表单与 locale
└── components/
    ├── project_tree.rs
    ├── project_content.rs
    ├── transcript_view.rs
    ├── model_card.rs
    ├── progress_row.rs
    └── overlay.rs
```

不要建立一个同时处理树、详情、下载和设置的通用巨型 View。项目树操作、内容投影和详情悬浮层分别拥有清晰的输入状态。

## 实施顺序与当前状态

### UI-0：领域对象冻结

已完成 `MeetingProject` 向音频项目字段和语义子项目操作的第一步升级；后续仍需将领域类型正式重命名为中性的 `AudioProject`/`ProjectNode`。

验收：项目树可表达单项目和三层语义子项目；小型录音无需创建子项目；所有移动操作可由领域校验。

### UI-1：Assistant 首页与应用壳

已实现单次输入首页、全局导航、全局任务摘要和无历史空状态；LLM service 仍未接入。

验收：默认进入 Assistant；提交请求不产生对话历史；导航到 Meetings/Models/Settings 不丢失后台任务。

### UI-2：Meetings 项目树

已实现最多三层树展示、展开折叠、项目内容卡片、Summary/Original 切换和拖动建立父子关系；拖动前预览与同级前后排序仍需补齐。

验收：拖动图标可以排序和改变父级；非法深度和循环被拒绝；总结文为默认视图；详情仍未跳页。

### UI-3：悬浮详情与处理流程

已将现有会议详情迁移为 overlay，并绑定导入、处理、取消和 speaker 修正；Markdown 子项目链接与音频定位仍需在内容模型完成后接入。

验收：打开/关闭详情不改变主工作区；处理任务和下载任务不因 overlay 关闭而取消。

### UI-4：Settings 与 i18n 基础

已增加 UI locale 字段，与转录语言分离，并绑定 backend、模型快照、设备和语言设置；完整 i18n 字符串资源表仍需补齐。

验收：设置可保存、重启恢复；界面字符串通过 locale 查找；用户内容不被错误翻译。

### UI-5：Models 与 LLM 整理

已绑定模型目录、下载队列、暂停/继续/取消；语义拆分建议和语音库索引状态等待 LLM service 接入。

验收：模型任务全局持续；LLM 的树结构和文本变更必须显式确认；首页不产生聊天历史。

## 明确不纳入本轮

- 不引入 SQLite、localStorage 或 WebView。
- 不把应用定位锁定为会议记录软件。
- 不把主音频的所有子项目内容复制到主项目正文。
- 不把 LLM 输出未经确认直接写入项目树或总结文。
- 不在 UI 中直接调用 backend、repository、模型文件或网络客户端。
- 不在本规划确认前修改 `crates/ui` 实现。
-  不做LLM相关的功能实现，UI暂时只用于占位符
