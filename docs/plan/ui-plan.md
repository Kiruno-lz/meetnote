# Rune UI 计划

## UI 边界

桌面 UI 是 MeetNote application/document service 的客户端，只负责交互与状态投影。它不执行音频 IO、模型下载协议、推理、聚类、语义拆分或 Agent 决策；页面代码不直接调用 `std::fs`，文件、模型、处理与 Assistant 操作经 `workspace_host` 提交到后台任务，结果在每帧 `poll` 时回到界面状态。

```text
UI action
  -> MeetNote state（apps/meetnote/src/main.rs）
  -> workspace_host：runtime::spawn_blocking / JobRuntime
  -> application/document/model/settings service
  -> typed result，poll 时投影到页面状态
```

文档目录是项目树事实来源。UI 的树、内容和状态在启动及文件变化后重新读取，不维护独立项目副本。

## 信息架构

```text
MeetNote  透明标题栏 + 导航 rail + 可收起侧栏 + 内容平面
├── Assistant       内置 Agent 客户端（⌘1）
├── Documents       三层项目树与搜索（侧栏）、文档内容与处理（⌘2，rail 文案为 Meetings）
│   └── Detail      内容区右侧详情面板，不切换 route
├── Models          catalog、安装、下载与 probe（⌘3）
└── Settings        外观、工作区、音频、Speaker Library、Assistant 连接（⌘4 / ⌘,）
```

Route 定义为 `apps/meetnote/src/main.rs` 的 `Route { Assistant, Documents, Models, Settings }`，默认 Documents。未打开工作区且非 `--sample` 时显示选择工作区目录的启动页。导航文案为 Meetings 时，代码和内容模型仍使用中性的 document/audio project。

## 技术栈

| 依赖 | 来源与锁定 | 说明 |
|---|---|---|
| `rune` | `Kiruno-lz/rune` git rev `80456b72fd5155291cc37c7f0a3ad7493cac779d`，`default-features = false`，features `accessibility`、`metal-backend`、`native-file-dialog`、`svg` | 窗口、布局、文本、输入、无障碍树与离屏渲染 |
| `rune-kit` | 同一 rev，feature `markdown` | 控件、Toast、Markdown 渲染 |
| `ui-text` | `crates/ui-text` | `zh-CN`/`en-US` 文案；缺失 key 返回空串 |
| `objc2-app-kit` | macOS only | 读取 `NSWorkspace` 的 Reduce Transparency |

- Rune 的修改在 Rune 仓库完成，MeetNote 只整体升级 rev，不 vendor、不 `[patch]`。
- 全局 git 配置把 `https://github.com/` 改写为 SSH 时，cargo 拉取 git 依赖需设置 `CARGO_NET_GIT_FETCH_WITH_CLI=true`。
- //TODO - `scripts/package-macos-app.sh` 声明的 `LSMinimumSystemVersion` 13.0 尚未验证。验证：在 macOS 13 设备或虚拟机上启动应用并跑完关键路径；不通过则提高声明版本。

### 代码归属

| 文件 | 职责 |
|---|---|
| `main.rs` | `AppSpec`（标题 MeetNote、默认 1180×760、最小 900×600、macOS 透明窗口与交通灯）、应用状态、确认对话框、`view` 分发、离屏场景与 Runtime 测试 |
| `shell.rs` | rail、可拖动侧栏（192–360）、`WindowTitlebar`（前进/后退/侧栏开关）、历史、缩放（0.75–2.0）与全局快捷键 |
| `controls.rs`、`toasts.rs` | 深/浅主题色与小控件；rune-kit `ToastStack`（右下角、4 s、悬停展开） |
| `documents.rs`（`details.rs`、`speakers.rs`） | Documents 页、项目树、搜索、Detail 与说话人修正 |
| `assistant.rs`（`live.rs`） | Assistant 页 |
| `settings.rs`（`live_*.rs`） | Settings 与 Models 页 |
| `workspace_host.rs`（子模块 assistant、assistant_connection、audio、diarization_stages、models、preferences、processing、speaker_library） | 后台任务提交与结果轮询，桥接 application 服务 |
| `desktop.rs` | 无窗口 CLI（`agent …`、`process <audio>`）与 Host 装配 |

每个页面有 sample 与 live 两套投影：`--sample` 及所有离屏场景使用内置示例数据，live 由打开的工作区驱动。

