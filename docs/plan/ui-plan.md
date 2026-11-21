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
├── Assistant       内置 Assistant 客户端与模型选择（⌘1）
├── Documents       三层项目树与搜索（侧栏）、文档内容与处理（⌘2，rail 文案为 Meetings）
│   └── Detail      内容区右侧详情面板，不切换 route
├── Models          catalog、安装、下载与 probe（⌘3）
├── Providers       LLM 供应商卡片、登录、额度与模型列表（⌘4）
└── Settings        外观、工作区、音频、Speaker Library、Assistant 行为（⌘5 / ⌘,）
```

Route 定义为 `apps/meetnote/src/main.rs` 的 `Route { Assistant, Documents, Models, Providers, Settings }`，默认 Documents。未打开工作区且非 `--sample` 时显示选择工作区目录的启动页。导航文案为 Meetings 时，代码和内容模型仍使用中性的 document/audio project。

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
| `providers.rs`（`sheet.rs`、`logos.rs`） | Providers 页、详情 sheet、供应商 logo 与 Assistant 模型选择器 |
| `workspace_host.rs`（子模块 assistant、audio、diarization_stages、models、preferences、processing、providers、speaker_library） | 后台任务提交与结果轮询，桥接 application 服务 |
| `desktop.rs` | 无窗口 CLI `process <audio>` 与其服务装配 |

每个页面有 sample 与 live 两套投影：`--sample` 及所有离屏场景使用内置示例数据，live 由打开的工作区驱动。

### 外观与无障碍

- 主题跟随系统/浅色/深色；"毛玻璃"开关使用 `SystemMaterial::Popover`，macOS Reduce Transparency 开启时强制不透明且不改写用户偏好。
- 侧栏开合动画遵循 reduce motion；收起的侧栏控件不进入焦点序列。
- 启用 Rune `accessibility` feature；rail、tab list、项目树与对话框设置 role 与 label，测试读取 `accessibility_json` 断言。
- 快捷键：⌘1–5 切换 route、⌘, 打开 Settings、⇧⌘F 聚焦搜索（Esc 返回原焦点）、⌘= / ⌘- / ⌘0 缩放、⌘[ / ⌘] 历史、⌘O 导入音频、⌘S 保存编辑；rail tooltip 显示对应快捷键。

## 当前状态

下表只列代码中已实现的行为；测试覆盖见下节，未列出的测试不代表已验证。

| 区域 | 已实现 | 当前缺口 |
|---|---|---|
| Shell | 透明标题栏与交通灯、rail 五入口与导入音频、可拖动/可收起侧栏、前进后退历史、缩放、Toast、Reduce Transparency/Motion | Accessibility Inspector 手工检查 |
| Documents | 三层项目树（拖动与 Option-方向键 reorder/reparent，经带 revision 的命令写回，拒绝跨音频与非法层级）；Summary/Original/Notes/Edit 四种模式；五种工作流状态；草稿按文档保留，revision 冲突保留草稿；外部 Markdown 变化不丢弃草稿；处理、重新处理与取消；缺失音频提示并经原生文件选择重新关联；scanner issue 列表；Detail 面板（转写、处理来源、统计）；说话人改名与确认后合并；复制转写 | 文件导出（TXT/SRT），见[产品优先级](product-opportunities.md) |
| Workspace search | 侧栏 Enter 搜索已保存的标题、标签与 Markdown 等内容，结果保留至清除；转写命中打开 Original 并定位、标记段落；未保存草稿不参与搜索 | 段内词级定位、音频 seek、筛选与大规模索引 |
| Assistant | ask/organize/partition/extract/move 五个场景；当前文档或全部可见文档 scope；发送与取消；pending review 的接受/拒绝与撤销；页头模型选择器（`供应商 · 模型`），未配置供应商时提示前往 Providers | 真实 provider 桌面验收 |
| Models | catalog 刷新、下载、暂停/继续/取消、重新 probe、probe 与 backend 检查、checksum 显示；所选模型引用写入设置；处理前要求模型已安装 | — |
| Providers | `apps/meetnote/src/providers.rs`：已连接供应商卡片（账号/套餐、额度窗口、重置卡、余额、模型数与“助手使用中”），未配置的推荐供应商列表、“更多提供商”按厂商类型分组与自定义端点；详情 sheet 为草稿，含登录/导入 Codex/退出、API key、模型模式与 chips、端点、高级参数，保存经 `ProviderService::save` 按 revision 提交；网络工作在 `workspace_host/providers.rs` 后台执行并可取消；sample fixture 与 `providers`/`provider-detail`/`provider-more` 场景共用同一渲染路径 | 真实供应商登录与额度接口未实测 |
| Settings | 外观（主题、毛玻璃、界面语言）、工作区、音频（转写语言、设备 Auto/CPU/Metal/Core ML、说话人数、diarization stage）、Speaker Library（目录切换、改名、合并、删除）、Assistant 行为（启用、写回策略、tool round 与校验重试上限）；保存时不覆盖其他字段 | 能力联动说明与完整校验反馈 |

UI 只允许一个前台音频处理任务；下载任务由 `ModelManager` 独立持续。音频处理与 Assistant 共用应用级 JobRuntime，受同一并发上限约束。

## 测试基线

- **Runtime 交互测试**：`apps/meetnote/src/main.rs` 的测试模块驱动 Rune `Runtime`（900×600、深色、scale 1），按元素 key 发送点击与按键，断言可见文本、元素 frame 与 `accessibility_json`。覆盖：
  - 最小窗口下的 route 与对话框交互、Settings 下拉与下载控件、历史、缩放、侧栏拖动/收起/动画与 reduce motion、tooltip 快捷键、Esc 焦点返回、Detail 关闭后的焦点；
  - 项目树拖动与 Option-方向键移动、搜索与转写定位、说话人改名与合并、草稿切换保留、Assistant review 接受与撤销、Reduce Transparency 回退；
  - `file_workspace_*` 系列在临时目录的真实工作区上验证导入、保存与 revision 冲突、外部 Markdown 变化、音频重新关联、偏好/语言/diarization stage/Speaker Library/Assistant 行为持久化、模型选择、经本地 HTTP 回环的 Assistant 读取与写回、供应商连接/模型刷新/每系最新/移除与登录取消。
- **纯逻辑测试**：`shell.rs`（缩放步进、图标几何）、`documents.rs`（匹配区间）、`toasts.rs`、`assistant_service.rs`、`desktop.rs`。
- **集成测试**：`apps/meetnote/tests/command_line.rs`（`process` 命令）。
- **显式 ignored**：下载并 probe 公开 Speakrs bundle 的测试；以 `MEETNOTE_RUNE_TEST_MODEL_ROOT` 与 `MEETNOTE_RUNE_TEST_AUDIO` 处理真实音频的测试；`tests/long_audio_real.rs`。未运行不代表通过。
- **离屏快照**：Rune 提供 `--scene <name> --snap <out.png>`（另有 `--list-scenes`、`--size WxH`、`--scale`、`--theme dark|light`、`--tree out.json`）。场景共 18 个：documents、details、original、edit、assistant、assistant-review、models、providers、providers-search、provider-detail、provider-account、provider-custom、provider-more、settings、settings-menu、toasts、toasts-expanded、import，均为 sample 数据。仓库不保存快照基线，快照用于人工按视觉清单核对，不是自动门禁。
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

本节承担 Assistant 页面与 Providers 页的交互，不属于 [Assistant 构建计划](assistant-plan.md)的能力接口/最小 Loop 验收。UI 依赖 Agent 层的 typed result/event，不能反向实现工具循环。

- 页头模型选择器列出已启用供应商的可见模型（`供应商 · 模型`），选择经 `ProviderService::set_model` 保存；未配置任何供应商时显示空状态并跳转 Providers；供应商被禁用或退出登录时选择器标明不可用。

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
- Assistant 与 UI 使用相同 document service，不允许直接编辑 processing/results。
- provider 不可用时保留文档管理能力，并显示可操作的配置错误。
- 供应商与模型选择保存在 Settings 的 `assistant.providers`/`assistant.model`，secret 只在 `credentials.json`；写回策略与 loop 上限在 Settings。服务契约见 [Assistant 构建计划](assistant-plan.md)。

## Models

- 按 catalog 展示任务、状态、大小、license、来源和候选 backend。
- 只有 `ready` 且缺失/部分安装的模型提供下载。
- 下载任务展示进度与暂停/继续/取消；完成后显示 checksum 与 backend probe 结果，失败时显示原因与重试。
- 处理前要求所选模型已安装且与 backend 兼容。

## Providers

供应商页只服务内置 Assistant，不为外部 Agent 暴露连接或授权。

```text
Providers
├── 已连接   两列卡片（900 px 窗口为一列）；卡片除开关外可点开详情
├── 添加供应商  标题右侧搜索按钮；推荐列表 → 更多提供商（模型厂商 / 聚合平台 / 本机）→ 添加自定义端点
└── 详情 sheet  账号 · API key · 模型 · 端点（区域 + 每种 API 一行）· 高级（每种 API 的 URL 覆盖）；移除 / 取消 / 保存
```

- 登录：OpenAI 走 ChatGPT OAuth（PKCE，固定回调 `localhost:1455`），也可导入本机 Codex 登录（只读 `auth.json`）；OpenRouter 走官方 PKCE 换取 API key。xAI 等无公开第三方 OAuth 的供应商只用 API key；Anthropic 订阅登录不支持。等待浏览器回调可复制链接、可取消，取消终止后台等待。
- 额度：ChatGPT 显示各窗口剩余比例与重置时间、可用重置卡数和订阅到期日；OpenRouter/DeepSeek/Moonshot 显示余额。额度只做展示，打开页面或 sheet 时刷新，手动刷新按钮重新获取。
- 模型：`供应商列表`（live ∪ 手动添加，上限 24）、`仅所选`（chip 可切换）、`每系最新`（每个模型系列取最新）。每系最新模式下刷新列表后，Assistant 当前模型自动升级到同系列最新版本并保存。
- 搜索：点击“添加供应商”右侧的搜索按钮展开输入框，按名称或 ID（不区分大小写）在全部未配置供应商中过滤为平铺列表，无匹配显示空状态；“添加自定义端点”始终保留；Esc 或关闭按钮清空并收起，焦点回到搜索按钮。
- 端点与协议：供应商提供多种 API 时，模型 chip 与 Assistant 选择器标注该模型将使用的 API（Chat / Responses / Messages），选择时把解析结果写入 `ModelChoice.protocol`；只提供一种 API 时不标注。自定义端点填写名称、鉴权头与 Chat / Responses / Messages 三个 URL（至少一个）。
- 编辑是草稿，保存时一次提交供应商设置与凭据变化；revision 冲突重新加载草稿并显示原因；移除需确认，同时删除凭据并清除指向它的模型选择。
- 本机分组含 Ollama、LM Studio、oMLX（`http://localhost:8000/v1`，Chat Completions、Responses 与 Anthropic Messages）与 magpie 网关（`http://127.0.0.1:3425/v1`，同样三种 API；本机请求不需要网关 key），均无 key 行。
- 品牌 logo 来自 lobehub icons 与 magpie 仓库（MIT）以及 oMLX 仓库（Apache-2.0），见 `THIRD_PARTY_NOTICES.md`。
- 验证：L1 覆盖窗口标签、剩余/重置文案（双语）、套餐名、各模式 chip 状态；L2（`apps/meetnote/src/main.rs` Runtime 测试）覆盖 rail 与 ⌘4 进入、API key 连接并对 loopback stub 刷新模型、每系最新过滤、settings/credentials 落盘分离、Assistant 选择模型后运行到达 stub、确认移除、登录开始与取消、供应商搜索（过滤/无匹配/Esc/关闭）、选择模型写入解析出的 API、自定义端点至少一个 URL、端点行随区域与覆盖变化；L3 为 `--scene providers|providers-search|provider-detail|provider-account|provider-custom|provider-more` 的 headless 截图检查。

