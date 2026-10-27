# GPUI UI 计划

## UI 边界

GPUI 是 MeetNote application/document service 的客户端，只负责交互与状态投影。它不执行音频 IO、模型下载协议、推理、聚类、语义拆分或 Agent 决策；渲染路径不做文件系统探测，安装状态、路径有效性等由服务在事件发生时刷新到 UI 状态。

```text
UI action
  -> controller
  -> application/document/model/settings service
  -> typed result or event
  -> UI state projection
```

文档目录是项目树事实来源。UI 的树、内容和状态在启动及文件变化后重新读取，不维护独立项目副本。

## 信息架构

```text
MeetNote  透明标题栏 + 玻璃侧栏 + 不透明内容平面
├── Assistant       内置 Agent 客户端
├── Documents       三层项目树（侧栏）、文档内容与处理
│   └── Detail      右侧不透明详情面板，不切换 route
├── Models          catalog、安装与下载任务
└── Settings        外观、backend、模型、设备、语言与路径
```

侧栏承载四个入口与导入音频；Documents 路由下侧栏内嵌项目树，内容平面只显示当前文档。导航文案为 `Meetings` 时，代码和内容模型仍使用中性的 document/audio project。

## 技术栈与视觉系统

### 依赖

| crate | 来源与锁定 | 说明 |
|---|---|---|
| `gpui`、`gpui_platform` | 声明为 `zeronsh/zui` rev `1561532176f54262e360acbb8bc00c6fcf183975`，`default-features = false`，features `font-kit`、`runtime_shaders`；根 `Cargo.toml` 的 `[patch."https://github.com/zeronsh/zui"]` 把该来源的全部 crate 指向 `Kiruno-lz/zui` 分支 `meetnote/1561532` | Apache-2.0；macOS 由 Metal + MetalPerformanceShaders 渲染，wgpu 只用于 Linux/Web |
| `gpui-base`、`gpui-component-assets` | `zeronsh/gpui-component` rev `5086ff4fb162ae97ca9d806876e7c6694ac3a81f` | Apache-2.0；Zeron 使用的同一 pin，已适配 zui API；`gpui-base` 提供无样式交互原语，assets 提供 Lucide 图标；不依赖 styled `gpui-component` crate |
| `gpui_tokio` | 不引入 | MeetNote 不使用 tokio；异步由 JobRuntime 与 gpui executor 承担 |

- zui 独有 API（`Window::paint_backdrop_blur`、`Window::with_edge_fade`、`paint_image_fitted_masked`）只允许在 `crates/ui/src/theme/` 的玻璃原语中调用；原语在不透明材质下绘制 token 填充。
- 不启用 `http_client/github-download`：它是 zui 中唯一把 GPL `path` 引入 gpui 依赖图的 feature。
- fork 的 gpui-component 以 longbridge `da4f9369` 为基线，缺少当前 pin `1081d2b` 所含的 78 个上游提交，包括：
  - #2810：masked Input 复制明文；
  - #2809：IME undo；
  - #2818：残留 `focused_input`；
  - #2824：单行 Input 绘制滚动条；
  - #2819：进度条边框；
  - `Select`/`Progress::accessibility_label`。
- 全局 git 配置把 `https://github.com/` 改写为 SSH 时，cargo 拉取 git 依赖需设置 `CARGO_NET_GIT_FETCH_WITH_CLI=true`。

### 维护与退出

- zui 与 fork 只按 Zeron `Cargo.toml` 中成对锁定的 rev 一起升级，每次升级都在 macOS 27 上运行完整门禁与离屏基线。
- `Kiruno-lz/zui` 分支 `meetnote/<zui rev>` 以该 zui rev 为基，只 cherry-pick 上游 zed 已合并的修复，不承载 MeetNote 自有改动。当前一项：zed `7bddd16a09`（#62775），测试窗口的 raw window/display handle 返回 `HandleError::NotSupported`，否则 MeetNote 窗口根（`crates/ui/src/controls/window.rs`）在 macOS 上调用 gpui-base 的 `install_window_hit_test_forwarder` 时使所有 `TestAppContext` 窗口 panic。
  - 升级时在新 zui rev 上重建分支，丢弃新 rev 已包含的 cherry-pick；分支不再携带提交时删除 `[patch]`，直接使用 `zeronsh/zui`。
  - `[patch]` 必须覆盖 `Cargo.lock` 中来自 `zeronsh/zui` 的每个 crate，否则 gpui-base 与 MeetNote 会解析出两份 gpui。验证：`grep 'zeronsh/zui' Cargo.lock` 无结果。
