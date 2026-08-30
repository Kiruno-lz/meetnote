# meetnote 文档入口

## 架构

- [系统架构总览](architecture/overview.md)
- [ASR 与 diarization 后端管理层](architecture/backend-system.md)
- [模型目录与下载协议](architecture/model-management.md)
- [工程实施计划](architecture/implementation-plan.md)
- [架构决策](architecture/decisions.md)
- [GPUI UI 规划](architecture/ui-plan.md)
- [语音模型格式与 Rust 后端研究](reserch/voice-models-and-rust-runtimes.md)

## 当前阶段

旧版 Tauri、TypeScript、Python 和旧测试树已从工作目录清理。Git 提交 `0b031d0` 是清理前的文档归档检查点，之后的历史提交保留旧实现。当前已进入 v0.1 Rust-native foundation。

当前指导方向：MeetNote 是面向任意 Agent 的本地音频处理与 AI 可读文档后端，会议、一天的记录、访谈和其他内容都只是文档的可能语义。系统先把音频处理为完整、可追溯的原始转录和说话人对齐结果，再保存为可被 Agent 读取和编辑的文档资料。文档是否拆分、如何命名和如何总结，由 Assistant 或外部 Agent 使用 Skill 决定，并记录在目标目录下文档自身的 JSON/结构化状态中。MeetNote 实时扫描这些文档，主动解析、校验并维护固定最多三层的语义项目树；文档是事实来源，派生树可随时重建，不使用 SQLite 或独立项目数据库。Assistant 保留为与外部 Agent 同层的内置客户端，复用相同文档接口，不内置特殊语义服务。
