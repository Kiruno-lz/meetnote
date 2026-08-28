# Rust-native 工程实施计划

## v0.1：可启动的 Rust-native foundation

### 目标

建立一个可编译、可测试、可启动并已接入第一组真实推理后端的 Rust-native foundation，不下载模型权重，不实现完整音频资料工作区 UI。

### 交付目录

```text
Cargo.toml
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
├── llm/
└── ui/
```

### v0.1 必须具备

- GPUI 最小窗口和应用入口。
- `domain` 的会议项目、转录段、diarization 段和基础对齐算法。
- `contracts` 的 ASR、diarization、模型和错误类型。
- `application` 的 ASR/Diarization trait 与 fake pipeline。
- 独立的 ASR backend registry/manager。
- 独立的 diarization backend registry/manager。
- JobRuntime 的 job id、取消状态、进度事件和测试。
- Rust 读取并校验 `resources/models/catalog.json`。
- `persistence` 的 in-memory repository 与 JSON project store；不实现 SQLite。
- `whisper-rs` 的真实 ASR adapter，支持 GGML/GGUF 文件、CPU/Metal 设备选择和时间戳结果。
- `sherpa-onnx` 的真实 diarization adapter，支持 ONNX bundle、CPU/CoreML provider 和 speaker clustering。
- 应用入口注册 `whisper-rs + sherpa-onnx`，GPUI 读取并展示两个 backend descriptor。

### v0.1 不实现

- 不在构建或测试阶段加载模型权重；模型加载发生在应用收到有效 `ModelRef` 推理任务之后。
- 不下载模型权重；推理依赖可以编译进 workspace，但测试不要求本地存在模型 artifact。
- 不实现 SQLite schema。
- 不实现完整会议项目 UI。
- 不实现 LLM provider 的网络调用。

### v0.1 验收命令

```bash
cargo fmt --all -- --check
cargo test --workspace --exclude meetnote --exclude ui
cargo build -p meetnote
```

### v0.1 验收结果

当前结果：

- workspace 已从根目录解析。
- `cargo fmt --all -- --check` 已通过。
- `cargo test --workspace --exclude meetnote --exclude ui` 已通过，覆盖 fake ASR、fake diarization、对齐、JobRuntime 取消/进度事件、两个 backend registry、in-memory repository 和 catalog。
- GPUI 依赖已锁定并完成 Rust 依赖编译；通过 `runtime_shaders` feature 避免构建期调用 `xcrun metal`。
- `cargo build -p meetnote`、`cargo test --workspace` 和短暂 `cargo run -p meetnote` 启动验证均已通过。
- Xcode 26.6 已被 `xcode-select` 选中，GPUI 使用 `runtime_shaders` 完成 macOS 编译与启动验证。
- `whisper-rs` 通过 `whisper-rs-sys` 接入 whisper.cpp；`sherpa-onnx` 通过 native C API 接入 speaker diarization；两者均经过 Rust 类型检查。
- foundation checkpoint 当时只验证了 backend probe；随后已下载本地测试模型并完成真实 M4A 端到端推理，结果已由 v0.2 workspace 保存链路复用。
- 新增 backend 只需实现 trait 并注册，不修改 application pipeline。
- catalog 中的模型来源、格式和状态已能被 Rust 读取。

## 阶段 0：架构冻结

产物：

- Cargo workspace 结构
- domain 类型边界
- contracts 版本
- ASR/Diarization trait
- JobRuntime 事件模型
- 模型 catalog schema

验收：新增 backend 不需要修改 application 和 UI 接口。

## 阶段 1：纯 Rust 基础层

实现：

- domain
- contracts
- application interfaces
- runtime
- fake backend
- fake repository

验收：不用真实模型即可测试导入、转录、分离、对齐、修正和保存完整流程。

## 阶段 2：模型管理

实现：

- catalog 解析
- cache layout
- HTTP Range 下载
- pause/resume/cancel
- SHA-256 校验
- atomic install
- backend probe

验收：应用重启后能恢复下载状态；错误模型不会进入 installed。

## 阶段 3：第一个真实纵向流程

已落地组合：

```text
whisper-rs ASR
+
sherpa-onnx diarization
```

流程：

```text
导入音频
  -> ASR
  -> diarization
  -> 对齐
  -> 保存 canonical project
```

验收：不依赖 Python、不依赖 Tauri、不经过跨进程 JSON；已使用本地 GGML 与 ONNX 权重完成真实 M4A 音频端到端测试。

## 阶段 4：替换后端验证

加入：

- sherpa-onnx ASR
- `diarization` crate backend

验收：同一个 application pipeline 可以切换 backend；UI 不修改。

## 产品领域演进：音频项目与语义项目树

