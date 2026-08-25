# meetnote 文档入口

## 架构

- [系统架构总览](architecture/overview.md)
- [ASR 与 diarization 后端管理层](architecture/backend-system.md)
- [模型目录与下载协议](architecture/model-management.md)
- [工程实施计划](architecture/implementation-plan.md)
- [架构决策](architecture/decisions.md)

## 当前阶段

旧版 Tauri、TypeScript、Python 和旧测试树已从工作目录清理。Git 提交 `0b031d0` 是清理前的文档归档检查点，之后的历史提交保留旧实现。当前已进入 v0.1 Rust-native foundation。

v0.1 已建立 Cargo workspace、domain、contracts、application、runtime、model catalog、fake pipeline、backend managers、真实 `whisper-rs` ASR、真实 `sherpa-onnx` diarization 和最小 GPUI 入口。模型下载、真实音频端到端验证与 SQLite 按实施计划后续接入。
