# AGENTS.md

## 项目定位

MeetNote 是 Rust-native、本地优先的音频处理与 AI 可读文档项目管理平台。它把音频转换为带时间戳和说话人信息的文档，以目标目录中的 Markdown 与文档 JSON 为事实来源，实时构建并维护最多三层的语义项目树。会议、全天录音和访谈只是文档内容类型，不是领域模型限制。

## 当前架构边界

- GPUI 只负责窗口、视图、交互和 UI 状态投影。
- `domain` 只包含领域模型、校验和纯业务算法，不依赖 GPUI、数据库或具体推理库。
- `application` 只编排用例，通过 trait 调用后端、持久化和模型服务。
- ASR 与 diarization 分别拥有独立的 backend manager，后端可替换、可组合。
- 推理后端不得启动 Python subprocess；项目不维护 Rust/Python 兼容层。
- 模型下载、校验、缓存和安装由 Rust model manager 统一负责。
- 文档 Markdown 与同目录 JSON 状态是项目结构的唯一事实来源；不使用 SQLite 或独立项目数据库。运行时索引和项目树必须能从目标目录重新扫描生成。
- MeetNote 主动监听或扫描目标目录，解析文档关系并实时维护最多三层的项目树；通过 MeetNote 写入时必须校验最大深度、循环和来源一致性，外部修改无效时必须显示错误而不是静默构造另一份结构。
- 复合录音的语义边界、标题、标签和总结由 Agent 按 Skill 决定并写入文档；MeetNote 负责读取、验证、保存和反映结果，不内置自动语义拆分器。
- Assistant 保留为内置 Agent 客户端，与任意外部 Agent 使用同一 application/document service、相同权限和相同写入规则，不得拥有直接修改 repository 或文件的特权路径。
- 所有后端输出先归一化到统一 contracts，再进入对齐、人工修正和文档生成流程；原始音频、原始转录和原始 diarization 结果不得被语义编辑覆盖。

## Workspace 约定

```text
apps/meetnote/
crates/domain/
crates/contracts/
crates/application/
crates/runtime/
crates/persistence/
crates/models/
crates/asr/
crates/asr-whisper-rs/
crates/asr-sherpa-onnx/
crates/diarization/
crates/diarization-sherpa-onnx/
crates/diarization-native/
crates/llm/
crates/ui/
```

`crates/` 下使用语义名称，不重复 `meetnote-` 前缀。Cargo package 名称与目录保持一致，只有需要区分具体实现时才使用后端后缀。

`crates/llm/` 当前只允许承载可选 Agent/provider 适配边界，不承载项目树、拆分、总结或索引的业务规则。语义规则属于 Skill，文档后端能力属于 application/persistence。

## 工程规则

- 先读当前状态和相关架构文档，再修改代码。
- 复杂逻辑必须拆分到职责明确的模块；GPUI View 不得承载领域算法或 IO。
- 新增 ASR/diarization backend 时，不得修改 application、domain 或 UI 的既有接口。
- 后端特有字段只能放入明确的 metadata/raw payload，不得污染统一领域模型。
- 所有长任务必须经过统一 JobRuntime，支持进度、取消、失败和恢复状态。
- 模型必须通过 catalog 管理，下载完成后进行尺寸、SHA-256 和 backend probe 校验。
- 文档写入必须采用原子替换并携带版本；文件变化后必须刷新项目树派生视图。
- 项目树固定最多三层。这个限制是正式产品需求，不得作为临时 MVP 约束移除。
- Assistant 和外部 Agent 的读写命令必须进入同一 controller/application 边界。
- 不提交模型权重、音频样本、secret、`.env`、构建产物或本机缓存。
- GPUI 版本必须锁定；新增 UI 依赖前先确认是否破坏跨平台和打包边界。

## 验证分层

- `domain`：纯 Rust 单元测试。
- `application`：使用 fake backend、fake repository 的用例测试。
- `persistence`：文档目录扫描、原子写入、外部修改检测、索引重建和冲突测试。
- 项目树：三层上限、循环、缺失父节点、跨音频来源和实时刷新测试。
- backend：每个实现独立的模型加载、输出归一化和错误测试。
- integration：真实模型与真实音频的受控测试。
- performance：冷启动、模型加载、实时率、内存峰值、WER/CER、DER。
- GPUI：视图和交互测试，不把 UI 测试当作推理正确性证明。

## 禁止事项

- 禁止恢复 Tauri、WebView 或 Python subprocess 作为默认架构。
- 禁止把多个 backend 的逻辑塞入单个 manager 或单个文件。
- 禁止把“任意 ONNX 文件”误认为可以无需模型族适配直接推理。
- 禁止在没有 schema、模型来源、license 和验证方式的情况下加入模型。
- 禁止使用 SQLite、全局 JSON project store 或其他独立数据库作为文档项目结构的事实来源。
- 禁止维护无法从目标目录文档重建的项目树状态。
- 禁止移除三层项目树限制。
- 禁止让 Assistant 绕过外部 Agent 使用的统一文档接口。
- 禁止把 Skill 的语义拆分规则硬编码进 MeetNote application/domain。
- 禁止在架构未确认前大规模实现 UI。