### 外观与无障碍

- 主题跟随系统/浅色/深色；"毛玻璃"开关使用 `SystemMaterial::Popover`，macOS Reduce Transparency 开启时强制不透明且不改写用户偏好。
- 侧栏开合动画遵循 reduce motion；收起的侧栏控件不进入焦点序列。
- 启用 Rune `accessibility` feature；rail、tab list、项目树与对话框设置 role 与 label，测试读取 `accessibility_json` 断言。
- 快捷键：⌘1–4 切换 route、⌘, 打开 Settings、⇧⌘F 聚焦搜索（Esc 返回原焦点）、⌘= / ⌘- / ⌘0 缩放、⌘[ / ⌘] 历史、⌘O 导入音频、⌘S 保存编辑；rail tooltip 显示对应快捷键。

## 当前状态

下表只列代码中已实现的行为；测试覆盖见下节，未列出的测试不代表已验证。

| 区域 | 已实现 | 当前缺口 |
|---|---|---|
| Shell | 透明标题栏与交通灯、rail 四入口与导入音频、可拖动/可收起侧栏、前进后退历史、缩放、Toast、Reduce Transparency/Motion | Accessibility Inspector 手工检查 |
| Documents | 三层项目树（拖动与 Option-方向键 reorder/reparent，经带 revision 的命令写回，拒绝跨音频与非法层级）；Summary/Original/Notes/Edit 四种模式；五种工作流状态；草稿按文档保留，revision 冲突保留草稿；外部 Markdown 变化不丢弃草稿；处理、重新处理与取消；缺失音频提示并经原生文件选择重新关联；scanner issue 列表；Detail 面板（转写、处理来源、统计）；说话人改名与确认后合并；复制转写 | 文件导出（TXT/SRT），见[产品优先级](product-opportunities.md) |
| Workspace search | 侧栏 Enter 搜索已保存的标题、标签与 Markdown 等内容，结果保留至清除；转写命中打开 Original 并定位、标记段落；未保存草稿不参与搜索 | 段内词级定位、音频 seek、筛选与大规模索引 |
| Assistant | ask/organize/partition/extract/move 五个场景；当前文档或全部可见文档 scope；发送与取消；pending review 的接受/拒绝与撤销；无连接时提示配置 | 真实 provider 桌面验收 |
| Models | catalog 刷新、下载、暂停/继续/取消、重新 probe、probe 与 backend 检查、checksum 显示；所选模型引用写入设置；处理前要求模型已安装 | — |
| Settings | 外观（主题、毛玻璃、界面语言）、工作区、音频（转写语言、设备 Auto/CPU/Metal/Core ML、说话人数、diarization stage）、Speaker Library（目录切换、改名、合并、删除）、Assistant 连接 profile（endpoint、模型、协议、认证、限额与连通性 probe）；保存时不覆盖其他字段 | 能力联动说明与完整校验反馈 |

UI 只允许一个前台音频处理任务；下载任务由 `ModelManager` 独立持续。音频处理与 Assistant 共用应用级 JobRuntime，受同一并发上限约束。

## 测试基线

- **Runtime 交互测试**：`apps/meetnote/src/main.rs` 的测试模块驱动 Rune `Runtime`（900×600、深色、scale 1），按元素 key 发送点击与按键，断言可见文本、元素 frame 与 `accessibility_json`。覆盖：
  - 最小窗口下的 route 与对话框交互、Settings 下拉与下载控件、历史、缩放、侧栏拖动/收起/动画与 reduce motion、tooltip 快捷键、Esc 焦点返回、Detail 关闭后的焦点；
  - 项目树拖动与 Option-方向键移动、搜索与转写定位、说话人改名与合并、草稿切换保留、Assistant review 接受与撤销、Reduce Transparency 回退；
  - `file_workspace_*` 系列在临时目录的真实工作区上验证导入、保存与 revision 冲突、外部 Markdown 变化、音频重新关联、偏好/语言/diarization stage/Speaker Library/Assistant 连接持久化、模型选择、经本地 HTTP 回环的 Assistant 读取与写回。
