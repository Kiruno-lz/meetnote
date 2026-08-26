# GPUI UI 规划

## 目标

UI 是 Rust-native meeting intelligence platform 的交互层，只负责：

- 展示 application、settings 和 model manager 的状态。
- 将用户操作转换为明确的 command。
- 订阅后台任务事件并投影为可读状态。
- 管理页面导航、选中项和临时表单状态。

UI 不负责音频解码、模型下载、后端选择、项目持久化或领域算法。

本规划只定义界面和绑定边界，不实现 GPUI 组件。

## 当前 UI 基线

当前 `crates/ui/src/lib.rs` 已具备：

- GPUI 原生窗口和基础两栏布局。
- 项目列表、音频导入和项目选择。
- 处理任务启动、取消和状态事件展示。
- 基础 speaker 合并操作。
- 模型 readiness 展示和旧版一次性下载入口。

当前需要在 UI 重构阶段处理：

- `RootView` 同时承担导航、业务状态、文件 IO 调度和渲染，必须拆分。
- 模型下载入口仍调用旧的同步函数，必须改为绑定 `ModelManager` 任务事件。
- 模型和 backend 仍由 `DesktopServices` 固定注入，必须改为读取和更新 `AppSettings`。
- 当前 UI 只适合单一工作区，不能承载模型库和设置页面。

## 信息架构

```text
meetnote
├── Meetings                 默认页面：项目列表与最近会议
│   └── Meeting detail       单个会议的处理与 transcript
├── Models                   模型目录、安装状态和下载任务
└── Settings                 backend、模型、设备和语言设置
```

顶部保留全局导航和任务提示；项目详情不打开独立业务窗口，使用同一 GPUI window 内的 route 切换。

## 页面规划

### Meetings

默认进入会议工作区。

```text
┌──────────────────────────────────────────────────────────────┐
│ meetnote                 Meetings   Models   Settings   task │
├──────────────┬───────────────────────────────────────────────┤
│ MEETINGS     │ Welcome / selected meeting                    │
│              │                                                 │
│ + Import     │ [title]                         [status]       │
│              │ audio path                                      │
│ meeting A    │                                                 │
│ meeting B    │ [Process meeting]                              │
│              │                                                 │
│              │ transcript / processing state                  │
└──────────────┴───────────────────────────────────────────────┘
```

项目列表只显示标题、最近状态和处理中的阶段。项目详情显示：

- 标题和音频文件信息。
- `Imported`、`Processing`、`Ready`、`Failed` 状态。
- ASR 和 diarization 的独立进度。
- 可取消的处理按钮。
- speaker-labelled transcript。
- speaker 合并和后续单段修正入口。
- Markdown 产物路径和打开/复制操作。

### Meeting detail

```text
┌──────────────────────────────────────────────────────────────┐
│ ← Meetings    meeting title                  [Process/Cancel] │
│ audio path · duration · backend/model summary                │
├──────────────────────────────────────────────────────────────┤
│ ASR              ███████████░░  82%                          │
│ Diarization      ███████░░░░░  running                       │
├──────────────────────────────────────────────────────────────┤
│ speaker-0   00:00:00.000                                      │
│ original transcript text                                      │
│                                                              │
│ speaker-1   00:00:06.140                                      │
│ original transcript text                         [edit]       │
└──────────────────────────────────────────────────────────────┘
```

处理中的 transcript 区域显示阶段状态，不伪造未完成结果。处理完成后显示 canonical project 中的对齐结果。

### Models

模型页消费 `ModelManager::browse` 和 `ModelManager::list`，不直接读取 catalog 文件。

```text
┌──────────────────────────────────────────────────────────────┐
│ Models                                      [refresh]         │
│ Task [All]   Backend [All]   State [All]   Search             │
├──────────────────────────────────────────────────────────────┤
│ Whisper Base English                         Installed        │
│ whisper-rs · GGML · Apache-2.0                               │
│ [Use in settings]                                            │
├──────────────────────────────────────────────────────────────┤
│ Speaker diarization bundle                  Missing          │
│ sherpa-onnx · ONNX · MIT                                     │
│ [Download]                                                   │
├──────────────────────────────────────────────────────────────┤
│ Download queue                                                  │
│ model · artifact 1/2     ██████░░░░  60%     [Pause] [Cancel] │
└──────────────────────────────────────────────────────────────┘
```

模型卡片必须显示：

- 模型名称、任务、候选 backend、格式和 license。
- `Missing`、`Partial`、`Installed` 状态。
- 下载中的 artifact、已下载字节数、总字节数（如果来源提供）。
- `Pause`、`Resume`、`Cancel` 操作。
- 需要认证的模型必须显示原因，不能在 UI 中要求通过命令行传 token。

下载事件通过 `DownloadTask.events` 进入 UI；UI 只负责订阅、取消订阅和状态投影。

### Settings

设置页编辑 `AppSettings`，保存动作通过 `SettingsService::update` 完成。

