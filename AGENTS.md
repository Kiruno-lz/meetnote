# AGENTS.md

## 定位与入口

MeetNote 是 Rust-native、本地优先的音频与 AI 可读文档工作区。基础框架由 Workspace Host、Audio Host、Agent Host 和 GPUI UI 组成。MeetNote 维护音频事实、文件与结构完整性；Agent 按 profile/Skill 决定语义。项目树固定最多三层。

- [架构总览](docs/architecture/overview.md)：四个基础层与当前 crate 的对应关系。
- [Workspace Host](docs/architecture/workspace-host.md) / [文档状态](docs/architecture/document-state.md)：事实来源、操作边界、外部编辑与恢复。
- [Agent Host](docs/architecture/agent-host.md)：会话运行时、backend、工具与场景的分工。
- [Audio Host 与 provider](docs/architecture/audio-host.md) / [模型管理](docs/architecture/model-management.md)：原生处理与模型生命周期。
- [Extension](docs/architecture/extension-system.md)：服务注册、依赖、生命周期与执行载体。
- [架构决策](docs/architecture/decisions.md)：长期约束与设计取舍。
- [架构对齐计划](docs/plan/architecture-alignment.md)：当前架构进度、职责收敛和最小实施步骤。
- [工程计划](docs/plan/implementation-plan.md) / [Assistant 计划](docs/plan/assistant-plan.md) / [UI 计划](docs/plan/ui-plan.md)。其他材料见[文档索引](docs/index.md)。

## 稳定工程约束

- 修改前读取代码、相关文档与工作树状态；代码证明实现状态，最新确认的方向指导目标设计。四个基础层是架构归属，不要求一对一新增 crate。
- 选择满足当前需求的最简单长期实现，复用已有依赖。
- `domain` 保持纯模型与算法；`application` 编排四个基础层的用例；GPUI UI 不执行文件 IO、下载、推理和语义判断。桌面使用 Rust/GPUI，不恢复 Tauri/WebView/Flutter。
- `document.md + state.json` 和普通资产文件是事实来源；树与索引可重建，不引入权威 SQLite/global project store。
- 源文件音频、Markdown、项目 JSON 和 Speaker Library 是 Workspace 的不可删除资产；全文索引、embedding index、解析状态和搜索缓存是可重建派生状态。
- MeetNote 管理的写入统一校验 schema、revision、来源、范围、循环和三层上限；外部编辑是正常流程，无效文件必须报告路径和原因。
- 插件只能通过 Host 提供的版本化 capability port 访问 Workspace 能力；port 不等于任意文件系统权限，也不能删除或静默覆盖不可删除资产。
- 内置 Assistant 与外部 Agent 共用 capability port 执行具体操作。
- Audio 保留独立 ASR/diarization provider、manager 和具体 backend；新增兼容实现不改领域与 UI contract。新能力才扩展 contract。
- 短音频保留整文件 baseline；长音频使用重叠离线窗口。ASR 拼接归 backend；diarization 汇总 observation 后录音级聚类；Speaker Library 在匿名聚类后匹配身份。
- 模型下载与安装归 models；backend 不下载模型、不更新 UI 或项目树。外部进程/云 provider 可按明确需求接入，必须管理取消、退出、资源和错误；不改变现有原生链路。
- 长任务统一纳入 JobRuntime 的进度、取消、失败与受控并发；Agent 会话状态不能另造 worker 调度器。
- 写入使用原子替换；多文件/多文档提交须有恢复协议，并保留普通附件。事务、undo、会话历史不能误归类为可随意删除的缓存。
- 锁定 GPUI 和推理依赖；vendor 改动最小可审计。模型需有 schema、来源、revision、license、checksum、probe 和真实加载验证。
- 保留无关用户改动，不提交模型权重、样本、secret、运行数据或构建产物。

## 文档与验证

未经用户明确授意，不创建、删除或修改项目文档。获授权后，将变化融入对应章节，总览保持简短；实现、测试存在、测试通过、真实模型执行、质量与生产验收必须分开陈述。

研究仅用于复杂 debug 或会影响架构的验证，放在 `docs/reserch/`，按“探索原因 / 探索目标 / 探索结果 / 复现说明 / 注意事项与补充”组织；清理临时代码并保留核心方法。

验证流程保证：提出/发现bug -> 复现场景，测试红线 -> 进行修复 -> 测试转绿

代码改动按影响范围运行验证，基础检查为 `cargo fmt --all -- --check`、`cargo check --workspace`、`cargo test --workspace`。纯文档改动检查链接、事实与 diff。真实模型测试必须显式提供模型和音频路径；未运行的 ignored 测试、短样本或退出码不证明精度、长音频资源或发布质量。
