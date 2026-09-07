# MeetNote 文档索引

## 当前架构

- [系统架构总览](architecture/overview.md)：产品边界、数据流、crate 分区和依赖方向。
- [模型管理](architecture/model-management.md)：catalog、下载、安装、校验和 probe 生命周期。
- [文档与音频项目模型](architecture/document-model.md)：文档包、音频处理结果、三层项目树和 Agent 协作。
- [模型后端系统](architecture/backend-system.md)：兼容层接口、具体 ASR/diarization backend 和长音频协议。
- [架构决策](architecture/decisions.md)：当前必须长期保持的设计约束。

## 实施计划

- [工程实施计划](plan/implementation-plan.md)：当前完成度、生产化缺口和验收顺序。
- [GPUI UI 计划](plan/ui-plan.md)：现有 UI、目标信息架构和后续工作。
- [Assistant 构建计划](plan/assistant-plan.md)：Agent 场景、Skill/Tool/validator 边界、LiteLLM 协议 adapter、写回策略和分阶段验收。

## 研究依据

- [Diarization 三阶段模型与 native backend](reserch/diarization-native-three-stage.md)
- [语音模型格式与 Rust 后端](reserch/voice-models-and-rust-runtimes.md)
- [长音频离线分窗与语义子项目](reserch/audio-streaming-and-semantic-splitting.md)

研究文档提供选型与验证依据；当前产品事实以 `architecture/`、`plan/` 和代码为准。
