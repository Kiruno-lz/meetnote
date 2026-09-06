# AGENTS.md

## 项目定位

MeetNote 是 Rust-native、本地优先的音频处理与 AI 可读文档工作区。它把任意音频资产转换为带时间戳、说话人和处理来源的文档，并从文档自身的 Markdown/JSON 实时派生最多三层的项目树。

MeetNote 负责音频事实、文档服务和结构校验；Assistant 或外部 Agent 按 Skill 决定语义拆分、标题、标签、总结与父子关系。会议、全天录音和访谈只是内容类型，不是领域模型。

## 当前架构

```text
GPUI / CLI / Agent adapter
          -> application
               ├── document service -> persistence -> Markdown/JSON
               ├── processing -> audio/runtime
               ├── ASR provider -> ASR backends
               ├── diarization provider -> diarization backends
               └── Speaker Library identity service
```

- `domain`：领域模型、校验和纯算法，不依赖 GPUI、文件系统或推理库。
- `contracts`：跨层请求、结果、能力、模型引用、文档命令和统一错误。
- `application`：用例编排，只依赖 trait，不依赖具体 backend 或 UI。
- `audio`：解码、16 kHz mono 重采样、chunk 和离线窗口。
- `runtime`：任务、进度、取消和阻塞 worker 并发。
- `persistence`：文档包、目录扫描、关系错误和文件监听。
- `settings` / `models`：设置、catalog、下载、安装和 probe。
- `asr` / `diarization`：各自独立的 registry、manager 和 provider。
- 具体 backend crate：模型族加载、推理、后处理和窗口 session。
- `llm`：可选 Agent/provider 边界，不保存语义规则。
- `ui`：GPUI 视图、交互和状态投影。

完整分区见[架构总览](docs/architecture/overview.md)。

## 核心数据约束

- 每个项目由 `document.md + state.json` 表达；文档目录是项目结构的唯一事实来源。
- 项目树固定最多三层，必须能从目录完整重建。
- MeetNote 写入必须校验 schema、revision、循环、层级和音频来源。
- 外部文件修改无效时必须报告可定位错误，不得静默构造另一份结构。
- 处理任务状态、`processing` 记录和 `document_status` 必须分离。
- 原始音频引用、ASR、diarization、对齐与 processing provenance 不得被语义编辑覆盖。
- Assistant 与外部 Agent 必须通过同一 document service，具有相同权限和写回规则。
- `DocumentCommandService` 只是内部 application API，不是外部 Agent 的固定 ABI。

## 音频与后端约束

- 短音频保留整文件 baseline；长音频使用有上下文重叠的离线分窗，不建设实时流式协议。
- `AudioChunkReader` 负责连续解码、downmix、重采样、sample offset、取消和错误定位。
- ASR 拼接属于具体 ASR backend；application 不实现模型族特有去重。
- diarization 窗口不得输出最终局部 speaker label；必须汇总兼容 embedding observation 后执行录音级聚类。
- Speaker Library 只在匿名聚类之后进行身份匹配，不替代 clustering。
- ASR 与 diarization manager 必须独立；新增 backend 不修改 domain、application 或 UI contract。
- backend 不下载模型，不启动 Python，不更新 GPUI 或文档树。
- 模型由 catalog 管理，安装必须经过大小、SHA-256、快速 probe、处理前 preflight 和推理时真实加载。

## 工程规则

- 修改前先读取当前代码、相关架构文档和工作树状态；代码是实现状态的唯一证据。
- 选择满足当前要求的最简单长期实现，不增加推测性抽象、配置或兼容层。
- 复杂逻辑拆到职责明确的模块；GPUI View 不承载领域算法、文件 IO、下载、推理或 Agent 调用。
- 优先复用项目已有依赖；增加依赖前检查现有 API、跨平台和打包影响。
- 所有长任务经过统一 JobRuntime，必须具备进度、取消、失败可见性和受控并发。
- 文档和 Speaker Library 使用临时文件加原子替换；多文件或多文档操作必须设计故障恢复，不能只更新内存。
- 不提交模型权重、音频样本、secret、`.env`、运行数据、构建产物或本机缓存。
- GPUI 与第三方推理依赖必须锁定；vendor 变更必须保持最小、可审计并有对应测试。
- 工作树存在用户改动时保留并隔离无关文件，不覆盖、不清理、不顺带提交。

