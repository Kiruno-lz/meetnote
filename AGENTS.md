# AGENTS.md

## 项目定位

meetnote 是 Rust-native meeting intelligence platform。桌面 UI 使用 GPUI，业务编排使用 Rust application layer，ASR 和 speaker diarization 通过可注册的 Rust/native backend 提供能力。

## 当前架构边界

- GPUI 只负责窗口、视图、交互和 UI 状态投影。
- `domain` 只包含领域模型、校验和纯业务算法，不依赖 GPUI、数据库或具体推理库。
- `application` 只编排用例，通过 trait 调用后端、持久化和模型服务。
- ASR 与 diarization 分别拥有独立的 backend manager，后端可替换、可组合。
- 推理后端不得启动 Python subprocess；项目不维护 Rust/Python 兼容层。
- 模型下载、校验、缓存和安装由 Rust model manager 统一负责。
- SQLite 保存可编辑项目状态；音频、模型和 Markdown 是文件系统资产。
- 所有后端输出先归一化到统一 contracts，再进入对齐、人工修正和纪要生成流程。

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

## 工程规则

- 先读当前状态和相关架构文档，再修改代码。
- 复杂逻辑必须拆分到职责明确的模块；GPUI View 不得承载领域算法或 IO。
- 新增 ASR/diarization backend 时，不得修改 application、domain 或 UI 的既有接口。
- 后端特有字段只能放入明确的 metadata/raw payload，不得污染统一领域模型。
- 所有长任务必须经过统一 JobRuntime，支持进度、取消、失败和恢复状态。
- 模型必须通过 catalog 管理，下载完成后进行尺寸、SHA-256 和 backend probe 校验。
- 不提交模型权重、音频样本、secret、`.env`、构建产物或本机缓存。
- GPUI 版本必须锁定；新增 UI 依赖前先确认是否破坏跨平台和打包边界。

## 验证分层

- `domain`：纯 Rust 单元测试。
- `application`：使用 fake backend、fake repository 的用例测试。
- backend：每个实现独立的模型加载、输出归一化和错误测试。
- integration：真实模型与真实音频的受控测试。
- performance：冷启动、模型加载、实时率、内存峰值、WER/CER、DER。
- GPUI：视图和交互测试，不把 UI 测试当作推理正确性证明。

## 禁止事项

- 禁止恢复 Tauri、WebView 或 Python subprocess 作为默认架构。
- 禁止把多个 backend 的逻辑塞入单个 manager 或单个文件。
- 禁止把“任意 ONNX 文件”误认为可以无需模型族适配直接推理。
- 禁止在没有 schema、模型来源、license 和验证方式的情况下加入模型。
- 禁止在架构未确认前大规模实现 UI。