```text
┌──────────────────────────────────────────────────────────────┐
│ Settings                                                     │
├──────────────────────────────────────────────────────────────┤
│ Transcription                                                │
│ ASR backend              [whisper-rs                 ▼]       │
│ ASR model                [whisper-base-en-ggml       ▼]       │
│ Language                 [Auto                       ▼]       │
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

选择器只展示已注册且与模型兼容的 backend/model 组合。未安装模型可以被选择并保存，但处理按钮必须在 probe 前明确提示模型缺失。

## UI 状态模型

UI 使用一个根级 `WorkspaceUiState` 作为页面协调状态，各页面只拥有自己的局部表单和过滤状态。

```text
WorkspaceUiState
├── route: Route
├── projects: ProjectListState
├── selected_project: Option<ProjectId>
├── processing: Option<ProcessingState>
├── downloads: DownloadQueueState
├── settings: SettingsState
├── model_browser: ModelBrowserState
└── notice: NoticeState
```

页面状态的规则：

- 服务返回的数据进入 state 后再 render，render 不执行 IO。
- 长任务只保存 `job_id`/`download_id` 和最新快照，不保存 backend 内部对象。
- route 切换不清除后台任务；返回页面时从服务重新读取当前状态。
- 表单状态与已保存 settings 分离，避免输入未保存时影响正在运行的会议。
- 错误必须带来源和可操作建议，不能只显示裸字符串。

## Command 与 Event 边界

### UI command

```text
ImportAudio(path)
SelectProject(project_id)
StartProcessing(project_id)
CancelProcessing(project_id)
MergeSpeaker(project_id, from, into)
BrowseModels(filter)
StartModelDownload(model_id)
PauseDownload(download_id)
ResumeDownload(download_id)
CancelDownload(download_id)
UpdateSettings(AppSettings)
ResetSettings
```

### 后台 event

```text
ProjectChanged(MeetingProject)
ProcessingEvent(JobEvent)
DownloadEvent(DownloadEvent)
SettingsChanged(AppSettings)
ServiceError(UiError)
```

GPUI action handler 只创建 command 或调用 controller；controller 负责调用 service、启动后台订阅，并将结果回送到 `WorkspaceUiState`。

## 关键交互流程

### 启动

```text
GPUI Application
  -> 打开 JsonProjectRepository
  -> 打开 JsonSettingsRepository
  -> 创建 ModelManager
  -> 创建 backend inventory
  -> 加载 WorkspaceUiState
  -> 打开 Meetings route
```

启动失败只阻断对应能力：项目仓库失败阻断 Meetings，catalog 失败阻断 Models，settings 失败使用默认内存设置并提示保存失败；不能因为模型缺失阻断窗口启动。

### 选择 backend 并处理

```text
Settings 表单
  -> 校验 backend/model 组合
  -> SettingsService::update
  -> WorkspaceUiState 更新
  -> StartProcessing 使用新的 ModelRef.backend_id
  -> application provider 选择具体 backend
```

处理时 UI 不重新创建 backend，也不直接读取模型文件。

### 下载模型

```text
Models route
  -> ModelManager::start(model_id)
  -> DownloadTask.events
  -> UI 更新 DownloadQueueState
  -> Pause/Resume/Cancel 调用 ModelManager
  -> Completed 后刷新 catalog availability
```

下载事件接收必须在后台线程或异步适配层中完成，禁止在 GPUI render 或 action handler 中阻塞等待 `std::sync::mpsc::Receiver`。

## 组件拆分计划

仅在用户确认本规划后实现，目标目录如下：

```text
crates/ui/src/
├── lib.rs                 GPUI application 启动和根 view
├── state.rs               WorkspaceUiState 与页面状态
├── route.rs               Route 和导航动作
├── controller.rs          command 到 service 的异步协调
├── events.rs              backend/runtime event 到 UI event 的适配
├── views/
│   ├── shell.rs           顶部导航、全局任务提示和页面容器
│   ├── meetings.rs        项目列表和空状态
│   ├── meeting_detail.rs  会议处理状态和 transcript
│   ├── models.rs          模型筛选、卡片和下载队列
│   └── settings.rs        设置表单和校验提示
└── components/
    ├── status_badge.rs
    ├── progress_row.rs
    ├── model_card.rs
    └── transcript_row.rs
```

拆分原则：

- `views` 只组合组件并读取 state。
- `components` 只处理展示和局部交互，不调用 service。
- `controller` 是唯一的 application/model/settings 调用入口。
- `events` 统一处理后台事件订阅和生命周期。
- 单个 Rust 文件保持在单一页面或单一职责范围内，不建立通用巨型组件。

## 实施顺序

### UI-1：状态和导航骨架

产物：`WorkspaceUiState`、`Route`、根级 controller、Meetings/Models/Settings 页面空壳。

验收：页面可以切换；不接入真实下载和推理；render 不包含文件 IO。

### UI-2：迁移现有 Meetings 流程

产物：项目导入、项目列表、处理进度、取消、transcript 展示和 speaker 合并。

验收：复用已有 application API；现有 M4A 处理链不改变；UI 事件不会阻塞 GPUI。

### UI-3：绑定 Settings

产物：ASR/Diarization backend、模型、设备、语言和 speaker 数量表单。

验收：设置可保存、重启后恢复；选择 backend 后下一次处理使用对应 `ModelRef.backend_id`；无效组合不能保存。

### UI-4：绑定 Models

产物：catalog 浏览、过滤、安装状态、下载队列、暂停、继续、取消和错误状态。

验收：同一模型不会重复下载；暂停不会丢失 `.part`；继续可收到进度；完成后模型状态变为 `Installed`。

### UI-5：交互和可用性收敛

产物：键盘导航、焦点状态、空状态、错误恢复、窗口尺寸适配和任务提示。

验收：所有异步操作都有 loading/success/error/cancelled 状态；窗口关闭或 route 切换不丢失后台任务引用。

## 明确不纳入本轮 UI

- 不引入 SQLite、localStorage 或 WebView。
- 不加入 LLM 去口语化界面。
- 不在 UI 中直接暴露模型 token 或 Python 环境。
- 不把实时音频波形、音频编辑器和复杂播放器作为 v0.2.1 前置条件。
- 不在未确认本规划前修改 `crates/ui` 实现。