- //TODO - `scripts/package-macos-app.sh` 声明的 `LSMinimumSystemVersion` 13.0 尚未验证。验证：在 macOS 13 设备或虚拟机上启动应用并运行 L3；不通过则把声明版本提高到已验证版本。
- 退出条件，满足任一即评估回退上游 GPUI：
  - zui 或 fork 连续 90 天无提交，且存在影响 MeetNote 的未修复缺陷；
  - 上游 gpui 的安全修复 30 天内无法移植；
  - 新 macOS 版本破坏渲染且 zui 无修复。
- 回退成本：
  - 恢复上游 gpui 与 longbridge gpui-component 版本对，`controls/` 包装改用 longbridge 的 styled 组件；
  - 玻璃原语退化为 token 填充，失去 in-window blur 与 edge fade；
  - 窗口 blur 在 macOS 26+ 需要自行维护 `NSVisualEffectView` 材质补丁。

### 视觉原则

- 三层材质：
  - 窗口玻璃：`WindowBackgroundAppearance::Blurred` 叠加半透明 `window_glass` 底色，承载标题栏与侧栏；zui 已修正 macOS 26+ 下 `NSVisualEffectView` 材质失效。
  - 内容平面：不透明，承载文档阅读、transcript、Detail、表单和编辑。
  - 浮层：菜单、下拉、对话框与 Assistant 输入条使用场景 backdrop blur。阅读与编辑内容不放在玻璃上。
- 滚动列表、transcript 与标题栏下方使用 EdgeFade；超长标签用尾部渐隐代替截断省略号。浮层不加阴影，只有不透明回退时使用阴影。
- blur 强度为 `BlurLevel` 枚举，最多两个常量值，同屏 blur kernel 数量由类型保证。开合动画只改变透明度与位置，不改变 blur 半径与浮层尺寸，避免每帧新建 MPS kernel 与纹理。
- 设计 token 集中在 `crates/ui/src/theme/`：含 alpha 的颜色、hairline、wash、scrim、radius、spacing、控件高度、字号与 motion，深浅两套。
  - 窗口根按 token 绘制整窗底色，gpui-base 的全局滚动条样式随 token 更新。
  - 玻璃材质下 `tokens.background` 的 alpha 为 0。
  - 页面不写颜色字面量。
- 玻璃底色透明度由纯函数按最坏桌面背景计算，保证正文对比度 ≥ 4.5:1、次要文字 ≥ 3:1。
- 外观：
  - 跟随系统浅色/深色；Settings 提供"跟随系统/浅色/深色"与"毛玻璃/不透明"，保存在 AppSettings 的 `appearance` 字段（`#[serde(default)]`），不经 Settings capability 暴露。
  - 主题切换后重新应用窗口背景，因为 gpui 会移除原有 `NSVisualEffectView`。
  - motion token 遵循 `cx.reduce_motion()`。
  - 启动时读取 `NSWorkspace.accessibilityDisplayShouldReduceTransparency`，为真时强制不透明；`effective_material(preference, reduce_transparency)` 的 L1 测试覆盖四种组合。
- Zeron（MIT）的 theme、frost、edge_fade、motion 代码可以移植，需在文件头保留版权与许可声明，并登记到仓库根 `THIRD_PARTY_NOTICES.md`。zui 的 `NOTICE` 按 Apache-2.0 §4(d) 一并收录。不复制 Zeron 标识、第三方品牌标识和许可未知的音频。
- 字体默认 `.SystemUIFont`（SF/PingFang）；Geist（OFL）只作可选拉丁字体；引入前以 L3 截取选用 Geist 时的中英混排文本并通过视觉清单，确认中文回退到 PingFang，并在 `THIRD_PARTY_NOTICES.md` 附 OFL 文本。

### 组件归属

| 类别 | 实现 |
|---|---|
| 窗口外壳、标题栏拖拽区、侧栏、工具栏、列表行、状态 chip | MeetNote 自建，按 token 与 Zeron 布局配方实现 |
| 下拉菜单、对话框、Assistant 输入条 | gpui-base 无样式原语（`Select`、`Dialog`、`popover`、`focus_trap`、`positioner`）+ MeetNote frosted 渲染，保留键盘、焦点与 `accessibility_label` |
| 按钮、分隔线、进度、折叠、滚动条 | gpui-base 原语上的 MeetNote 包装（`controls/button.rs`、`separator.rs`、`progress.rs`、`collapsible.rs`、`scrollbar.rs`），按 token 渲染 |
| 单行输入 | gpui-base input 上的带 label 包装（`controls/text_field.rs`），支持粘贴、IME、undo/redo 与 secret 掩码；密钥框拦截 Copy/Cut：Copy 不写剪贴板，Cut 不改变值与选区 |

