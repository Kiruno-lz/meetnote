# meetnote

Rust-native meeting intelligence platform：本地导入音频，使用可切换的 Rust/native ASR 与 speaker diarization 后端，生成可编辑、可追溯的会议纪要。

## 当前状态

当前仓库已完成旧版 Tauri、TypeScript、Python 实现的清理，并完成 v0.1 Rust-native foundation：workspace、contracts、fake pipeline、backend managers、model catalog 和最小 GPUI 应用入口已经建立。新的工程边界、后端管理层、模型管理和实施顺序见 [`docs/index.md`](docs/index.md)。

## 目标架构

```text
GPUI UI
  -> Rust application layer
  -> ASR / diarization / LLM / model managers
  -> Rust or native-library inference backends
```

项目不再依赖 Tauri、WebView、Python subprocess 或 Python 运行环境。

## 文档

- [架构总览](docs/architecture/overview.md)
- [ASR 与 diarization 后端管理层](docs/architecture/backend-system.md)
- [模型目录与下载协议](docs/architecture/model-management.md)
- [工程实施计划](docs/architecture/implementation-plan.md)
- [架构决策](docs/architecture/decisions.md)

## 后端方向

- ASR：`whisper-rs`、`sherpa-onnx`，后续可加入 Candle 或其他 native backend。
- Diarization：`sherpa-onnx`、`diarization` crate，后续可加入自定义 ONNX/CoreML pipeline。
- 后端通过统一 Rust trait 注册和选择，应用层不依赖具体推理实现。

## 当前不执行

本阶段不下载模型、不接入真实推理依赖。完整 workspace 构建、测试和最小 GPUI 入口启动验证已通过；GPUI 使用 `runtime_shaders`，避免构建期依赖完整 Xcode 的 Metal shader compiler。
