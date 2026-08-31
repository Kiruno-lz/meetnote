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
└── ui/
```

当前工作区仍保留 `crates/llm/` 占位 crate；它只能作为 Assistant/外部 Agent provider 的可选适配边界，不得承载项目树、拆分、总结或索引规则。

### v0.1 必须具备

- GPUI 最小窗口和应用入口。
- `domain` 的音频资产、文档、转录段、diarization 段和基础对齐算法。
- `contracts` 的 ASR、diarization、模型和错误类型。
- `application` 的 ASR/Diarization trait 与 fake pipeline。
- 独立的 ASR backend registry/manager。
- 独立的 diarization backend registry/manager。
- JobRuntime 的 job id、取消状态、进度事件和测试。
- Rust 读取并校验 `resources/models/catalog.json`。
- `persistence` 的文档文件读写与可重建扫描索引；不实现独立项目数据库或 SQLite。
- `whisper-rs` 的真实 ASR adapter，支持 GGML/GGUF 文件、CPU/Metal 设备选择和时间戳结果。
- `sherpa-onnx` 的真实 diarization adapter，支持 ONNX bundle、CPU/CoreML provider 和 speaker clustering。
- 应用入口注册 `whisper-rs + sherpa-onnx`，GPUI 读取并展示两个 backend descriptor。

### v0.1 不实现

- 不在构建或测试阶段加载模型权重；模型加载发生在应用收到有效 `ModelRef` 推理任务之后。
- 不下载模型权重；推理依赖可以编译进 workspace，但测试不要求本地存在模型 artifact。
- 不实现 SQLite schema。
- 不实现完整文档工作区 UI。
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
- 文档包与 `state.json` schema
- 固定三层项目树派生规则
- Assistant/外部 Agent 统一文档命令

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
  -> 保存 canonical document files
```

验收：不依赖 Python、不依赖 Tauri、不经过跨进程 JSON；已使用本地 GGML 与 ONNX 权重完成真实 M4A 音频端到端测试。

## 阶段 4：替换后端验证

已加入：

- sherpa-onnx ASR
- `diarization-native`：Pyannote powerset segmentation、WeSpeaker embedding、
  AHC/PLDA/VBx clustering、embedding observation、原子 JSON speaker profile 和自动身份标注。

验收：同一个 application pipeline 可以在 `sherpa-onnx` 和 `diarization-native` 间切换；
backend registry、application pipeline 和 UI 选择接口不需要按模型族增加分支。native backend 已通过
fake 三阶段测试、持久化/identity 测试和本地真实模型 M4A 验收。模型与算法选择依据见
[Diarization 三阶段研究](../reserch/diarization-native-three-stage.md)。

## 产品核心：AI 可读文档后端

领域模型应从“会议专用项目”收敛为音频处理结果、文档文件和固定三层项目树：

```text
AudioAsset
├── RawTranscript
├── SpeakerDiarization
└── AlignedTranscript
    └── Document files
        ├── Markdown content
        ├── JSON state
        └── ProjectTreeProjection (最多三层，可重建)
```

文档不携带固定内容类型。它可以代表一天、一次会议、一次访谈或其他由 Assistant/外部 Agent 定义的知识单元。音频必须先完成完整转录和说话人对齐，之后 Agent 才能基于这些资料决定是否创建子文档、如何命名和如何总结。

MeetNote 提供文档管理原语：扫描/监听目标目录，读取文档内容和 JSON 状态，创建文档，编辑文档，更新处理状态，保存版本，以及按照文档自身字段提供列表和查询。MeetNote 主动解析父子关系并实时维护最多三层的项目树。文档 JSON 是事实来源；不维护独立的权威数据库，任何内存索引和项目树都必须能够从文档重新生成。

Skill 指导 Assistant 或外部 Agent 决定是否拆分、拆成什么文档、如何命名和如何建立关系，并把结果写入相关文档。MeetNote domain 负责把这些声明解析为项目树、校验三层上限/循环/来源一致性并实时刷新，但不实现自动语义拆分、自动总结或自动标签。

Agent 接入必须是模型、厂商和 Skill 无关的机器可读文档读写合约。GPUI Assistant、CLI、IPC 或其他外部 Agent 适配器都复用同一个 document service、相同权限和三层树规则，不允许绕过服务破坏原始音频、ASR、diarization 和对齐结果。

## v0.2：桌面工作区与文档管理

当前落地：

- GPUI 项目列表和项目详情窗口（当前仍是文档管理迁移前的 MVP UI）。
- 系统音频文件选择器，支持导入 WAV、MP3、M4A、FLAC 等 Symphonia 可解码格式。
- `WorkspaceService` 连接项目导入、模型 probe、后台 pipeline 和项目状态。
- 当前仍使用全局 JSON project repository 和 Markdown 输出；迁移为“每份文档自身 JSON 为事实来源、列表索引可扫描重建”尚未完成。
- 真实 ASR/diarization 结果在 GPUI 中按 speaker 展示。

仍需完成：

- JobRuntime 事件订阅、取消和 ASR/diarization 状态展示已完成；仍需补充更细的阶段百分比和恢复策略。
- 完整 diarization bundle 的多文件下载、archive member 提取和 readiness 检查已完成。
- speaker 合并和 Markdown 回写已完成；文档 JSON 状态作为事实来源、文档版本和 Agent 读写边界仍需实现。

验收目标：GPUI 可导入音频、创建文档、执行 pipeline、重启后从目标目录恢复三层项目树，并显示 speaker-labelled transcript。当前 `MeetingProject` 命名和全局 JSON repository 仍是 MVP 实现；三层项目树本身是正式产品特性，迁移时必须保留。

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

1. 冻结音频资产、处理结果、文档内容、文档 JSON 状态和三层树派生规则。
2. 构建实时三层项目树、文档内容查看和基于文档状态的管理界面。
3. 绑定文档版本、状态和 Assistant/外部 Agent 变更结果展示。
4. 将文档 detail 实现为悬浮面板，不进行页面跳转。
5. 绑定设置、UI locale 和 i18n 基础能力。
6. 绑定模型浏览、下载队列和 ASR/diarization 处理状态。

当前已实现 Assistant、Meetings、Models、Settings 四个工作区入口、三层项目树、Summary/Original 展示、悬浮详情、模型下载任务绑定和 backend 设置选择。Assistant 作为长期保留的内置 Agent 客户端，目前仍是占位；它与外部 Agent 共用同一 document service。GPUI View 不得直接执行领域算法、文件 IO、模型下载、语义分析或 Agent 调用。

仍需完成：

- 正式拆分音频资产、处理结果和中性的 `Document` 领域类型，移除 `MeetingProject` 对产品定义的约束。
- 以目标目录文档自身 JSON 状态替代独立项目数据库，支持监听/扫描恢复、内容版本、处理状态写回和项目树实时刷新。
- 保留三层项目树和 `create_semantic_child` 的结构能力，将语义边界判断交给 Skill/Agent，并补充外部无效文档关系的错误状态。
- 暴露无 UI 的文档读取、创建、编辑、状态更新和项目树查询 service，供 Assistant 与任意外部 Agent 使用。

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