所有控件经 `crates/ui/src/controls/` 的包装构造，交互控件的构造函数要求 label。窗口第一层是 MeetNote 窗口根（`controls/window.rs`），负责整窗底色、字体、rem 尺寸、macOS 无障碍 hit-test 转发与 gpui-base focus trap 内的 Tab 循环。tooltip 当前没有使用场景，未提供包装。


### 验证分层

- **L1**：controller/state/theme 纯函数测试。覆盖：
  - 对比度与 `effective_material`；
  - i18n key 源码扫描；
  - 颜色字面量扫描：`crates/ui/src` 除 `theme/` 外不得出现 `rgb(`/`rgba(`/`hsla(`，测试用 `include_str!` 读取源码，初始红线为 61 处；
  - 渲染函数中无 `std::fs`/`Path::is_file` 的源码扫描；
  - 下拉、进度条、对话框与文本输入只经 `controls/` 包装构造，以及 radio 选择只由带 label 的 `RadioGroup` 创建的源码扫描。
- **L2**：`TestAppContext` + fake service 的 GPUI 交互测试。覆盖点击、键盘、焦点、异步完成与 `debug_bounds` 结构断言；涉及 `runtime::spawn_blocking` 真实线程的测试调用 `cx.executor().allow_parking()`。
- **L3**：`VisualTestAppContext::capture_screenshot`（`Window::render_to_image`，Metal 离屏）。
  - 以固定窗口尺寸 1180×760、固定 locale 与 fixture 工作区渲染每个 route，覆盖深/浅 × 玻璃/不透明。
  - `crates/ui/tests/visual.rs` 是 `harness = false` 目标，在 macOS 主线程运行（libtest 的测试线程不是主线程）。
    - 比较：`MEETNOTE_VISUAL=1 cargo test -p ui --test visual`；
    - 更新基线：另加 `MEETNOTE_VISUAL_UPDATE=1`；
    - 未设置 `MEETNOTE_VISUAL` 时跳过，默认 `cargo test -p ui` 只编译 harness。
  - 基线 PNG 存 `crates/ui/tests/visual/`。`baseline.json` 记录 scale factor、窗口尺寸、locale 与 GPUI、gpui-base revision；scale factor 与基线不一致时比较直接失败。
  - 单通道差 > 2 的像素占比超过 0.5% 视为变化，需人工确认后更新基线。
  - 除 route 外，基线还覆盖打开的 Settings 下拉与对话框（四种外观）。
  - in-window blur 可以确定性截图；窗口背后桌面 blur 由 `NSVisualEffectView` 在进程外合成，只做手工检查。
  - 资源探测：`MEETNOTE_VISUAL=1 MEETNOTE_RESOURCE_PROBE=1 cargo test --release -p ui --test visual`（debug 构建拒绝运行）。深色玻璃下对 Settings 下拉与对话框各预热 10 次、开合 50 次，门限为 `MTLDevice.currentAllocatedSize` 增长 ≤ 8 MiB、draw p95 ≤ 16.7 ms；render 耗时含 GPU 回读，只打印不设门限。M2 Pro 上一次实测：两者增长均为 0 MiB，draw p95 为 1.72 ms 与 1.70 ms。
- **无障碍**：
  - L2 测试窗口不启用 accessibility tree，因此交互控件一律经 MeetNote 包装创建，包装要求传入 label；由 L1 源码扫描保证页面不直接构造未包装控件。
  - 发布前在 macOS 27 用 Accessibility Inspector 手工检查各 route。
- **视觉清单**（L3 与手工检查共用）：
  - 关键元素可见；
  - 文案完整，无截断、无乱码；
  - 无元素重叠、无溢出窗口、无样式崩坏；
  - route 与窗口标题符合当前步骤；
  - 玻璃材质下可见模糊，文字对比度满足上述阈值。

## 当前状态

下表描述当前工作树 UI。