## 文档规则

- 指导性文档只记录当前架构、当前进度和预期功能，不保留版本过渡叙述。
- 已实现、可编译、合成测试、真实模型执行、精度验收和生产就绪必须明确区分。
- 内容有机融入对应章节，避免重复；总览只做最小说明并引用具体文档。
- 代码变化影响公开边界、数据格式、验证方式或计划状态时，同步更新对应文档。
- 未经用户明确授意，不创建、删除或修改项目文档。
- 研究文档只用于复杂 debug 或会改变系统架构的验证，存放在 `docs/reserch/`。
- 研究文档使用“探索原因 / 探索目标 / 探索结果 / 复现说明 / 注意事项与补充”结构；临时代码在探索后删除，只在附录保留核心方法。

## 验证分层

- `domain`：纯 Rust 单元测试。
- `application`：fake backend/repository、处理编排、文档命令和身份匹配。
- `persistence`：扫描、文件写回、外部修改、关系错误和重建。
- `audio`：packet 连续性、重采样、窗口覆盖、取消和压缩格式等价。
- backend：模型布局、probe、归一化输出和 backend-specific stitcher。
- integration：真实模型与真实音频的受控测试。
- performance：冷启动、实时率、峰值内存、长音频资源和并发。
- quality：WER/CER、DER、speaker count、跨窗口身份一致性和误匹配率。
- GPUI：视图与交互；UI 测试不能证明推理正确。
- release：打包、签名、notarization、license 和全新环境安装。

基础检查：

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```

真实模型测试必须显式提供模型和音频路径；`#[ignore]` 测试未运行时不得声称真实推理已验收。

## 禁止事项

- 禁止恢复 Tauri、WebView、Python subprocess、SQLite 或独立全局 project store。
- 禁止把语义拆分、总结、标签或索引规则硬编码进 MeetNote domain/application。
- 禁止让 Assistant 绕过外部 Agent 使用的统一文档边界。
- 禁止把多个 backend 或多个模型族实现塞入单个 manager。
- 禁止把任意 ONNX 文件视为无需模型族 adapter 的可运行模型。
- 禁止在缺少 schema、来源、revision、license、checksum 和验证方式时把模型标为 `ready`。
- 禁止移除三层项目树限制或把它描述为临时约束。
- 禁止把窗口、VAD、局部 speaker label 或 embedding observation 当成语义项目事实。
- 禁止在未确认架构前进行大规模 UI 实现。
- 禁止用短样本、无 ground truth 的输出或进程退出码代替生产验收。

## 完整文档索引

- [README](README.md)：项目简介、快速构建、架构入口。
- [文档索引](docs/index.md)：面向阅读者的文档导航。
- [系统架构总览](docs/architecture/overview.md)：分层、crate 边界与依赖方向。
- [模型管理](docs/architecture/model-management.md)：模型资产、下载、安装与 probe。
- [文档与音频项目模型](docs/architecture/document-model.md)：文档包、数据流、三层树与 Agent 协作。
- [模型后端系统](docs/architecture/backend-system.md)：兼容层和具体 backend。
- [架构决策](docs/architecture/decisions.md)：不可偏离的长期约束。
- [工程实施计划](docs/plan/implementation-plan.md)：完成度、生产化缺口和验收顺序。
- [GPUI UI 计划](docs/plan/ui-plan.md)：界面边界、当前状态和后续步骤。
- [Diarization 三阶段研究](docs/reserch/diarization-native-three-stage.md)：native pipeline 选型依据。
- [语音模型与 Rust runtime 研究](docs/reserch/voice-models-and-rust-runtimes.md)：模型格式与 runtime 依据。
- [长音频离线分窗研究](docs/reserch/audio-streaming-and-semantic-splitting.md)：长音频、聚类与语义边界依据。
