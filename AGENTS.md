# AGENTS.md

## 角色

本仓库的 Agent 是 meetnote 项目的工程化协作者，负责把会议音频转录、发言人区分、忠实去口语化整理和 Markdown 历史库沉淀为可维护的本地桌面应用。所有行动都应先读当前仓库状态，再基于事实规划；不得虚构已存在的脚本、依赖或目录。

## 当前仓库状态

- 仓库已从空仓库演进为包含完整代码的 MVP 实现（2026-06-06）。
- 前端：312 个 L1 测试通过，TypeScript 类型检查 0 error；包含 11 个 UI 组件、6 个类型 Schema、10 个工具模块。
- Rust 桥接：4 个模块（asr/diarization/download/utils）基于 Tauri 2.x。
- Python 后端：ASR 4 引擎 CLI、pyannote speaker diarization CLI、模型下载器。
- L2 契约测试（8 个文件）因路径解析问题当前全部失败（详见 `docs/project/gap-analysis.md#G-01`）。
- 完整的代码深度审查报告见 `docs/project/code-review.md`。
- L3 Playwright E2E 测试（4 个 spec）上次运行 2 个失败。
- 文档现状：所有文档已整理到 `docs/` 目录，详见 `docs/index.md`。根目录仅保留 `README.md`（轻量入口）和 `AGENTS.md`（本文）。
- 项目内 `skills/` 目录未发现；后续若新增 Skill，执行任务前必须重新扫描并记录 name/description。

## 项目目标

- ASR 默认质量优先，优先评估 Whisper large-v3；主要面向中文会议，并支持夹杂少量英文片段。
- TTS 不属于 MVP 范围；除非后续用户明确要求朗读或语音生成能力，否则不得纳入早期实现。
- 模型不进入 app 包，也不承诺开箱即用；安装 app 后应提供按需下载、配置、校验和切换模型的能力。
- speaker 纪要是核心功能，必须闭环覆盖发言人区分、时间段对齐、speaker 标签、人工修正和最终纪要生成。
- `pyannote/speaker-diarization-3.1` 可作为外部下载/配置能力集成，但 speaker 标注与纪要闭环不能后置到 MVP 之后。
- 去口语化优先由本地 LLM 或云端 LLM 完成；本地支持 Ollama，云端支持 custom provider，并用严格 schema 约束输出。目标是在去除口语化基础上忠实原文，不做总结或摘要。
- 录音文件采用项目化管理，每个录音项目持久化保存标题、音频路径/指纹、转录、speaker、人工修正和可后期编辑的会议内容。
- 支持导入 `.wav`、`.mp3`、`.m4a` 音频文件。
- Tauri 是桌面壳，前端可用 TypeScript 实现主要业务逻辑；Rust 仅在确有必要时承担本地能力、性能、权限或进程编排，不强制作为默认实现。
- 首个交付目标是开发者可运行的 MVP；dmg、签名、公证和 Homebrew cask 属于 release 阶段。

## 架构边界

- Tauri 负责桌面壳、文件权限与本地能力入口；TypeScript 优先承载 UI、状态管理、项目数据模型和可测试业务逻辑。
- Rust 是可选能力层，仅在 TS 无法可靠完成权限、性能或进程编排时引入；不得为了技术偏好强制增加 Rust 复杂度。
- ASR 与 pyannote 初期走 subprocess/CLI 边界，使用稳定 JSON schema、错误码、进度事件和模型元数据连接 app。
- 模型权重、pyannote 资产和 LLM 配置均按需下载或由用户配置，不提交进仓库、不打进 app 包。
- Markdown 历史库是核心数据资产，MVP 默认采用纯文件重建索引，不引入 SQLite/Tantivy 作为默认依赖。
- 原始转录、speaker 修正、去口语化文本和最终 Markdown 必须可追溯；不可逆处理不得覆盖原文。
- `script/dev.sh` 是开发环境入口，负责依赖检查、端口检测、服务启动和健康检查。
- `packaging/`、dmg、签名、公证和 cask 只在 release 阶段规划与实现；MVP 不以打包发布为验收标准。

## Sub Agent 优先流程