## Settings

- Appearance：跟随系统/浅色/深色、毛玻璃开关、界面语言。
- Workspace：文档目录。
- Audio：转写语言、设备、预期说话人数、diarization stage。
- Speaker Library：目录与 profile 管理。
- Assistant：启用、写回策略（自动/逐次确认）、tool round 与校验重试上限；供应商、模型与凭据在 Providers 页。

UI locale 与 transcription language 独立。backend 不支持的设备或 stage 不显示为有效选择。

## 实施顺序

每次改动运行 `cargo fmt --all -- --check`、`cargo check --workspace --all-targets` 与 `cargo test --workspace`。新增交互先在 `main.rs` 的 Runtime 测试中写出红线，再实现；视觉改动附对应场景的 `--snap` 截图按视觉清单核对。

- //TODO - 发布前在 macOS 27 用 Accessibility Inspector 逐 route 检查 tab list、tree、combo box、switch、password input 的角色、label 与状态，以及焦点环与检查器焦点一致。
- //TODO - 为关键场景建立快照基线比对（场景 × 深/浅），确定差异阈值后纳入门禁。

UI 完成标准：完整产品流程不需要 CLI 或手工编辑 JSON；外部文件变化、版本冲突、模型失败和任务取消都有明确且可恢复的反馈；每个 route 在深/浅两种外观下通过视觉清单。