下一阶段的领域模型必须从 `MeetingProject` 演进为音频资产和语义项目节点：

```text
AudioAsset
└── ProjectNode (最多三层)
    ├── metadata: UUID / parent / time range / title / tags
    ├── original content
    ├── summary content
    └── child nodes
```

处理顺序固定为完整转录、语义分析、再决定是否整理和拆分。长音频允许按语义场景拆分；短音频或单场景音频保持单项目。主项目承担对子项目的整理、总结和索引，只通过内部链接引用子项目，不复制子项目全文。所有拆分、重排、总结和标签变更都必须是可追溯的 application command。

LLM service 负责语义拆分建议、项目整理、总结和语音库索引；原始转录是不可覆盖的输入，LLM 输出必须经过 schema 校验，涉及项目树或内容写入时必须经过用户确认。

## v0.2：桌面工作区

当前落地：

- GPUI 项目列表和项目详情窗口。
- 系统音频文件选择器，支持导入 WAV、MP3、M4A、FLAC 等 Symphonia 可解码格式。
- `WorkspaceService` 连接项目导入、模型 probe、后台 pipeline 和项目状态。
- JSON 项目仓库和 Markdown 产物保存。
- 真实 ASR/diarization 结果在 GPUI 中按 speaker 展示。

仍需完成：

- JobRuntime 事件订阅、取消和 ASR/diarization 状态展示已完成；仍需补充更细的阶段百分比和恢复策略。
- 完整 diarization bundle 的多文件下载、archive member 提取和 readiness 检查已完成。
- speaker 合并和 Markdown 回写已完成；自由命名编辑与 LLM 忠实去口语化属于后续 hardening。

验收：GPUI 可导入音频、创建项目、执行 pipeline、重启后恢复项目，并显示 speaker-labelled transcript。现有 `MeetingProject` 是 MVP 结构，后续按“产品领域演进”升级为音频项目树。

## v0.2.1：后端能力层

UI 暂不绑定本阶段新增接口；所有能力先在 Rust crate 中可测试、可独立调用。

已完成：

- `settings` crate 提供版本化 `AppSettings`、JSON 原子保存、读取、更新和重置；不使用 SQLite。
- `AsrBackendProvider` 与 `DiarizationBackendProvider` 让 application 根据 `ModelRef.backend_id` 运行时选择后端。
- `models::ModelManager` 提供 catalog 按任务/backend 浏览、缺失/部分/已安装状态查询和安装目录规划。
- 下载任务提供进度事件、暂停、继续、取消；取消和中断保留 `.part`，继续使用 HTTP Range，校验失败清理临时文件。
- gated 模型在缺少认证接入时拒绝开始下载，不把 token 写入 catalog、日志或命令行。

后端验收：

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```

GPUI 页面后续只需要订阅 `DownloadTask.events`，调用 `ModelManager::pause/resume/cancel`，
并将 `AppSettings` 绑定到设置表单；不需要修改下载器或 application pipeline。

## 阶段 5：GPUI UI 实施状态与后续顺序

完整布局、状态模型、Command/Event 边界和验收标准以 [GPUI UI 规划](ui-plan.md) 为准，实施顺序固定为：

1. 冻结音频项目与语义项目节点的数据契约。
2. 构建无历史的 Assistant 单次输入首页和全局导航。
3. 构建最多三层的项目树、Summary/Original 切换和拖动重排。
4. 将 Meeting detail 实现为悬浮面板，不进行页面跳转。
5. 绑定设置、UI locale 和 i18n 基础能力。
6. 绑定模型浏览、下载队列和 LLM 整理/索引状态。

当前已实现 Assistant、Meetings、Models、Settings 四个工作区入口、项目树、Summary/Original 展示、悬浮详情、模型下载任务绑定和 backend 设置选择。GPUI View 不得直接执行领域算法、文件 IO、模型下载或 LLM 调用。

仍需完成：

- 正式拆分中性的 `AudioProject`/`ProjectNode` 领域类型和项目内容版本。
- 将 LLM 语义拆分建议、项目总结和语音库索引接入 controller。
- 补齐拖动同级前后排序、完整 i18n 资源和可编辑文本输入。

## 阶段 6：生产化

- 崩溃恢复
- 模型安装恢复
- Metal/CoreML 性能评测
- WER/CER/DER 基准
- macOS 打包、签名和第三方 license

## 完成标准

- `cargo test --workspace` 通过。
- fake backend 能覆盖 application 主流程。
- 每个真实 backend 有独立 integration test。
- ASR 和 diarization 可独立选择和组合。
- 无 Python、Tauri、WebView 依赖。
- 模型来源、checksum、license 和运行版本可追溯。