- 编码、文件操作、搜索、分析和 review 默认应交给 sub agent 执行。
- 分配前必须整理完整 prompt：目标、仓库状态、限制、验收标准、禁止事项。
- 检查 sub agent 输出；若失败，带上错误命令、错误内容和必要上下文重新分配修复。
- 代码和功能完成后，额外分配 review sub agent 审查实现、测试和文档。
- 如果当前运行环境没有可用 task/sub-agent 工具，应在结果中说明限制，并保持改动最小、可审查。

## 代码与文档风格

- TypeScript 使用 strict、明确命名和可测试的纯函数边界，优先实现可在 L1 测试中验证的业务逻辑。
- Rust 仅在实际引入时使用清晰模块边界，提交前应支持 `cargo fmt --all -- --check`。
- Python 仅用于必要的 diarization/模型工具链，依赖与环境管理使用 `uv`。
- 文档使用中文为主，命令和路径保持精确；计划文档必须给出产物、验收标准和验证命令。
- 不提交模型权重、音频样本隐私数据、构建产物、`node_modules/` 或 secret。

## 测试与验证分层

- L1 Function Test：TypeScript `bun test` 优先覆盖项目管理、schema、speaker 合并、LLM 输出校验和 Markdown 纯文件逻辑；若引入 Rust/Python，则补充对应单元测试。新增核心逻辑覆盖率目标不低于 90%。
- L2 API/Contract Test：验证 app 与 ASR CLI、diarization CLI、LLM provider、Markdown 历史库之间的 JSON/CLI 契约。
- L3 E2E Test：通过 Tauri 应用完成导入 `.wav`/`.mp3`/`.m4a`、转录、speaker 标注、人工修正、LLM 去口语化、保存 Markdown、历史库重建等关键流程。
- L3-R Regression：E2E 发现确定性缺陷时，先保存最小复现脚本和 `REPRO.md`，再回查 L1/L2 后修复。
- 交付摘要必须说明实际运行的测试、未运行的测试和原因；文档-only 修改不得声称已构建、已安装依赖、已下载模型或已运行 app。

## Git Submodule 管理原则

- 仅把稳定、必要、可追踪的上游项目加入 submodule；MVP 不默认添加 submodule。
- 加入 submodule 前先记录用途、版本/commit、许可证、更新策略和本地 patch 策略。
- ASR 与 pyannote 相关集成优先通过外部 CLI、Python 依赖锁定和接口适配管理；除非确有源码级定制需求，不默认加入 submodule。
- 更新 submodule 必须有单独提交或清晰变更说明，并同步验证 license、构建和接口影响。

## Packaging、Cask 与 dmg 约束

- `script/dev.sh` 应作为本地开发环境入口，负责依赖检查、端口检查、服务启动和健康检查。
- MVP 不要求 dmg、签名、公证或 Homebrew cask；首个验收目标是开发者可通过脚本运行 app。
- release 阶段再在 `packaging/` 维护 dmg 构建脚本、签名/公证流程说明、checksum 校验脚本和 cask 模板。
- brew cask 只管理用户安装分发，不承担开发依赖安装；开发依赖应在 README/脚本中单独说明。
- dmg 构建不得依赖本机未记录状态；签名证书、notary 密钥、token 等必须通过环境变量或安全存储注入。

## 禁止事项

- 禁止在未询问用户的情况下做重大架构变更、添加重型依赖或修改 CI/CD。
- 禁止下载模型、添加 submodule、安装依赖或运行构建，除非当前任务明确要求。
- 禁止绕过失败测试、提交 secret、回滚他人改动、修改 `node_modules/`。
- 禁止把口语化清洗、发言人合并等不可逆处理覆盖原始转录；必须保留可追溯原文。
- 禁止把摘要、行动项或概括性总结作为 MVP 去口语化输出；若需要，只能标为后续功能。
- 禁止将模型权重、pyannote 认证信息、云端 LLM token 或用户音频内容写入仓库。

## 交付输出要求

- 最终回复包含：角色、执行步骤、修改文件、关键内容、验证命令或未运行说明。
- 若涉及代码，说明 `[affects: ...]` 影响面、测试结果和性能风险。
- 若只更新文档，明确说明未构建、未安装依赖、未下载模型、未添加 submodule。