- **纯逻辑测试**：`shell.rs`（缩放步进、图标几何）、`documents.rs`（匹配区间）、`toasts.rs`、`assistant_service.rs`、`desktop.rs`。
- **集成测试**：`apps/meetnote/tests/` 的 `agent_cli`、`agent_host`、`command_line`、`host_probe`、`settings_host_contract`。
- **显式 ignored**：下载并 probe 公开 Speakrs bundle 的测试；以 `MEETNOTE_RUNE_TEST_MODEL_ROOT` 与 `MEETNOTE_RUNE_TEST_AUDIO` 处理真实音频的测试；`tests/long_audio_real.rs`。未运行不代表通过。
- **离屏快照**：Rune 提供 `--scene <name> --snap <out.png>`（另有 `--list-scenes`、`--size WxH`、`--scale`、`--theme dark|light`、`--tree out.json`）。场景共 12 个：documents、details、original、edit、assistant、assistant-review、models、settings、settings-menu、toasts、toasts-expanded、import，均为 sample 数据。仓库不保存快照基线，快照用于人工按视觉清单核对，不是自动门禁。
- 运行：`cargo test -p meetnote`；全量门禁见[实施顺序](#实施顺序)。

视觉清单（快照与手工检查共用）：关键元素可见；文案完整，无截断、无乱码；无元素重叠、无溢出窗口、无样式崩坏；route 与窗口标题符合当前步骤；毛玻璃下文字可读。

## Documents

左侧树最多三层：

```text
root document
└── semantic child
    └── semantic child
```

- 拖入节点内部：reparent；拖到节点前后：同级 reorder；松开前不改变父节点。
- 非法来源、跨音频移动、循环和第四层必须拒绝。
- watcher 发现无效外部文件时，在工作区显示路径和原因。
- 内容区默认显示 Summary，可切换 Original、Notes、Edit。
- 编辑标题、标签、总结、正文或状态时携带当前 revision；冲突不得覆盖磁盘内容。
- processing/results 只读，语义字段可编辑。
- 关闭 Detail 不得取消处理或下载任务。

## Assistant

本节承担 Assistant 页面与 Settings 中 Assistant 连接区的交互，不属于 [Assistant 构建计划](assistant-plan.md)的能力接口/最小 Loop 验收。UI 依赖 Agent 层的 typed result/event，不能反向实现工具循环。

```text
输入请求
  -> Built-in 最小 Tool Loop + profile/Skill
  -> 读取文档
  -> 查询结果，或 typed write tool
  -> settings write policy：自动执行或用户确认
  -> Workspace capability
  -> watcher 刷新
```

- 纯查询不创建独立项目状态。
- 写回在 confirm 模式下先显示 pending review，接受后执行并提供撤销；拒绝不写入。
- Assistant 与外部 Agent 使用相同 document service，不允许直接编辑 processing/results。
- provider 不可用时保留文档管理能力，并显示可操作的配置错误。
- 连接 profile、协议、模型和写回策略统一保存在 Settings。

## Models

- 按 catalog 展示任务、状态、大小、license、来源和候选 backend。
- 只有 `ready` 且缺失/部分安装的模型提供下载。
- 下载任务展示进度与暂停/继续/取消；完成后显示 checksum 与 backend probe 结果，失败时显示原因与重试。
- 处理前要求所选模型已安装且与 backend 兼容。

## Settings

- Appearance：跟随系统/浅色/深色、毛玻璃开关、界面语言。
- Workspace：文档目录。
- Audio：转写语言、设备、预期说话人数、diarization stage。
- Speaker Library：目录与 profile 管理。
- Assistant：连接 profile、协议、模型、认证、限额与 probe。

UI locale 与 transcription language 独立。backend 不支持的设备或 stage 不显示为有效选择。

## 实施顺序

每次改动运行 `cargo fmt --all -- --check`、`cargo check --workspace --all-targets` 与 `cargo test --workspace`。新增交互先在 `main.rs` 的 Runtime 测试中写出红线，再实现；视觉改动附对应场景的 `--snap` 截图按视觉清单核对。

- //TODO - 发布前在 macOS 27 用 Accessibility Inspector 逐 route 检查 tab list、tree、combo box、switch、password input 的角色、label 与状态，以及焦点环与检查器焦点一致。
- //TODO - 为关键场景建立快照基线比对（场景 × 深/浅），确定差异阈值后纳入门禁。

UI 完成标准：完整产品流程不需要 CLI 或手工编辑 JSON；外部文件变化、版本冲突、模型失败和任务取消都有明确且可恢复的反馈；每个 route 在深/浅两种外观下通过视觉清单。