| 区域 | 已实现 | 当前缺口 |
|---|---|---|
| Application shell | 窗口标题 MeetNote、透明标题栏、交通灯与拖拽条，最小 900×600；玻璃侧栏含四个入口（高亮当前 route）、导入音频与 Meetings 下的项目树；不透明内容平面；`crates/ui/src/theme/` 的深/浅 × 玻璃/不透明 token，跟随系统外观与 reduce transparency/motion；窗口第一层为 MeetNote 窗口根；frosted 浮层与 edge fade 原语；全部控件为带 label 的 gpui-base 包装；每个可聚焦控件只在键盘聚焦时显示焦点环；Tab 顺序为侧栏四个入口、导入音频、页面，Tab 到达滚动区内的控件时自动滚入视野；侧栏是带 label 的 tab list，上下键切换；项目树是一个 Tab 停靠点，方向键选择、展开与进入父子，Option-↑/↓ 调整同级顺序、Option-→ 移入上一兄弟、Option-← 移到上一层末尾，经同一带 revision 的 `Reparent`/`Reorder` 写回；对话框关闭后焦点回到打开者，关闭 Detail 回到"打开详情"，切换 route 后停在对应侧栏入口；选择型按钮为 radio 角色，由带 label 的 `RadioGroup` 包裹 | Accessibility Inspector 手工检查 |
| Assistant | 内容平面底部的 frosted 输入条（Enter 发送）、场景/scope、连接、结果、warning/多 mutation、确认/undo 与进度渲染；任务状态由共享 JobRuntime 的 `JobEvent` 按请求投影；pending 过期后队列推进到下一项 | — |
| Workspace search | 项目侧栏按 Enter 搜索已保存的标题、标签、摘要、转写与 Markdown；`⌘⇧F` 展开侧栏并聚焦；显示命中片段、来源与转写时间，最多 50 个匹配文档；原文命中打开 Original，按 revision、segment ID、时间范围与 speaker 来源定位并标记段落，避开顶部渐隐区；定位一次后允许正常滚动，过期目标提示重新搜索；无处理记录或部分对齐时仍保留原始转写；Markdown 命中打开独立的最新保存笔记，未保存草稿保留，显式切回编辑时刷新干净草稿；保留搜索结果，清除后展开所选文档祖先；外部变更重新查询和读取笔记，编辑查询或切换工作区拒绝迟到结果；读取失败显示原因，笔记可重试 | 段内词级定位、音频 seek、日期/标签筛选与大规模索引，见[产品优先级](product-opportunities.md) |
| 笔记复制 | 笔记页复制当前已加载的保存快照，保留 Markdown、Unicode 与换行，成功后通知；保留未保存草稿和文件字节；加载/失败/空正文/文档不匹配/工作区切换不复制；中英文、最小窗口和 Tab/Enter 覆盖 | TXT/SRT 等文件导出及转写复制 |
| Documents | 目录数据源、三层树、展开、拖动重排、Summary/Original/编辑、处理与取消；内容平面顶部列出 scanner issue（路径与原因）；标题、标签、状态、摘要与正文按文档保存草稿，经一次带 revision 的 `Update` 提交；树移动经 `Reparent`/`Reorder` 命令；revision 冲突显示通知并保留草稿，用户重新加载后才丢弃；Original 视图的 speaker 列表支持改名与确认后合并；scanner issue 按 `DocumentIssueKind` 本地化；选中文档的音频在后台检查，缺失或 checksum 不符时在树、标题栏与 Detail 标记并禁止处理，经文件选择按 revision 重新关联；列表读取与处理启动在后台执行；整个处理 future 由 GPUI 后台执行器轮询，包含推理后来源校验和保存，等待恢复后也不回到界面线程；界面只接收进度与最终结果，启动完成前的取消在启动返回后生效 | — |
| Detail | 内容平面内右侧不透明面板：identity、audio source、层级、document status、revision、processing provenance、最近错误、结果统计与缺失的 provenance 项、transcript | 单文档的 Agent 变更来源（mutation journal 尚无按文档查询） |
| Models | catalog 分组、下载、暂停、继续、取消和进度，安装状态来自 `InstallationService`；卡片显示 license、来源与短 checksum，展开后显示完整 checksum、逐 backend 与逐文件 probe 结果；probe 结果、类型与 UTC 时间；本地化的失败原因与不可读记录；`ready` 且缺失/部分安装的模型可下载或重试，已安装模型可重新 probe | — |
| Settings | 外观（主题、材质）、文档工作区（后台切换并按 revision 保存；切换失败保持原目录，保存失败则切回；切换后重新订阅并清理旧目录的草稿、选择与 Assistant 待确认项）、locale、ASR/DIA backend/model、语言、设备、speaker 数、stage、Speaker Library 路径与管理（profile 列表、改名、确认后合并与删除；删除后焦点移到下一 profile 名称，没有下一项时移到声纹库标题，异步写入期间用户已移动的焦点保持不变；路径变化时重建服务并同步处理参数）；即时保存项经 `update_if_equal` 在后台逐项提交，Assistant 连接区为草稿并以 typed patch 保存（新建连接为一次 `patch_if_equal`）；revision 冲突后从 repository 重新加载再重试 | 能力联动说明和完整校验反馈 |

