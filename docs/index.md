# MeetNote 文档索引

## 当前架构

- [系统架构总览](architecture/overview.md)：Workspace Host、Audio Host、Agent Host、GPUI UI，以及现有 crate 的归属和依赖方向。
- [Workspace Host](architecture/workspace-host.md)：文件事实、窄操作、外部编辑与恢复。
- [Agent Host](architecture/agent-host.md)：Capability API、最小 Tool Loop 与场景分工。
- [Extension](architecture/extension-system.md)：服务注册、依赖、生命周期与执行载体。
- [模型管理](architecture/model-management.md)：catalog、下载、安装、校验和 probe 生命周期。
- [文档状态与音频项目模型](architecture/document-state.md)：文档包、音频处理结果、三层项目树和 Agent 协作。
- [Audio Host 与模型后端](architecture/audio-host.md)：兼容层接口、具体 ASR/diarization backend 和长音频协议。
- [架构决策](architecture/decisions.md)：当前必须长期保持的设计约束。

## 实施计划

- [架构对齐计划](plan/architecture-alignment.md)：当前架构进度、职责收敛、保留项和最小实施步骤。
- [工程实施计划](plan/implementation-plan.md)：当前完成度、生产化缺口和验收顺序。
- [GPUI UI 计划](plan/ui-plan.md)：现有 UI、目标信息架构和后续工作。
- [Assistant 构建计划](plan/assistant-plan.md)：Workspace、Audio、Settings 的 capability、Host 真实回应、MoonBit 插件与 Codex 经 Host 完成任务。

## 研究依据

- [Diarization 三阶段模型与 native backend](reserch/diarization-native-three-stage.md)
- [语音模型格式与 Rust 后端](reserch/voice-models-and-rust-runtimes.md)
- [长音频离线分窗与语义子项目](reserch/audio-streaming-and-semantic-splitting.md)

研究文档提供选型与验证依据；当前产品事实以 `architecture/`、`plan/` 和代码为准。
