# meetnote 文档入口

## 架构

- [系统架构总览](architecture/overview.md)
- [ASR 与 diarization 后端管理层](architecture/backend-system.md)
- [模型目录与下载协议](architecture/model-management.md)
- [工程实施计划](architecture/implementation-plan.md)
- [架构决策](architecture/decisions.md)
- [GPUI UI 规划](architecture/ui-plan.md)

## 当前阶段

旧版 Tauri、TypeScript、Python 和旧测试树已从工作目录清理。Git 提交 `0b031d0` 是清理前的文档归档检查点，之后的历史提交保留旧实现。当前已进入 v0.1 Rust-native foundation。

当前指导方向：meetnote 是音频转录与语音资料整理工具，会议只是一个场景。系统以音频资产为根，按语义拆分为最多三层的项目树；每个项目保存 UUID、时间范围、标题、分类标签、原始文和总结文，主项目通过内部链接索引子项目。GPUI Assistant、项目树、悬浮详情、Models 和 Settings 工作区已在 `codex/ui-plan` 分支实现；GPUI UI 规划见 [GPUI UI 规划](architecture/ui-plan.md)。SQLite 不纳入本项目架构；LLM 负责语义拆分建议、项目整理、总结和语音库索引。