当前 GPUI 上的测试基线：

- L1：i18n key、颜色字面量、渲染路径文件探测，以及 `controller.rs` 中文件探测与线程创建的源码扫描；对比度、`effective_material` 与 token 纯函数；
- L2：路由结构与交互测试（`crates/ui/tests/routes.rs`、`crates/ui/src/lib.rs`、`crates/ui/src/document_tests.rs`、`crates/ui/src/speaker_tests.rs`），其中阻塞的 fake repository/model manager 断言文件 IO 不占用 UI 线程；树快捷键覆盖 revision、层级拒绝与焦点，Speaker Library 删除覆盖下一项、空库与异步焦点保持；真实 worker 等待使用 20 s deadline，挂起的 fake 调用必须显式放行，pending timer 用固定时间验证过期边界；
- L3：四个 route、键盘焦点态、打开的下拉与对话框、Detail 与 issue、编辑与冲突、speaker 列表与 Speaker Library、缺失音频与工作区设置、Assistant 部分结果与 review 队列在四种外观下的 60 张基线（`crates/ui/tests/visual/`，scale factor 2.0，zh-CN），以及 release 构建的资源探测。

处理执行位置与保存前取消检查的红绿回归、独立 review 见[处理执行器验证](../quality/ui-processing-executor-validation.json)。该验证覆盖执行上下文与写入边界，不代表实测帧率或音频质量。

UI 只允许一个前台音频处理任务；下载任务由 `ModelManager` 独立持续。音频处理与 Assistant 共用 `main.rs` 创建的应用级 JobRuntime，受同一并发上限约束；Assistant 任务状态由按请求标记的 `JobEvent` 投影，阶段与工具进度经 `AgentProgress` 回调补充细节。通用 session 不属于当前范围。

### 已知缺陷

新发现的缺陷按"复现 → 测试红线 → 修复 → 转绿"处理，测试写在 controller/state 层。

## Documents

左侧树最多三层：

```text
root document
└── semantic child
    └── semantic child
```

- 拖入节点内部：reparent。
- 拖到节点前后：同级 reorder。
- 非法来源、循环和第四层必须拒绝。
- watcher 发现无效外部文件时，在工作区显示路径、原因和刷新状态。
- 内容区默认显示 summary，可切换 speaker-labelled original。
- 编辑标题、标签、总结、正文或状态时携带当前 revision；冲突不得覆盖磁盘内容。
- processing/results 只读，语义字段可编辑。

Detail 作为右侧不透明面板，以 hairline 与内容区分隔；展示 identity、audio source、hierarchy、document status、revision、processing provenance 与结果统计；内容平面提供 Summary/Original/笔记/编辑模式，Markdown 在笔记中读取已保存正文、在编辑中维护草稿，Detail 不重复显示。关闭面板不得取消处理或下载任务。

## Assistant

本节承担 Assistant 页面与 Settings 页面更新，不属于 [Assistant 构建计划](assistant-plan.md)的能力接口/最小 Loop 验收。依赖 Agent 层的 typed result/event，不能让 UI 反向实现工具循环。

页面交互规则（由 `crates/ui/src/assistant_tests.rs` 的 L2 页面测试覆盖）：

- 一次运行的 typed result 到达前 Send 保持禁用，接受、拒绝或撤销进行中同样禁用；多写后失败或取消时，全部 receipt 按"已写入 N 项变更"分组，逐项可撤销，warning 全部保留；已被新运行或工作区切换取代的运行，其迟到的决定、撤销与 probe 结果不改变页面。
- pending 卡片显示"第 N / M 项"、UTC 过期时间与"运行在此写入前暂停"；决定进行中两个按钮禁用并改为进行中文案；接受或拒绝后焦点移到下一项，过期不抢占输入框焦点；溢出提示说明超出的 review 不会写入。Settings 说明自动写回与逐次确认的含义，未保存时给出提示。
- 连接 probe 显示运行中、已取消、结果或错误；`retryable` 错误提供重试，失败运行的 retryable 错误可重发请求；`SETTINGS_CREDENTIAL_TARGET_CONFLICT` 保留原错误码，在密钥框下提示需要重新输入密钥，否则说明留空会保留已存密钥。

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
- 修改标题、标签、总结、正文、状态或树关系时，automatic 模式直接执行并显示结果与撤销入口；confirm_each 模式在执行前显示明确 diff/proposal。
- Assistant 与外部 Agent 使用相同 document service，不允许直接编辑 processing/results。
- provider 不可用时保留文档管理能力，并显示可操作的配置错误。
- UI 投影 capability 结果与 JobRuntime 任务；外部 Agent 自管 Session/Context/Loop，不建设通用会话管理页。
- 输入框为内容平面底部的 frosted 输入条；结果、diff 与 pending 卡片位于不透明内容平面。
- Built-in connection profile、协议、模型和写回策略统一保存在 Settings；服务契约见 [Assistant 构建计划](assistant-plan.md)，页面与交互验收由本计划负责。

## Models

- 按 model family 展示任务、状态、大小、显存、license、来源和候选 backend。
- 只有 `ready` 且缺失/部分安装的模型提供下载按钮。
- 下载任务展示 artifact、字节进度、速度、ETA 和 queued/downloading/paused/verifying 状态。
- 完成后显示 checksum 与 backend probe 结果；失败时显示原因和重试入口。
- Settings 的模型选择器只允许选择已安装且与当前 backend 兼容的模型。

## Settings

设置项分为：

- Appearance：跟随系统/浅色/深色、毛玻璃/不透明。
- Workspace：文档目录、Speaker Library 路径、音频资产策略。
- Interface：UI locale。
- ASR：backend、model、transcription language。
- Diarization：backend、model、segmentation、embedding、clustering、expected speakers。
- Runtime：Auto/CPU/Metal/CoreML 与 backend 能力提示。
- Assistant：Skill/profile、能力授权与 Built-in connection、协议、模型、API key/API key file、loop 上限；Workspace 写回策略统一配置。

UI locale 与 transcription language 独立。backend 不支持的设备或 stage 不显示为有效选择。即时保存项在修改后给出成功或失败反馈；Assistant 连接区为草稿，点击保存才写入。

## 实施顺序

每阶段完成时运行 `cargo fmt --all -- --check`、`cargo check --workspace --all-targets` 和 `cargo test --workspace --no-fail-fast`。下列 14 个用例依赖尚未提交的实现，在对应实现提交前允许失败；出现任何其他失败即阻断当前阶段。

- Workspace Host 持久化（agent mutation journal）：
  - `agent_cli`：`external_agent_moves_reorders_corrects_revisions_and_rejects_invalid_trees`、`external_agent_confirm_each_has_accept_and_reject_lifecycle`、`external_agent_executes_synthetic_gold_including_nested_pass_and_undo`、`external_agent_updates_replays_reads_back_corrects_an_error_and_undoes`；
  - `agent_host`：`real_plugin_reaches_every_host_operation_shape`、`real_plugin_confirms_and_rejects_live_pending_mutations`、`real_plugin_exercises_confirm_each_through_host`、`control_operations_reject_unknown_arguments_before_consuming_state`、`response_driven_plugin_corrects_perturbed_fixture_and_reads_back`、`host_replays_the_same_mutation_receipt_idempotently`、`real_plugin_negative_calls_have_stable_errors_and_no_side_effects`。
- Settings 存储层（`apiKeyFile` 解析与安全打开）：
  - `host_probe`：`host_probe_reads_the_api_key_file_that_settings_validates`、`host_probe_rejects_an_unreadable_api_key_file_before_contacting_the_provider`；
  - `meetnote` 二进制单元测试：`assistant_service::tests::resolves_api_key_file_from_the_settings_directory`。

U0 基线、U1 缺陷修复与测试基线、U2 依赖迁移、U3 token 与外壳、U4 玻璃原语与页面迁移、U5 产品功能已完成，当前状态见上文。

- 发布前在 macOS 27 用 Accessibility Inspector 逐 route 检查 tab list、tree、combo box、radio、group、password input 的角色、label 与状态，以及焦点环与检查器焦点一致。

UI 完成标准：完整产品流程不需要 CLI 或手工编辑 JSON；外部文件变化、版本冲突、模型失败和任务取消都有明确且可恢复的反馈；每个 route 在四种外观组合下通过视觉清单。
